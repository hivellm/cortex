//! Vectorizer client abstraction.
//!
//! Wraps the `vectorizer-sdk` v3 surface behind a trait so tests can
//! substitute an in-memory recorder without touching network code. The live
//! variant talks to Vectorizer over HTTP using the SDK's REST façade; the
//! memory variant keeps a per-collection id set so replays report correct
//! `deduped` counts.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ulid::Ulid;
use vectorizer_sdk::client::core::JwtToken;
use vectorizer_sdk::error::VectorizerError;
use vectorizer_sdk::models::{
    BatchTextRequest, DeleteReport, MoveReport, SimilarityMetric, VectorOpResult,
};
use vectorizer_sdk::{ClientConfig, VectorizerClient as SdkClient};

use super::chunker::Chunk;
use super::config::EmbedderConfig;

/// Similarity metric a Vectorizer collection should be configured with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Cosine similarity (Vectorizer default).
    Cosine,
    /// Euclidean distance.
    Euclidean,
    /// Dot product.
    DotProduct,
}

impl Metric {
    /// Lowercase label, matching the Vectorizer REST response casing.
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Cosine => "cosine",
            Metric::Euclidean => "euclidean",
            Metric::DotProduct => "dotproduct",
        }
    }
}

impl From<Metric> for SimilarityMetric {
    fn from(m: Metric) -> Self {
        match m {
            Metric::Cosine => SimilarityMetric::Cosine,
            Metric::Euclidean => SimilarityMetric::Euclidean,
            Metric::DotProduct => SimilarityMetric::DotProduct,
        }
    }
}

/// Declarative schema description used by [`VectorizerClient::ensure_collection`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSchema {
    /// Expected vector dimension.
    pub dim: u32,
    /// Similarity metric.
    pub metric: Metric,
    /// Metadata fields that should be indexed.
    pub metadata_index: Vec<String>,
    /// Whether hybrid (BM25 + dense) retrieval is enabled.
    pub hybrid: bool,
}

impl Default for CollectionSchema {
    fn default() -> Self {
        Self {
            dim: 768,
            metric: Metric::Cosine,
            metadata_index: vec![
                "kind".into(),
                "topics".into(),
                "repo".into(),
                "path".into(),
                "language".into(),
                "severity".into(),
            ],
            hybrid: true,
        }
    }
}

/// Per-chunk mapping emitted by an upsert call.
///
/// The Vectorizer server reassigns a UUID per stored vector regardless of
/// the client-supplied `id` (server bug #4 in the knowledge base), so the
/// canonical primary id can only be recovered from the server's response.
/// `UpsertedChunk` links the client-side dedup key back to that UUID for
/// downstream consumers (graph writer, query API) that need to reference
/// the stored vector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpsertedChunk {
    /// Client-side deterministic dedup key (from [`super::identity::dedup_key`]).
    pub dedup_key: String,
    /// Server-assigned primary id — a UUID on the current Vectorizer
    /// server; treat as opaque string for forward compatibility.
    pub server_id: String,
}

/// Report returned by a Vectorizer upsert call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpsertReport {
    /// Chunks newly written to Vectorizer (counted by successful entries
    /// in the server's batch response).
    pub written: u32,
    /// Chunks skipped because they already exist (matched by `dedup_key`
    /// in the orchestrator's pre-upsert scan — the wire-level response
    /// does not distinguish dedup hits).
    pub deduped: u32,
    /// Per-entry `dedup_key` → server-assigned UUID mapping for every
    /// newly-written chunk. Empty on a re-run where the orchestrator
    /// pre-filtered every chunk.
    #[serde(default)]
    pub new_entries: Vec<UpsertedChunk>,
}

/// Errors surfaced by a Vectorizer client.
#[derive(Debug, thiserror::Error)]
pub enum VectorizerClientError {
    /// HTTP-level failure (non-retriable 4xx other than 429).
    #[error("http {0}")]
    Http(String),
    /// Local schema disagrees with what Vectorizer has on file.
    #[error("schema mismatch in {collection}: {detail}")]
    SchemaMismatch {
        /// Collection that drifted.
        collection: String,
        /// Human-readable detail (dim / metric discrepancy).
        detail: String,
    },
    /// Retriable rate-limit (HTTP 429 / service overloaded).
    #[error("rate limited")]
    RateLimited,
    /// Retriable transport failure (connection refused, 5xx, timeout, …).
    #[error("transport: {0}")]
    Transport(String),
    /// Any other non-retriable failure.
    #[error("other: {0}")]
    Other(String),
}

impl VectorizerClientError {
    /// Whether this error class should be retried by [`with_retry`].
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            VectorizerClientError::RateLimited | VectorizerClientError::Transport(_)
        )
    }
}

/// Heuristic "collection does not exist" detector. v3's HTTP transport
/// surfaces a 404 via `VectorizerError::Server { message: "HTTP 404 ..." }`
/// instead of the typed `CollectionNotFound` variant, so we also sniff the
/// server's JSON body for `error_type: "collection_not_found"` or the
/// plain `HTTP 404` signature.
fn is_collection_not_found(err: &VectorizerError) -> bool {
    match err {
        VectorizerError::CollectionNotFound { .. } => true,
        VectorizerError::Server { message } => {
            let lower = message.to_ascii_lowercase();
            lower.contains("http 404")
                || lower.contains("collection_not_found")
                || lower.contains("not found")
        }
        _ => false,
    }
}

