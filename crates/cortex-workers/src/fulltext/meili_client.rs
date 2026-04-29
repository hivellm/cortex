//! Meilisearch HTTP client used by the indexer worker.
//!
//! Spec 08 §Meilisearch client describes the surface: `ensure_index`
//! creates the index plus applies the version-stamped settings,
//! `upsert_documents` POSTs a batch, `wait_task` polls task status for
//! the bootstrap fail-fast path.
//!
//! The client owns no business logic — it's the thin transport layer.
//! Retries with exponential backoff happen here so callers (worker,
//! bootstrap CLI) get a uniform "transient vs permanent" view of
//! Meilisearch errors.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::config::FulltextConfig;
use super::document::Document;

/// Meilisearch task identifier returned by document / settings calls.
pub type TaskUid = u64;

/// Status of a Meili task as reported by `GET /tasks/{uid}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task created but not yet picked up.
    Enqueued,
    /// Task currently processing.
    Processing,
    /// Task completed successfully.
    Succeeded,
    /// Task ran but returned an error from Meili.
    Failed,
    /// Task was canceled before completion.
    Canceled,
}

impl TaskStatus {
    /// Whether the status is terminal (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Canceled
        )
    }
}

/// Result counters returned by a successful `upsert_documents` call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertReport {
    /// Number of documents the client sent on the wire.
    pub documents_upserted: u32,
    /// Number of documents the client dropped because they were
    /// already present with the same `content_hash` (in-memory dedup).
    pub documents_deduped: u32,
    /// Task uid the Meili server returned.
    pub task_uid: TaskUid,
}

/// One row of the `/indexes` listing — the bare minimum the
/// boot-time stale-sweep needs to decide whether to delete an
/// index. `numberOfDocuments` is preserved verbatim so the sweep
/// can refuse to drop a non-empty index even when it violates the
/// canonical name convention (preserves any operator-injected
/// state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStat {
    /// Index uid (the name).
    pub uid: String,
    /// Document count reported by the Meili `/stats` endpoint.
    pub number_of_documents: u64,
}

/// Failure modes raised by [`MeiliClient`] implementations.
#[derive(Debug, Error)]
pub enum MeiliError {
    /// HTTP transport-layer error.
    #[error("meili http error: {0}")]
    Http(String),
    /// Retry-eligible transient failure (network blip, 5xx, 429).
    #[error("transient meili error: {0}")]
    TransientError(String),
    /// Meili returned a 4xx other than 401/403; the request is bad
    /// and replays will not help. Routes to dead-letter.
    #[error("meili request rejected: {detail}")]
    Rejected {
        /// Status code returned by Meili.
        status: u16,
        /// Human-readable detail.
        detail: String,
    },
    /// Authentication failed — fail fast at startup.
    #[error("meili auth failed: {0}")]
    AuthFailed(String),
    /// Pre-existing settings conflict with the version we're about to
    /// apply. Fatal at startup.
    #[error("meili settings drift: {0}")]
    SettingsDrift(String),
    /// `wait_task` exceeded the allotted timeout.
    #[error("meili task {task} did not finish within {elapsed_ms} ms")]
    TaskTimeout {
        /// Task uid that timed out.
        task: TaskUid,
        /// Wall-clock elapsed before giving up.
        elapsed_ms: u64,
    },
    /// `wait_task` saw the task move to `Failed` / `Canceled`.
    #[error("meili task {task} ended in non-success: {status:?}")]
    TaskNotSucceeded {
        /// Task uid.
        task: TaskUid,
        /// Final status.
        status: TaskStatus,
    },
}

impl MeiliError {
    /// Whether this error category is worth retrying.
    pub fn is_retriable(&self) -> bool {
        matches!(self, MeiliError::TransientError(_))
    }
}

/// Result alias used throughout the client surface.
pub type MeiliResult<T> = Result<T, MeiliError>;

/// Abstraction over a Meilisearch transport.
#[async_trait]
pub trait MeiliClient: Send + Sync {
    /// Create the index if missing, then apply `settings` idempotently.
    /// Returns `true` when settings were freshly applied (counter
    /// hint), `false` when the server reports they already match.
    async fn ensure_index(&self, index: &str, settings: &Value) -> MeiliResult<bool>;

