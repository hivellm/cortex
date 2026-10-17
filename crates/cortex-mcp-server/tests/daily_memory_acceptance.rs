//! Strict acceptance: four cold authenticated callers, real TCP/SQLite, no retries.
//! Client labels are fixtures, not evidence that external models consumed memory.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use axum::{http::StatusCode, response::IntoResponse, Json};
use cortex_api::{
    build_router_with_auth_and_cfg,
    storage::api_keys::{ApiKeyStore, IssuedKey},
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, PrincipalStore,
    QueryService, RoleBinding,
};
use cortex_mcp_server::{Server, ToolContext, ToolRegistry};
use cortex_storage::{
    daily_memory::{MemoryCapture, MemoryCategory, MemoryDeliveryStage, MemoryScope, MemoryStatus},
    MetadataStore,
};
use serde_json::{json, Value};
use tokio::{sync::Barrier, task::JoinHandle};

const REPO: &str = "Acceptance/Exact";
const CURRENT_ID: &str = "retry-current";
const CURRENT_BODY: &str = "ledgerretry: require an idempotency key before retrying writes.";
const PROMPT: &str = "Implement ledgerretry in src/http.rs";

struct RunningApi {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl RunningApi {
    async fn start(database: &Path, key_database: &Path, keys: &[IssuedKey]) -> Self {
        // Reopen persisted credentials on every restart; do not verify/warm them.
        let key_store = Arc::new(ApiKeyStore::open(key_database).unwrap());
        let mut principals = PrincipalStore::deny_by_default();
        principals.upsert_binding(
            "reader",
            RoleBinding {
                clearance_level: 1,
                compartments: vec![],
            },
        );
        let mut service = QueryService::with_memory_defaults(Orchestrator::new(
            Arc::new(MemoryVectorLane::new().with_fail()),
            Arc::new(MemoryKeywordLane::new().with_fail()),
            Arc::new(MemoryGraphLane::new().with_fail()),
        ));
        service.principal_store = Some(Arc::new(RwLock::new(principals)));
        for key in keys {
            service.acl.allow(&key.id, REPO);
        }
        let mut cfg = cortex_config::Config::default();
        cfg.runtime.profile = cortex_config::RuntimeProfile::Daily;
        cfg.ingestion.metadata_db = Some(database.to_string_lossy().into_owned());
        cfg.ingestion.ingestion_url = Some("http://127.0.0.1:1".into());
        cfg.embedder.vectorizer_url = Some("http://127.0.0.1:1".into());
        cfg.meili.meili_url = Some("http://127.0.0.1:1".into());
        cfg.nexus.nexus_url = Some("http://127.0.0.1:1".into());
        cfg.access_control.enabled = true;
        cfg.access_control.deny_on_missing_principal = true;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let router =
            build_router_with_auth_and_cfg(Arc::new(service), None, Some(key_store), Arc::new(cfg))
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| {
                        let observed = observed.clone();
                        async move {
                            let started = Instant::now();
                            let path = request.uri().path().to_owned();
                            // Explicit optional-search outage. Local memory still uses
                            // the production HTTP handler, authorization and SQLite.
                            let response = if path == "/v1/query" {
                                (
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    Json(json!({"reason":"optional_services_unavailable"})),
                                )
                                    .into_response()
                            } else {
                                next.run(request).await
                            };
                            observed.lock().unwrap().push(json!({
                                "path":path, "status":response.status().as_u16(),
                                "elapsed_us":started.elapsed().as_micros()
                            }));
                            response
                        }
                    },
                ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
        Self {
            url,
            requests,
            task,
        }
    }