/// Map a raw SDK error onto [`VectorizerClientError`], preserving retry
/// classification.
fn sdk_error(err: VectorizerError) -> VectorizerClientError {
    match err {
        VectorizerError::RateLimit { message } => {
            tracing::debug!(message, "vectorizer rate-limited");
            VectorizerClientError::RateLimited
        }
        VectorizerError::Network { message } => VectorizerClientError::Transport(message),
        VectorizerError::Timeout { timeout_secs } => {
            VectorizerClientError::Transport(format!("timeout after {timeout_secs}s"))
        }
        VectorizerError::Server { message } => {
            // 5xx is retriable; classify everything else as Other.
            if message.contains("5") && message.to_lowercase().contains("server") {
                VectorizerClientError::Transport(message)
            } else if message.contains("429") {
                VectorizerClientError::RateLimited
            } else {
                VectorizerClientError::Other(message)
            }
        }
        VectorizerError::Http(e) => VectorizerClientError::Transport(e.to_string()),
        VectorizerError::Io(e) => VectorizerClientError::Transport(e.to_string()),
        other => VectorizerClientError::Other(other.to_string()),
    }
}

/// Abstraction over the Vectorizer service.
#[async_trait]
pub trait VectorizerClient: Send + Sync + 'static {
    /// Ensure a collection exists with the expected schema. Fails fast on
    /// drift (dim / metric mismatch): schema migrations are a human-in-the-
    /// loop operation, see spec 06 §Decisions 5.
    async fn ensure_collection(
        &self,
        name: &str,
        schema: &CollectionSchema,
    ) -> Result<(), VectorizerClientError>;

    /// Upsert a batch of chunks. The implementation is expected to split
    /// into sub-batches internally if the caller exceeded the server-side
    /// batch limit.
    async fn upsert_chunks(
        &self,
        collection: &str,
        chunks: &[Chunk],
    ) -> Result<UpsertReport, VectorizerClientError>;

    /// Return the subset of `dedup_keys` that already have at least one
    /// stored vector in `collection` (matched against the per-vector
    /// `metadata.dedup_key` field).
    async fn exists_by_dedup_key(
        &self,
        collection: &str,
        dedup_keys: &[String],
    ) -> Result<BTreeSet<String>, VectorizerClientError>;

    /// Phase11o §2 — batch-delete every id in `ids` from `collection`.
    /// Used by the consolidation pruner's hard-purge sink. Per-id
    /// failures (missing vectors) are surfaced via [`DeleteReport`]
    /// without aborting the batch; transport / auth errors return
    /// the canonical [`VectorizerClientError`].
    async fn delete_vectors(
        &self,
        collection: &str,
        ids: &[String],
    ) -> Result<DeleteReport, VectorizerClientError>;

    /// Phase11o §2 — move `ids` from `src` to `dst` atomically per
    /// vector (insert into `dst` BEFORE deleting from `src`). Used by
    /// the tier-demotion sink: `cortex.consolidation.fp32` →
    /// `cortex.consolidation.pq` → `cortex.cold.binary`. Per-id
    /// failures (missing in src, dim mismatch on dst, etc.) populate
    /// [`MoveReport.results`] without aborting the batch.
    async fn move_vectors(
        &self,
        src: &str,
        dst: &str,
        ids: &[String],
    ) -> Result<MoveReport, VectorizerClientError>;
}

/// Chunks per `insert_texts` request. Hard-capped at 64 per the Vectorizer
/// guidance in spec 06 §Vectorizer client.
pub const INSERT_BATCH_SIZE: usize = 64;

/// Maximum number of pages the `exists` listing probe will walk before
/// giving up. `200 * 50 = 10 000` vectors bounds the worst-case listing
/// time on dev-scale collections. Larger collections should use a
/// server-side bulk-id endpoint once one exists.
pub const LIST_PAGE_HARD_LIMIT: usize = 50;

/// Retry helper — 3 attempts, exponential backoff 100 / 400 / 1600 ms.
/// Only retries [`VectorizerClientError::RateLimited`] and
/// [`VectorizerClientError::Transport`].
pub async fn with_retry<F, Fut, T>(max_attempts: u32, mut f: F) -> Result<T, VectorizerClientError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, VectorizerClientError>>,
{
    let attempts = max_attempts.max(1);
    let mut last_err: Option<VectorizerClientError> = None;
    for attempt in 0..attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !err.is_retriable() || attempt + 1 == attempts {
                    return Err(err);
                }
                // 100 ms, 400 ms, 1600 ms — `100 << (2 * attempt)`.
                let backoff_ms = 100u64 << (2 * attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or(VectorizerClientError::Other(
        "with_retry exhausted attempts without error".into(),
    )))
}