    /// Upsert `docs` into `index`. Returns the Meili task uid.
    async fn upsert_documents(
        &self,
        index: &str,
        docs: &[Document],
    ) -> MeiliResult<UpsertReport>;

    /// Poll `task` until it reaches a terminal status or `timeout`
    /// elapses.
    async fn wait_task(&self, task: TaskUid, timeout: Duration) -> MeiliResult<TaskStatus>;

    /// List every index Meili currently knows about, with its
    /// document count. Used by the boot-time stale-sweep
    /// (phase4a §3) to identify empty non-canonical names.
    async fn list_indexes(&self) -> MeiliResult<Vec<IndexStat>>;

    /// Drop the named index. Idempotent — a `404 Not Found` is
    /// surfaced as `Ok(())` so re-runs of the sweep don't fail
    /// after the first one already deleted it.
    async fn delete_index(&self, index: &str) -> MeiliResult<()>;
}

// ---------- Retry helper -----------------------------------------------

/// Retry helper — `max_attempts` attempts, exponential backoff
/// 100 / 400 / 1600 ms between attempts. Only retries errors classified
/// as [`MeiliError::TransientError`].
///
/// Mirrors the contract of [`crate::embedder::with_retry`] /
/// [`crate::graph::nexus_client::with_retry`] so all three workers
/// behave identically under transient remote-server pressure.
pub async fn with_retry<F, Fut, T>(max_attempts: u32, mut f: F) -> Result<T, MeiliError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, MeiliError>>,
{
    let attempts = max_attempts.max(1);
    let mut last_err: Option<MeiliError> = None;
    for attempt in 0..attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !err.is_retriable() || attempt + 1 == attempts {
                    return Err(err);
                }
                let backoff_ms = 100u64 << (2 * attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        MeiliError::Http("with_retry exhausted attempts without error".into())
    }))
}

// ---------- Live HTTP client ------------------------------------------

/// Production Meili client backed by `reqwest`.
#[derive(Debug, Clone)]
pub struct LiveMeiliClient {
    http: Client,
    base_url: String,
    max_retry: u32,
}