    async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for RunningApi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn seed(database: &Path) -> cortex_storage::daily_memory::MemoryReceipt {
    let mut store = MetadataStore::open(database).unwrap();
    let mut original = MemoryCapture::new(
        REPO,
        "Original network decision",
        "ledgerretry: retry every write without safeguards.",
        MemoryCategory::Decision,
    );
    original.id = Some("retry-original".into());
    store.capture_daily_memory(&original).unwrap();

    // Related meaning alone must not make an unrelated rule authoritative.
    // This neighbor shares no literal task term and is deliberately unpinned.
    let mut neighbor = MemoryCapture::new(
        REPO,
        "Transient network resilience",
        "A transient transport fault may trigger repetition after exponential backoff.",
        MemoryCategory::Lesson,
    );
    neighbor.id = Some("semantic-neighbor".into());
    store.capture_daily_memory(&neighbor).unwrap();
    for (id, foreign, branch, path, status) in [
        ("foreign-repo", true, None, None, MemoryStatus::Accepted),
        (
            "wrong-branch",
            false,
            Some("release"),
            None,
            MemoryStatus::Accepted,
        ),
        (
            "wrong-path",
            false,
            None,
            Some("tests"),
            MemoryStatus::Accepted,
        ),
        ("unaccepted", false, None, None, MemoryStatus::Proposed),
    ] {
        let mut capture = MemoryCapture::new(
            if foreign { "Acceptance/Other" } else { REPO },
            "Distractor",
            "ledgerretry: unrelated policy must not be delivered.",
            MemoryCategory::Decision,
        );
        capture.id = Some(id.into());
        capture.branch = branch.map(str::to_owned);
        capture.path = path.map(str::to_owned);
        capture.status = status;
        store.capture_daily_memory(&capture).unwrap();
    }
    let mut current = MemoryCapture::new(
        REPO,
        "Corrected network decision",
        CURRENT_BODY,
        MemoryCategory::Decision,
    );
    current.id = Some(CURRENT_ID.into());
    current.branch = Some("main".into());
    current.path = Some("src".into());
    current.supersedes = original.id;
    current.expected_revision = Some(
        store
            .daily_memory_revision(&MemoryScope::Repository(REPO.into()))
            .unwrap(),
    );
    store.capture_daily_memory(&current).unwrap()
}

async fn recall(server: Arc<Server>, barrier: Arc<Barrier>, repo: PathBuf, index: usize) -> Value {
    let recent_file = repo.join("src/http.rs");
    let frame = serde_json::to_vec(&json!({
        "jsonrpc":"2.0", "id":index, "method":"tools/call", "params":{
            "name":"cortex_pre_thinking", "arguments":{
                "user_prompt":PROMPT, "cwd":repo, "budget_ms":600, "budget_bytes":1536,
                "session_id":format!("cold-client-{index}"),
                "recent_files":[{"path":recent_file,"status":"modified","age_seconds":1}]
            }
        }
    }))
    .unwrap();
    barrier.wait().await;
    let started = Instant::now();
    let response =
        match tokio::time::timeout(Duration::from_secs(5), server.handle_frame(&frame)).await {
            Ok(Some(bytes)) => serde_json::from_slice::<Value>(&bytes)
                .unwrap_or_else(|error| json!({"decode_error":error.to_string()})),
            Ok(None) => json!({"missing_response":true}),
            Err(_) => json!({"outer_timeout":true}),
        };
    json!({"client":index, "elapsed_us":started.elapsed().as_micros(), "response":response})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_cold_keys_recall_exact_corrected_memory_after_restart_with_optional_outage() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("metadata.sqlite");
    let key_database = directory.path().join("api-keys.sqlite");
    let repo = directory.path().join("repository");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(repo.join("cortex.toml"), format!("[cortex]\nid='{REPO}'\n")).unwrap();
    // All writes complete sequentially before the concurrent measurement.
    let corrected = seed(&database);
    let issued = ApiKeyStore::open(&key_database).unwrap();
    let keys: Vec<_> = (0..4)
        .map(|index| {
            issued
                .issue_with_role("memory", &format!("cold-{index}"), Some("reader"))
                .unwrap()
        })
        .collect();
    assert!(issued
        .list()
        .unwrap()
        .iter()
        .all(|key| key.last_used_at_ms.is_none()));
    drop(issued);
    let first = RunningApi::start(&database, &key_database, &keys).await;
    let health = reqwest::get(format!("{}/healthz", first.url))
        .await
        .unwrap();
    assert!(health.status().is_success());
    first.stop().await;
    let restarted = RunningApi::start(&database, &key_database, &keys).await;
    let barrier = Arc::new(Barrier::new(4));
    let mut pending = tokio::task::JoinSet::new();
    for (index, key) in keys.iter().enumerate() {
        let server = Arc::new(Server::with_registry(
            ToolContext::new(&restarted.url).with_api_key(&key.cleartext),
            ToolRegistry::for_profile(cortex_config::RuntimeProfile::Daily),
        ));
        pending.spawn(recall(server, barrier.clone(), repo.clone(), index));
    }
    let mut samples = Vec::new();
    while let Some(result) = pending.join_next().await {
        samples.push(result.unwrap_or_else(|error| json!({"join_error":error.to_string()})));
    }
    // Keep every sample, including empty bundles, 503s and missed deadlines.
    // No retry, success-only percentile, or scheduling tolerance changes verdict.
    let http = restarted.requests.lock().unwrap().clone();
    eprintln!(
        "daily_memory_acceptance={}",
        json!({"samples":samples,"http":http})
    );
    let mut failures = Vec::new();
    let mut deliveries = BTreeSet::new();
    let store = MetadataStore::open(&database).unwrap();
    for sample in &samples {
        let response = &sample["response"];
        let output: Value = response["result"]["content"][0]["text"]
            .as_str()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or(Value::Null);
        let mut check = |condition: bool, reason: &str| {
            if !condition {
                failures.push(format!("client {}: {reason}", sample["client"]));
            }
        };
        check(
            sample["elapsed_us"].as_u64().is_some_and(|us| us < 600_000),
            "wall clock exceeded 600 ms",
        );
        check(
            response["result"]["isError"] == false,
            "MCP returned error or no result",
        );
        check(
            output["memory_outcome"] == "prepared",
            "local memory was not prepared",
        );
        check(
            output["fail_open"] == true,
            "optional outage did not fail open",
        );
        check(
            output["memory_cache_hit"] == false,
            "distinct cold caller reused another caller's cache",
        );
        let bundle = output["bundle"].as_str().unwrap_or("");
        check(
            bundle.len() <= 1536 && bundle.contains(CURRENT_BODY),
            "missing exact body or over budget",
        );
        for rejected in [
            "retry-original",
            "semantic-neighbor",
            "foreign-repo",
            "wrong-branch",
            "wrong-path",
            "unaccepted",
        ] {
            check(
                !bundle.contains(rejected),
                "superseded or distractor ID leaked",
            );
        }
        if let Some(id) = output["memory_delivery_id"].as_str() {
            check(
                deliveries.insert(id.to_owned()),
                "delivery ID reused across callers",
            );
            match store.daily_memory_delivery(REPO, id).unwrap() {
                Some(delivery) => {
                    check(
                        delivery.stage == MemoryDeliveryStage::Prepared,
                        "receipt stage changed",
                    );
                    check(
                        delivery.request.context_bytes <= 1536,
                        "receipt bytes over budget",
                    );
                    check(
                        delivery.request.memories.len() <= 5,
                        "more than five delivered memories",
                    );
                    check(
                        delivery.request.memories.len() == 1,
                        "distractor entered prepared receipt",
                    );
                    if let Some(item) = delivery.request.memories.first() {
                        check(
                            item.id == CURRENT_ID
                                && item.version == corrected.version
                                && item.event_id == corrected.event_id,
                            "receipt ID/version/event differs from committed correction",
                        );
                        check(
                            item.scope == MemoryScope::Repository(REPO.into()),
                            "receipt scope differs",
                        );
                    }
                }
                None => check(false, "durable receipt missing"),
            }
        } else {
            check(false, "delivery ID missing");
        }
    }
    if samples.len() != 4 {
        failures.push(format!("expected all four samples, got {}", samples.len()));
    }
    if http
        .iter()
        .filter(|row| row["path"] == "/v1/memory/query" && row["status"] == 200)
        .count()
        != 4
    {
        failures.push("not all four authenticated local HTTP calls succeeded".into());
    }
    if http
        .iter()
        .filter(|row| row["path"] == "/v1/query" && row["status"] == 503)
        .count()
        != 4
    {
        failures.push("not all four optional outages were exercised".into());
    }
    restarted.stop().await;
    assert!(
        failures.is_empty(),
        "acceptance failures (all samples retained):\n{}",
        failures.join("\n")
    );
}