/// Live Vectorizer client, backed by `vectorizer-sdk` v3.0.3's HTTP REST
/// surface.
pub struct LiveVectorizerClient {
    config: EmbedderConfig,
    /// Phase11s §3.2 — wrap the SDK client so token rotation can
    /// rebuild it (the SDK 3.0.3 takes the bearer at construction
    /// and never refreshes; rotating the JWT in-place requires
    /// swapping the entire SDK client behind this lock).
    sdk: tokio::sync::RwLock<SdkClient>,
    /// Direct HTTP client used ONLY for the one surface the SDK does not
    /// expose: paginated `GET /collections/{c}/vectors`, needed by
    /// [`VectorizerClient::exists`] because the server's per-id
    /// `GET /collections/{c}/vectors/{id}` is still buggy (returns 200 with
    /// a synthetic `[0.1, 0.1, …]` for any id). See the v3.0.3 rustdoc on
    /// `SdkClient::get_vector` and ADR 0001 for details.
    http: reqwest::Client,
    max_retry: u32,
    /// Phase11s §3.2 — token cache. Empty when the client was
    /// built directly from a static JWT (legacy path); populated
    /// when the client was built via [`Self::with_credentials`]
    /// and is responsible for refreshing.
    token_cache: std::sync::Arc<super::token_cache::TokenCache>,
    /// Phase11s §3.2 — credentials used to re-login when the
    /// cached token nears expiry. `None` means no auto-refresh.
    credentials: Option<VectorizerCredentials>,
}

/// Phase11s §3.2 — credentials the token-refresh path uses. The
/// embedder bin populates these from `CORTEX_EMBEDDER_VECTORIZER_USER`
/// + `CORTEX_EMBEDDER_VECTORIZER_PASSWORD` so a token rotation
/// keeps using the same identity. When the original config carried
/// a raw JWT (no username/password), this stays `None` and the
/// client runs in legacy-no-refresh mode.
#[derive(Debug, Clone)]
pub struct VectorizerCredentials {
    /// Vectorizer base URL the client logs in against.
    pub base_url: String,
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

impl LiveVectorizerClient {
    /// Build a new live client from configuration.
    ///
    /// When `vectorizer_password` holds a bearer token (typically a JWT
    /// minted via [`LiveVectorizerClient::login`]) it is passed through as
    /// the SDK's `api_key`. The SDK 3.0.3 HTTP transport sniffs the
    /// three-segment JWT shape and sends it as `Authorization: Bearer …`.
    pub fn new(config: EmbedderConfig) -> Result<Self, VectorizerClientError> {
        let sdk_config = ClientConfig {
            base_url: Some(config.vectorizer_url.clone()),
            api_key: config.vectorizer_password.clone(),
            timeout_secs: Some(30),
            ..ClientConfig::default()
        };
        let sdk = SdkClient::new(sdk_config).map_err(sdk_error)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| VectorizerClientError::Other(format!("reqwest client: {e}")))?;
        let max_retry = config.max_retry.max(1);
        let token_cache = std::sync::Arc::new(super::token_cache::TokenCache::new());
        // Seed the cache with whatever JWT the config supplies so
        // `last_login_ts_ms` reads correctly before the first
        // refresh fires. Expiry parsed from the JWT's `exp` claim
        // when present; falls back to `now + DEFAULT_TOKEN_TTL_SECS`.
        if let Some(token) = config.vectorizer_password.clone() {
            let now_ms = chrono::Utc::now().timestamp_millis().max(0);
            let expires_at_ms = super::token_cache::parse_jwt_exp_ms(&token).unwrap_or_else(|| {
                now_ms.saturating_add(
                    super::token_cache::DEFAULT_TOKEN_TTL_SECS.saturating_mul(1_000),
                )
            });
            token_cache.record_refresh(token, expires_at_ms, now_ms);
        }
        Ok(Self {
            config,
            sdk: tokio::sync::RwLock::new(sdk),
            http,
            max_retry,
            token_cache,
            credentials: None,
        })
    }

    /// Phase11s §3.2 — build a client that owns its credentials and
    /// automatically refreshes the JWT before expiry. Runs `/auth/login`
    /// once at construction so the cache holds a fresh token; from
    /// there [`Self::ensure_token_fresh`] handles rotation.
    pub async fn with_credentials(
        mut config: EmbedderConfig,
        credentials: VectorizerCredentials,
    ) -> Result<Self, VectorizerClientError> {
        // Initial login. The minted JWT lands on the config so the
        // first SDK build below uses it as the bearer.
        let jwt = Self::login(
            &credentials.base_url,
            &credentials.username,
            &credentials.password,
        )
        .await?;
        config.vectorizer_url = credentials.base_url.clone();
        config.vectorizer_password = Some(jwt.access_token.clone());
        let mut client = Self::new(config)?;
        client.credentials = Some(credentials);
        Ok(client)
    }