impl LiveMeiliClient {
    /// Build a [`LiveMeiliClient`] from a [`FulltextConfig`].
    pub fn new(config: &FulltextConfig) -> MeiliResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(key) = config.meili_api_key.as_deref() {
            let bearer = format!("Bearer {key}");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&bearer)
                    .map_err(|e| MeiliError::Http(format!("invalid auth header: {e}")))?,
            );
        }
        let http = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| MeiliError::Http(format!("reqwest builder: {e}")))?;
        Ok(Self {
            http,
            base_url: config.meili_url.trim_end_matches('/').to_string(),
            max_retry: config.max_retry,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait]
impl MeiliClient for LiveMeiliClient {
    async fn ensure_index(&self, index: &str, settings: &Value) -> MeiliResult<bool> {
        // Step 1: create the index if missing. POST /indexes is
        // idempotent on (uid, primaryKey) — Meili 1.x returns 202
        // when the index already exists with the same primary key,
        // and 4xx when the primary key conflicts.
        let create_body = serde_json::json!({
            "uid": index,
            "primaryKey": "id",
        });
        let create_url = self.url("/indexes");
        let max = self.max_retry;
        let http = self.http.clone();
        let body_owned = create_body.clone();
        with_retry(max, move || {
            let http = http.clone();
            let url = create_url.clone();
            let body = body_owned.clone();
            async move {
                let resp = http
                    .post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(e, "ensure_index/create"))?;
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                if status == StatusCode::ACCEPTED || status == StatusCode::CREATED {
                    return Ok(());
                }
                if status == StatusCode::CONFLICT {
                    // Index already exists — that's the idempotent path.
                    return Ok(());
                }
                let detail = resp.text().await.unwrap_or_default();
                Err(classify_status(status, "ensure_index/create", &detail))
            }
        })
        .await?;

        // Step 2: apply settings. PATCH /indexes/{uid}/settings.
        // Meili rejects unknown top-level fields, so strip the
        // tooling-only `version` marker baked into settings.v1.json
        // before forwarding the document.
        let settings_url = self.url(&format!("/indexes/{index}/settings"));
        let mut settings_owned = settings.clone();
        if let Some(map) = settings_owned.as_object_mut() {
            map.remove("version");
        }
        let http = self.http.clone();
        let max = self.max_retry;
        with_retry(max, move || {
            let http = http.clone();
            let url = settings_url.clone();
            let body = settings_owned.clone();
            async move {
                let resp = http
                    .patch(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(e, "ensure_index/settings"))?;
                let status = resp.status();
                if status.is_success() || status == StatusCode::ACCEPTED {
                    return Ok(());
                }
                let detail = resp.text().await.unwrap_or_default();
                if status == StatusCode::BAD_REQUEST && detail.contains("incompatible") {
                    return Err(MeiliError::SettingsDrift(detail));
                }
                Err(classify_status(status, "ensure_index/settings", &detail))
            }
        })
        .await?;

        // Always report `true` — the server doesn't expose a "settings
        // unchanged" response, so the indexer treats every successful
        // `ensure_index` as a settings bump for telemetry purposes.
        Ok(true)
    }

    async fn upsert_documents(
        &self,
        index: &str,
        docs: &[Document],
    ) -> MeiliResult<UpsertReport> {
        if docs.is_empty() {
            return Ok(UpsertReport::default());
        }
        // `?primaryKey=id` is required so per-project indexes that the
        // bootstrap walker materialises lazily (e.g. `cortex-vectorizer-code`)
        // are auto-created with the right primary key. Without it Meili
        // 1.x tries to infer from `id` vs `event_id`, finds two
        // candidates ending in `id`, and fails the whole batch with
        // `index_primary_key_multiple_candidates_found`.
        let url = self.url(&format!("/indexes/{index}/documents?primaryKey=id"));
        let payload = serde_json::to_value(docs)
            .map_err(|e| MeiliError::Http(format!("serialize docs: {e}")))?;
        let http = self.http.clone();
        let max = self.max_retry;
        let count = u32::try_from(docs.len()).unwrap_or(u32::MAX);
        let task_uid = with_retry(max, move || {
            let http = http.clone();
            let url = url.clone();
            let body = payload.clone();
            async move {
                let resp = http
                    .post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(e, "upsert_documents"))?;
                let status = resp.status();
                if !status.is_success() && status != StatusCode::ACCEPTED {
                    let detail = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, "upsert_documents", &detail));
                }
                let body: TaskAccepted = resp
                    .json()
                    .await
                    .map_err(|e| MeiliError::Http(format!("decode task body: {e}")))?;
                Ok(body.task_uid)
            }
        })
        .await?;
        Ok(UpsertReport {
            documents_upserted: count,
            documents_deduped: 0,
            task_uid,
        })
    }

    async fn list_indexes(&self) -> MeiliResult<Vec<IndexStat>> {
        // The Meili `/stats` endpoint reports every index in one
        // response (no pagination), unlike `/indexes` which defaults
        // to limit=20. Phase4a's diagnostic showed `/indexes` had
        // hidden 15 of 35 indexes on the live cluster — `/stats`
        // is the trustworthy surface for the sweep.
        let url = self.url("/stats");
        let max = self.max_retry;
        let http = self.http.clone();
        let stats: StatsBody = with_retry(max, move || {
            let http = http.clone();
            let url = url.clone();
            async move {
                let resp = http
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(e, "list_indexes"))?;
                let status = resp.status();
                if !status.is_success() {
                    let detail = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, "list_indexes", &detail));
                }
                resp.json::<StatsBody>()
                    .await
                    .map_err(|e| MeiliError::Http(format!("decode stats body: {e}")))
            }
        })
        .await?;
        let mut out: Vec<IndexStat> = stats
            .indexes
            .into_iter()
            .map(|(uid, body)| IndexStat {
                uid,
                number_of_documents: body.number_of_documents,
            })
            .collect();
        out.sort_by(|a, b| a.uid.cmp(&b.uid));
        Ok(out)
    }

    async fn delete_index(&self, index: &str) -> MeiliResult<()> {
        let url = self.url(&format!("/indexes/{index}"));
        let max = self.max_retry;
        let http = self.http.clone();
        with_retry(max, move || {
            let http = http.clone();
            let url = url.clone();
            async move {
                let resp = http
                    .delete(&url)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(e, "delete_index"))?;
                let status = resp.status();
                // 202 Accepted (task enqueued) and 204 No Content
                // are the documented success paths. 404 is also
                // success — the index already doesn't exist.
                if status.is_success() || status == StatusCode::ACCEPTED
                    || status == StatusCode::NOT_FOUND
                {
                    return Ok(());
                }
                let detail = resp.text().await.unwrap_or_default();
                Err(classify_status(status, "delete_index", &detail))
            }
        })
        .await
    }

    async fn wait_task(&self, task: TaskUid, timeout: Duration) -> MeiliResult<TaskStatus> {
        let url = self.url(&format!("/tasks/{task}"));
        let started = std::time::Instant::now();
        let poll = Duration::from_millis(200);
        loop {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| classify_reqwest(e, "wait_task"))?;
            let status = resp.status();
            if !status.is_success() {
                let detail = resp.text().await.unwrap_or_default();
                return Err(classify_status(status, "wait_task", &detail));
            }
            let task_body: TaskBody = resp
                .json()
                .await
                .map_err(|e| MeiliError::Http(format!("decode task body: {e}")))?;
            if task_body.status.is_terminal() {
                if matches!(task_body.status, TaskStatus::Succeeded) {
                    return Ok(TaskStatus::Succeeded);
                }
                return Err(MeiliError::TaskNotSucceeded {
                    task,
                    status: task_body.status,
                });
            }
            if started.elapsed() >= timeout {
                return Err(MeiliError::TaskTimeout {
                    task,
                    elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                });
            }
            tokio::time::sleep(poll).await;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TaskAccepted {
    #[serde(rename = "taskUid")]
    task_uid: TaskUid,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskBody {
    status: TaskStatus,
}

/// `/stats` response shape — only the bits the sweep needs. Meili
/// also returns `databaseSize`, `usedDatabaseSize`, and per-index
/// field-distribution maps; we drop them on the floor here.
#[derive(Debug, Clone, Deserialize)]
struct StatsBody {
    #[serde(default)]
    indexes: HashMap<String, StatsIndexBody>,
}

#[derive(Debug, Clone, Deserialize)]
struct StatsIndexBody {
    #[serde(rename = "numberOfDocuments", default)]
    number_of_documents: u64,
}

fn classify_reqwest(err: reqwest::Error, purpose: &str) -> MeiliError {
    let is_transient = err.is_timeout()
        || err.is_connect()
        || err
            .status()
            .map(|s| s.is_server_error())
            .unwrap_or(false);
    if is_transient {
        MeiliError::TransientError(format!("{purpose}: {err}"))
    } else {
        MeiliError::Http(format!("{purpose}: {err}"))
    }
}

fn classify_status(status: StatusCode, purpose: &str, detail: &str) -> MeiliError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return MeiliError::AuthFailed(format!("{purpose}: {status}: {detail}"));
    }
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return MeiliError::TransientError(format!("{purpose}: {status}: {detail}"));
    }
    MeiliError::Rejected {
        status: status.as_u16(),
        detail: format!("{purpose}: {detail}"),
    }
}