    /// Phase11s §3.2 — refresh the JWT when the cache reports
    /// `should_refresh`. No-op when:
    ///   - no credentials are configured (legacy static-JWT mode), or
    ///   - the cached token is still within
    ///     [`super::token_cache::REFRESH_BUFFER_SECS`] of expiry.
    ///
    /// Failures are recorded on the cache via `record_error` so
    /// /healthz surfaces them; the live token is left intact (a
    /// transient login failure does not invalidate a still-valid
    /// bearer). The caller decides whether to propagate the Err
    /// or continue with the stale token.
    pub async fn ensure_token_fresh(&self) -> Result<(), VectorizerClientError> {
        let credentials = match self.credentials.as_ref() {
            Some(c) => c,
            None => return Ok(()), // legacy mode — caller manages tokens
        };
        let now_ms = chrono::Utc::now().timestamp_millis().max(0);
        if !self.token_cache.should_refresh(now_ms) {
            return Ok(());
        }
        match Self::login(
            &credentials.base_url,
            &credentials.username,
            &credentials.password,
        )
        .await
        {
            Ok(jwt) => {
                let expires_at_ms = super::token_cache::parse_jwt_exp_ms(&jwt.access_token)
                    .unwrap_or_else(|| {
                        now_ms.saturating_add(
                            super::token_cache::DEFAULT_TOKEN_TTL_SECS.saturating_mul(1_000),
                        )
                    });
                let token = jwt.access_token.clone();
                self.token_cache
                    .record_refresh(token.clone(), expires_at_ms, now_ms);
                // Rebuild the SDK client with the fresh JWT so
                // every subsequent SDK call rides the new bearer.
                let sdk_config = ClientConfig {
                    base_url: Some(credentials.base_url.clone()),
                    api_key: Some(token),
                    timeout_secs: Some(30),
                    ..ClientConfig::default()
                };
                let new_sdk = SdkClient::new(sdk_config).map_err(sdk_error)?;
                *self.sdk.write().await = new_sdk;
                tracing::info!(
                    last_login_ts_ms = self.token_cache.last_login_ts_ms(),
                    refreshes_total = self.token_cache.refreshes_total(),
                    "vectorizer JWT refreshed"
                );
                Ok(())
            }
            Err(e) => {
                self.token_cache.record_error();
                tracing::warn!(error = %e, "vectorizer JWT refresh failed; keeping stale token");
                Err(e)
            }
        }
    }

    /// Phase11s §3.2 — read-only access to the token cache so the
    /// bin's /healthz extras + Prometheus renderer can surface the
    /// counters.
    pub fn token_cache(&self) -> &std::sync::Arc<super::token_cache::TokenCache> {
        &self.token_cache
    }

    /// Convenience: call `POST /auth/login` via the SDK and return the
    /// minted JWT. Callers then build a follow-up client with
    /// `config.vectorizer_password = Some(jwt.access_token)`.
    pub async fn login(
        base_url: &str,
        username: &str,
        password: &str,
    ) -> Result<JwtToken, VectorizerClientError> {
        let sdk_config = ClientConfig {
            base_url: Some(base_url.to_string()),
            api_key: None,
            timeout_secs: Some(30),
            ..ClientConfig::default()
        };
        let sdk = SdkClient::new(sdk_config).map_err(sdk_error)?;
        sdk.login(username, password).await.map_err(sdk_error)
    }

    /// Build an `Authorization: Bearer <token>` header when the config
    /// supplies one. Used only by the `exists` listing path.
    fn bearer_header(&self) -> Option<String> {
        self.config
            .vectorizer_password
            .as_ref()
            .map(|t| format!("Bearer {t}"))
    }

    /// Underlying configuration.
    pub fn config(&self) -> &EmbedderConfig {
        &self.config
    }

    /// Underlying SDK client — exposed for tests and advanced callers
    /// that need to reach surfaces not mapped through the [`VectorizerClient`]
    /// trait (e.g. `search_vectors`, `delete_collection`).
    ///
    /// Phase11s §3.2 — async because the SDK lives behind an
    /// `RwLock` so [`Self::ensure_token_fresh`] can rebuild it on
    /// JWT rotation. The returned guard is read-only; concurrent
    /// SDK calls share it.
    pub async fn sdk(&self) -> tokio::sync::RwLockReadGuard<'_, SdkClient> {
        self.sdk.read().await
    }

    /// Fetch every stored vector's `payload.dedup_key` in `collection` by
    /// paginating `GET /collections/{c}/vectors?limit=…&offset=…`.
    ///
    /// The SDK 3.0.3 does not expose a `list_vectors` surface even though
    /// its `get_vector` rustdoc recommends one — so we still drive the
    /// endpoint directly. When the SDK grows the method, this fallback
    /// can be deleted.
    ///
    /// Returns the set of `dedup_key` values stored in the collection's
    /// payload bags. Bounded at [`LIST_PAGE_HARD_LIMIT`] pages to avoid
    /// runaway scans against very large collections; callers that need
    /// strict guarantees should switch to the SDK path once it exists.
    async fn list_stored_dedup_keys(
        &self,
        collection: &str,
    ) -> Result<BTreeSet<String>, VectorizerClientError> {
        let mut out = BTreeSet::new();
        let mut offset: usize = 0;
        let page_size: usize = 200;
        for _ in 0..LIST_PAGE_HARD_LIMIT {
            let url = format!(
                "{}/collections/{}/vectors?limit={}&offset={}",
                self.config.vectorizer_url.trim_end_matches('/'),
                collection,
                page_size,
                offset,
            );
            let mut req = self.http.get(&url);
            if let Some(auth) = self.bearer_header() {
                req = req.header(reqwest::header::AUTHORIZATION, auth);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| VectorizerClientError::Transport(e.to_string()))?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Ok(out);
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                let err_msg = format!("HTTP {} {}", status.as_u16(), text);
                return Err(if status.as_u16() == 429 {
                    VectorizerClientError::RateLimited
                } else if status.is_server_error() {
                    VectorizerClientError::Transport(err_msg)
                } else {
                    VectorizerClientError::Http(err_msg)
                });
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| VectorizerClientError::Transport(e.to_string()))?;
            let Some(vectors) = body.get("vectors").and_then(|v| v.as_array()) else {
                break;
            };
            if vectors.is_empty() {
                break;
            }
            for vec in vectors {
                if let Some(key) = vec
                    .get("payload")
                    .and_then(|p| p.get("dedup_key"))
                    .and_then(|v| v.as_str())
                {
                    out.insert(key.to_string());
                }
            }
            if vectors.len() < page_size {
                break;
            }
            offset = offset.saturating_add(page_size);
        }
        Ok(out)
    }
}

#[async_trait]
impl VectorizerClient for LiveVectorizerClient {
    async fn ensure_collection(
        &self,
        name: &str,
        schema: &CollectionSchema,
    ) -> Result<(), VectorizerClientError> {
        // Phase11s §3.2 — refresh the JWT before any SDK call so an
        // expiring token never races a request. No-op when the
        // client runs in legacy static-JWT mode.
        let _ = self.ensure_token_fresh().await;
        let lookup = with_retry(self.max_retry, || async {
            let sdk = self.sdk.read().await;
            match sdk.get_collection_info(name).await {
                Ok(info) => Ok(Some(info)),
                Err(VectorizerError::CollectionNotFound { .. }) => Ok(None),
                Err(err) if is_collection_not_found(&err) => Ok(None),
                Err(other) => Err(sdk_error(other)),
            }
        })
        .await?;

        if let Some(info) = lookup {
            if info.dimension as u32 != schema.dim {
                return Err(VectorizerClientError::SchemaMismatch {
                    collection: name.to_string(),
                    detail: format!(
                        "dimension: local={} vs remote={}",
                        schema.dim, info.dimension
                    ),
                });
            }
            let remote_metric = info.metric.to_lowercase();
            if remote_metric != schema.metric.as_str() {
                return Err(VectorizerClientError::SchemaMismatch {
                    collection: name.to_string(),
                    detail: format!(
                        "metric: local={} vs remote={}",
                        schema.metric.as_str(),
                        remote_metric
                    ),
                });
            }
            return Ok(());
        }

        let metric: SimilarityMetric = schema.metric.into();
        with_retry(self.max_retry, || async {
            let sdk = self.sdk.read().await;
            sdk.create_collection(name, schema.dim as usize, Some(metric))
                .await
                .map(|_| ())
                .map_err(sdk_error)
        })
        .await
    }

    async fn upsert_chunks(
        &self,
        collection: &str,
        chunks: &[Chunk],
    ) -> Result<UpsertReport, VectorizerClientError> {
        if chunks.is_empty() {
            return Ok(UpsertReport::default());
        }
        // SDK 3.0.3 routes `insert_texts` to `POST /insert_texts` and now
        // surfaces the per-entry `BatchResultEntry` with the client-sent
        // id (`client_id`, which we set to the chunk's `dedup_key`) and
        // the server-assigned `vector_ids`. That mapping is what
        // `UpsertedChunk` carries.
        // Phase11s §3.2 — refresh JWT before the upsert loop. The
        // 2026-05-03 incident showed every embed call returning 401
        // when this path ran without rotation; ensure_token_fresh
        // is the structural fix.
        let _ = self.ensure_token_fresh().await;
        let mut total_written = 0u32;
        let mut total_failed = 0u32;
        let mut new_entries: Vec<UpsertedChunk> = Vec::with_capacity(chunks.len());
        for sub in chunks.chunks(INSERT_BATCH_SIZE) {
            let payload: Vec<BatchTextRequest> = sub.iter().map(chunk_to_batch_request).collect();
            let response = with_retry(self.max_retry, || {
                let payload = payload.clone();
                async move {
                    let sdk = self.sdk.read().await;
                    sdk.insert_texts(collection, payload)
                        .await
                        .map_err(sdk_error)
                }
            })
            .await?;
            total_written = total_written.saturating_add(response.successful_operations as u32);
            total_failed = total_failed.saturating_add(response.failed_operations as u32);
            for entry in response.results {
                if entry.status != "ok" {
                    continue;
                }
                let Some(server_id) = entry.vector_ids.into_iter().next() else {
                    continue;
                };
                new_entries.push(UpsertedChunk {
                    dedup_key: entry.client_id,
                    server_id,
                });
            }
        }
        if total_failed > 0 {
            tracing::warn!(
                collection,
                total_failed,
                total_written,
                "some vectorizer upsert operations failed"
            );
        }
        // If the server rejected every entry in the batch (e.g. dim
        // mismatch on `/insert_texts`), the SDK still returns Ok; the
        // failures are reported per-entry. Propagate that as a transport
        // error so the worker's `EmbedError::Vectorizer` path fires
        // instead of silently treating it as success.
        if total_written == 0 && total_failed > 0 {
            let detail = chunks
                .iter()
                .next()
                .map(|c| {
                    format!(
                        "all {} entries failed (collection={}, sample_dedup_key={})",
                        total_failed, collection, c.dedup_key
                    )
                })
                .unwrap_or_else(|| {
                    format!("all {total_failed} entries failed (collection={collection})")
                });
            return Err(VectorizerClientError::Other(detail));
        }
        // `deduped` on the wire stays zero: the v3 server response rolls
        // dedup hits into `inserted` and does not surface a separate count.
        // The orchestrator's `exists_by_dedup_key` pre-check handles dedup
        // accounting before any upsert reaches this call.
        Ok(UpsertReport {
            written: total_written,
            deduped: 0,
            new_entries,
        })
    }