// ---------- Memory client (tests) -------------------------------------

/// One observation captured by [`MemoryMeiliClient`].
#[derive(Debug, Clone)]
pub enum MemoryCall {
    /// Captured `ensure_index` call.
    EnsureIndex {
        /// Index name.
        name: String,
        /// Settings JSON.
        settings: Value,
    },
    /// Captured `upsert_documents` call.
    UpsertDocuments {
        /// Index name.
        name: String,
        /// Document keys + bodies (cloned).
        docs: Vec<Document>,
    },
    /// Captured `delete_index` call. Phase4a §3 — boot-time
    /// stale-index sweep emits one of these per dropped name.
    DeleteIndex {
        /// Index name that was deleted.
        name: String,
    },
}

/// In-memory Meili client for tests. Records every call without
/// touching a real Meilisearch instance.
#[derive(Debug, Default)]
pub struct MemoryMeiliClient {
    /// Captured calls, in order.
    pub calls: Mutex<Vec<MemoryCall>>,
    /// Sequential task uids the fake hands out.
    next_task: std::sync::atomic::AtomicU64,
    /// Predefined task statuses keyed by task_uid; defaults to
    /// `Succeeded` when missing.
    pub task_statuses: Mutex<HashMap<TaskUid, TaskStatus>>,
    /// Pre-seeded indexes that `list_indexes` returns. Phase4a §3
    /// drives the boot-time stale-sweep off this surface so tests
    /// can simulate the live cluster without a Meili container.
    /// Keys are index names, values the document count.
    pub indexes: Mutex<HashMap<String, u64>>,
}