    async fn exists_by_dedup_key(
        &self,
        collection: &str,
        dedup_keys: &[String],
    ) -> Result<BTreeSet<String>, VectorizerClientError> {
        // The per-id `GET /vectors/{id}` endpoint still returns a synthetic
        // 200 for any id (see the v3.0.3 rustdoc note on `get_vector`), so
        // we walk the collection's list view and intersect on the
        // `metadata.dedup_key` field that `chunk_to_batch_request` stores.
        if dedup_keys.is_empty() {
            return Ok(BTreeSet::new());
        }
        let stored = with_retry(self.max_retry, || async {
            self.list_stored_dedup_keys(collection).await
        })
        .await?;
        Ok(dedup_keys
            .iter()
            .filter(|k| stored.contains(*k))
            .cloned()
            .collect())
    }

    async fn delete_vectors(
        &self,
        collection: &str,
        ids: &[String],
    ) -> Result<DeleteReport, VectorizerClientError> {
        if ids.is_empty() {
            return Ok(DeleteReport {
                collection: collection.to_string(),
                count: 0,
                deleted: 0,
                failed: 0,
                results: Vec::new(),
            });
        }
        let _ = self.ensure_token_fresh().await;
        let collection = collection.to_string();
        let ids = ids.to_vec();
        with_retry(self.max_retry, || {
            let collection = collection.clone();
            let ids = ids.clone();
            async move {
                let sdk = self.sdk.read().await;
                sdk.delete_vectors(&collection, &ids)
                    .await
                    .map_err(sdk_error)
            }
        })
        .await
    }

    async fn move_vectors(
        &self,
        src: &str,
        dst: &str,
        ids: &[String],
    ) -> Result<MoveReport, VectorizerClientError> {
        if ids.is_empty() {
            return Ok(MoveReport {
                src: src.to_string(),
                dst: dst.to_string(),
                requested: 0,
                moved: 0,
                failed: 0,
                results: Vec::new(),
            });
        }
        let _ = self.ensure_token_fresh().await;
        let src = src.to_string();
        let dst = dst.to_string();
        let ids = ids.to_vec();
        with_retry(self.max_retry, || {
            let src = src.clone();
            let dst = dst.clone();
            let ids = ids.clone();
            async move {
                let sdk = self.sdk.read().await;
                sdk.move_to_collection(&src, &dst, &ids)
                    .await
                    .map_err(sdk_error)
            }
        })
        .await
    }
}

/// Translate a [`Chunk`] into the SDK's `BatchTextRequest`, flattening
/// metadata into the SDK's `HashMap<String, String>` bag.
///
/// The chunk's `dedup_key` is stored under the `dedup_key` metadata key
/// so [`LiveVectorizerClient::list_stored_dedup_keys`] can round-trip
/// client-side dedup keys even though the server reassigns its own UUIDs
/// on write (documented server bug — see ADR 0001). The `BatchTextRequest`'s
/// `id` field is also set to the dedup key so SDK 3.0.3's
/// `BatchResultEntry::client_id` round-trip resolves to the key we expect.
fn chunk_to_batch_request(chunk: &Chunk) -> BatchTextRequest {
    let mut metadata: HashMap<String, String> = HashMap::with_capacity(16);
    metadata.insert("dedup_key".into(), chunk.dedup_key.clone());
    for (k, v) in flatten_chunk_metadata(chunk) {
        metadata.insert(k, v);
    }
    BatchTextRequest {
        id: chunk.dedup_key.clone(),
        text: chunk.text.clone(),
        metadata: Some(metadata),
    }
}