impl MemoryMeiliClient {
    /// Construct an empty memory client.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of recorded calls, in arrival order.
    pub fn calls_snapshot(&self) -> Vec<MemoryCall> {
        self.calls
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Pre-program a task status the next `wait_task` call will return.
    pub fn set_task_status(&self, task: TaskUid, status: TaskStatus) {
        if let Ok(mut g) = self.task_statuses.lock() {
            g.insert(task, status);
        }
    }

    /// Pre-seed an index (uid + document count) so `list_indexes`
    /// returns it. `delete_index` removes the entry; the sweep tests
    /// poll `index_exists` post-sweep to confirm the deletion.
    pub fn seed_index(&self, uid: &str, number_of_documents: u64) {
        if let Ok(mut g) = self.indexes.lock() {
            g.insert(uid.to_string(), number_of_documents);
        }
    }

    /// Whether the named index is still present in the in-memory
    /// state. Tests use this as the post-sweep oracle.
    pub fn index_exists(&self, uid: &str) -> bool {
        self.indexes
            .lock()
            .map(|g| g.contains_key(uid))
            .unwrap_or(false)
    }
}

#[async_trait]
impl MeiliClient for MemoryMeiliClient {
    async fn ensure_index(&self, name: &str, settings: &Value) -> MeiliResult<bool> {
        if let Ok(mut guard) = self.calls.lock() {
            guard.push(MemoryCall::EnsureIndex {
                name: name.to_string(),
                settings: settings.clone(),
            });
        }
        Ok(true)
    }

    async fn upsert_documents(
        &self,
        name: &str,
        docs: &[Document],
    ) -> MeiliResult<UpsertReport> {
        let task = self
            .next_task
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut guard) = self.calls.lock() {
            guard.push(MemoryCall::UpsertDocuments {
                name: name.to_string(),
                docs: docs.to_vec(),
            });
        }
        Ok(UpsertReport {
            documents_upserted: u32::try_from(docs.len()).unwrap_or(u32::MAX),
            documents_deduped: 0,
            task_uid: task,
        })
    }

    async fn wait_task(&self, task: TaskUid, _timeout: Duration) -> MeiliResult<TaskStatus> {
        let status = self
            .task_statuses
            .lock()
            .map(|g| g.get(&task).copied())
            .unwrap_or(None)
            .unwrap_or(TaskStatus::Succeeded);
        if matches!(status, TaskStatus::Succeeded) {
            Ok(status)
        } else {
            Err(MeiliError::TaskNotSucceeded { task, status })
        }
    }

    async fn list_indexes(&self) -> MeiliResult<Vec<IndexStat>> {
        let snapshot: Vec<IndexStat> = self
            .indexes
            .lock()
            .map(|g| {
                let mut rows: Vec<IndexStat> = g
                    .iter()
                    .map(|(uid, n)| IndexStat {
                        uid: uid.clone(),
                        number_of_documents: *n,
                    })
                    .collect();
                rows.sort_by(|a, b| a.uid.cmp(&b.uid));
                rows
            })
            .unwrap_or_default();
        Ok(snapshot)
    }

    async fn delete_index(&self, index: &str) -> MeiliResult<()> {
        if let Ok(mut g) = self.indexes.lock() {
            g.remove(index);
        }
        if let Ok(mut g) = self.calls.lock() {
            g.push(MemoryCall::DeleteIndex {
                name: index.to_string(),
            });
        }
        Ok(())
    }
}