/// Flatten [`Chunk`] metadata into a list of `(key, value)` string pairs
/// suitable for the Vectorizer server's `metadata` bag. Both the SDK's
/// `BatchTextRequest` path and the server's per-vector `payload` surface
/// use this helper so the server sees identical keys regardless of code
/// path.
fn flatten_chunk_metadata(chunk: &Chunk) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::with_capacity(12);
    out.push(("parent_event_id".into(), chunk.parent_event_id.clone()));
    out.push((
        "parent_content_hash".into(),
        chunk.parent_content_hash.clone(),
    ));
    out.push((
        "chunk_content_hash".into(),
        chunk.chunk_content_hash.clone(),
    ));
    out.push(("kind".into(), format!("{:?}", chunk.metadata.kind)));
    out.push((
        "severity".into(),
        chunk.metadata.severity.as_str().to_string(),
    ));
    out.push((
        "source".into(),
        match chunk.metadata.source {
            super::chunker::ChunkSource::Code => "code".into(),
            super::chunker::ChunkSource::Doc => "doc".into(),
            super::chunker::ChunkSource::Summary => "summary".into(),
            super::chunker::ChunkSource::FallbackWindow => "fallback_window".into(),
            super::chunker::ChunkSource::RawOversize => "raw_oversize".into(),
        },
    ));
    if let Some(repo) = &chunk.metadata.repo {
        out.push(("repo".into(), repo.clone()));
    }
    if let Some(path) = &chunk.metadata.path {
        out.push(("path".into(), path.clone()));
    }
    if let Some(symbol) = &chunk.metadata.symbol {
        out.push(("symbol".into(), symbol.clone()));
    }
    if let Some(lang) = &chunk.metadata.language {
        out.push(("language".into(), lang.clone()));
    }
    if let Some((start, end)) = chunk.metadata.byte_range {
        out.push(("byte_start".into(), start.to_string()));
        out.push(("byte_end".into(), end.to_string()));
    }
    if let Some(pv) = &chunk.metadata.prompt_version {
        out.push(("prompt_version".into(), pv.clone()));
    }
    if !chunk.metadata.topics.is_empty() {
        out.push(("topics".into(), chunk.metadata.topics.join(",")));
    }
    out
}

/// Recorded call against [`MemoryVectorizerClient`].
#[derive(Debug, Clone)]
pub enum MemoryCall {
    /// `ensure_collection(name, schema)`.
    EnsureCollection(String, CollectionSchema),
    /// `upsert_chunks(collection, chunks)`. The recorded chunk list is the
    /// post-filter set actually sent to the client.
    Upsert(String, Vec<Chunk>),
    /// `exists_by_dedup_key(collection, dedup_keys)`.
    ExistsByDedupKey(String, Vec<String>),
    /// `delete_vectors(collection, ids)`.
    DeleteVectors(String, Vec<String>),
    /// `move_vectors(src, dst, ids)`.
    MoveVectors(String, String, Vec<String>),
}

/// In-memory Vectorizer client for tests.
///
/// Stores a per-collection `dedup_key -> server_id` map so replays of the
/// same dedup key return a stable `UpsertedChunk` rather than allocating a
/// fresh UUID. The orchestrator relies on the mapping surviving the
/// round-trip.
#[derive(Default)]
pub struct MemoryVectorizerClient {
    /// Call log, in order.
    pub calls: Mutex<Vec<MemoryCall>>,
    /// Per-collection `dedup_key` → synthetic server UUID map.
    pub dedup_keys_per_collection: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
}

impl MemoryVectorizerClient {
    /// Snapshot of the recorded calls.
    pub fn calls(&self) -> Vec<MemoryCall> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Total stored-chunk count across all collections.
    pub fn stored_count(&self) -> usize {
        self.dedup_keys_per_collection
            .lock()
            .map(|g| g.values().map(|m| m.len()).sum())
            .unwrap_or(0)
    }

    /// Stored-chunk count for a given collection.
    pub fn stored_count_for(&self, collection: &str) -> usize {
        self.dedup_keys_per_collection
            .lock()
            .map(|g| g.get(collection).map(|m| m.len()).unwrap_or(0))
            .unwrap_or(0)
    }
}

#[async_trait]
impl VectorizerClient for MemoryVectorizerClient {
    async fn ensure_collection(
        &self,
        name: &str,
        schema: &CollectionSchema,
    ) -> Result<(), VectorizerClientError> {
        self.calls
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .push(MemoryCall::EnsureCollection(
                name.to_string(),
                schema.clone(),
            ));
        self.dedup_keys_per_collection
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .entry(name.to_string())
            .or_default();
        Ok(())
    }

    async fn upsert_chunks(
        &self,
        collection: &str,
        chunks: &[Chunk],
    ) -> Result<UpsertReport, VectorizerClientError> {
        self.calls
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .push(MemoryCall::Upsert(collection.to_string(), chunks.to_vec()));
        let mut stored = self
            .dedup_keys_per_collection
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?;
        let map = stored.entry(collection.to_string()).or_default();
        let mut written = 0u32;
        let mut deduped = 0u32;
        let mut new_entries: Vec<UpsertedChunk> = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            if map.contains_key(&chunk.dedup_key) {
                deduped = deduped.saturating_add(1);
            } else {
                // Synthetic server id — shape-identical to a UUID so
                // downstream code treats it as opaque.
                let server_id = format!("mem-{}", Ulid::new().to_string().to_lowercase());
                map.insert(chunk.dedup_key.clone(), server_id.clone());
                written = written.saturating_add(1);
                new_entries.push(UpsertedChunk {
                    dedup_key: chunk.dedup_key.clone(),
                    server_id,
                });
            }
        }
        Ok(UpsertReport {
            written,
            deduped,
            new_entries,
        })
    }

    async fn exists_by_dedup_key(
        &self,
        collection: &str,
        dedup_keys: &[String],
    ) -> Result<BTreeSet<String>, VectorizerClientError> {
        self.calls
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .push(MemoryCall::ExistsByDedupKey(
                collection.to_string(),
                dedup_keys.to_vec(),
            ));
        let stored = self
            .dedup_keys_per_collection
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?;
        let empty = BTreeMap::new();
        let map = stored.get(collection).unwrap_or(&empty);
        Ok(dedup_keys
            .iter()
            .filter(|k| map.contains_key(*k))
            .cloned()
            .collect())
    }

    async fn delete_vectors(
        &self,
        collection: &str,
        ids: &[String],
    ) -> Result<DeleteReport, VectorizerClientError> {
        self.calls
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .push(MemoryCall::DeleteVectors(
                collection.to_string(),
                ids.to_vec(),
            ));
        let mut stored = self
            .dedup_keys_per_collection
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?;
        let map = stored.entry(collection.to_string()).or_default();
        let mut deleted = 0usize;
        let mut failed = 0usize;
        let mut results: Vec<VectorOpResult> = Vec::with_capacity(ids.len());
        for (idx, id) in ids.iter().enumerate() {
            // The Memory client stores by `dedup_key` and round-trips a
            // synthetic `server_id` on upsert. Match either to keep
            // tests symmetric with whatever id the caller threads back
            // through the demotion path.
            let removed = if map.remove(id).is_some() {
                true
            } else {
                let by_server: Option<String> = map
                    .iter()
                    .find(|(_, sid)| sid.as_str() == id)
                    .map(|(k, _)| k.clone());
                if let Some(k) = by_server {
                    map.remove(&k);
                    true
                } else {
                    false
                }
            };
            if removed {
                deleted += 1;
                results.push(VectorOpResult {
                    id: Some(id.clone()),
                    status: "ok".into(),
                    error: None,
                    index: Some(idx),
                });
            } else {
                failed += 1;
                results.push(VectorOpResult {
                    id: Some(id.clone()),
                    status: "missing_in_src".into(),
                    error: None,
                    index: Some(idx),
                });
            }
        }
        Ok(DeleteReport {
            collection: collection.to_string(),
            count: ids.len(),
            deleted,
            failed,
            results,
        })
    }

    async fn move_vectors(
        &self,
        src: &str,
        dst: &str,
        ids: &[String],
    ) -> Result<MoveReport, VectorizerClientError> {
        self.calls
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?
            .push(MemoryCall::MoveVectors(
                src.to_string(),
                dst.to_string(),
                ids.to_vec(),
            ));
        let mut stored = self
            .dedup_keys_per_collection
            .lock()
            .map_err(|_| VectorizerClientError::Other("memory client mutex poisoned".into()))?;
        // Pre-create dst so insert-before-delete can land even when no
        // upsert has touched it yet.
        stored.entry(dst.to_string()).or_default();
        let mut moved = 0usize;
        let mut failed = 0usize;
        let mut results: Vec<VectorOpResult> = Vec::with_capacity(ids.len());
        for id in ids {
            // Look up the (dedup_key, server_id) pair in src by either
            // side of the mapping — the live SDK addresses by server id,
            // tests address by dedup key; the trait abstraction has to
            // tolerate both.
            let pair = {
                let src_map = stored.get(src);
                src_map.and_then(|m| {
                    if let Some(sid) = m.get(id) {
                        Some((id.clone(), sid.clone()))
                    } else {
                        m.iter()
                            .find(|(_, sid)| sid.as_str() == id.as_str())
                            .map(|(k, sid)| (k.clone(), sid.clone()))
                    }
                })
            };
            match pair {
                Some((dedup_key, server_id)) => {
                    // Insert into dst FIRST (mirrors the server invariant).
                    let dst_map = stored.entry(dst.to_string()).or_default();
                    dst_map.insert(dedup_key.clone(), server_id.clone());
                    // Then delete from src.
                    if let Some(src_map) = stored.get_mut(src) {
                        src_map.remove(&dedup_key);
                    }
                    moved += 1;
                    results.push(VectorOpResult {
                        id: Some(id.clone()),
                        status: "ok".into(),
                        error: None,
                        index: None,
                    });
                }
                None => {
                    failed += 1;
                    results.push(VectorOpResult {
                        id: Some(id.clone()),
                        status: "missing_in_src".into(),
                        error: None,
                        index: None,
                    });
                }
            }
        }
        Ok(MoveReport {
            src: src.to_string(),
            dst: dst.to_string(),
            requested: ids.len(),
            moved,
            failed,
            results,
        })
    }
}
