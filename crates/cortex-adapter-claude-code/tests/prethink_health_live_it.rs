//! Pre-thinking observability through real loopback HTTP listeners.
//!
//! The production `SyncClient` and pre-thinking pipeline feed the same
//! `cortex_health::server::router` used by the daemon. Memory cache counters
//! reflect validated server responses, with a fresh prepared receipt on each
//! prompt; unavailable or invalid responses must not count as hits or misses.
//! A separate burst checks the latency histogram under normal session load.
//!
//! Upstream APIs are mocked. This hermetic harness does not establish that an
//! installed daemon or Claude Code hook is running this version.

use std::sync::Arc;
use std::time::Duration;

use cortex_adapter_claude_code::{AdapterSection, Metrics, PreThinkingSection, SyncClient};
use cortex_health::server::{router, HealthSnapshot, SnapshotProvider};
use cortex_health::HealthState;
use serde_json::{json, Map, Value};

/// Number of distinct queries fired to simulate "normal session load"
/// before sampling the latency histogram — a realistic burst of
/// `UserPromptSubmit` hooks across one working session.
const LOAD_QUERY_COUNT: usize = 50;

/// A structurally valid `cortex_api::QueryResponse` body — same fixture
/// shape used by `tests/dispatcher.rs::user_prompt_returns_bundle_when_api_responds`.
/// Mounted at `/v1/query` so optional retrieval succeeds independently of
/// local memory availability. Both success and fail-open paths record latency.
fn mock_query_response_body() -> Value {
    json!({
        "intent": "pre_change_context",
        "query_id": "01HX0DEMOQUERY0000000002",
        "scope_resolved": { "repos": [], "topics": [] },
        "results": {
            "snippets": [],
            "decisions": [],
            "violations": [],
            "graph_neighbors": [],
            "similar_turns": []
        },
        "laws_active": [],
        "budget": { "used_ms": 5, "cap_ms": 600, "cache": "miss" },
        "debug": { "lanes": {}, "errors": {} }
    })
}

async fn mock_query_server() -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/query"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(mock_query_response_body()),
        )
        .mount(&server)
        .await;
    server
}

fn build_sync_client(api_endpoint: String) -> Arc<SyncClient> {
    let cfg = AdapterSection {
        api_endpoint,
        pre_thinking: PreThinkingSection {
            timeout_ms: 2_000,
            ..Default::default()
        },
        ..Default::default()
    };
    let metrics = Arc::new(Metrics::new());
    Arc::new(SyncClient::new(&cfg, metrics))
}

/// Mirrors the two phase26e §2.1/§3 `/healthz` extras fields
/// `main.rs::run_daemon` surfaces from `SyncClient::prethink_metrics()`.
/// Only the fields this confirmation cares about are reproduced here —
/// the full daemon health closure carries additional adapter-wide
/// telemetry that is out of scope for this item.
fn snapshot_provider(sync: Arc<SyncClient>) -> SnapshotProvider {
    Arc::new(move || {
        let prethink = sync.prethink_metrics();
        let (cache_hit_total, cache_miss_total) = prethink.cache_counters();
        let (count, p50, p95, p99) = prethink.latency_quantiles();
        let mut extras = Map::new();
        extras.insert(
            "pre_thinking_cache_hit_total".into(),
            json!(cache_hit_total),
        );
        extras.insert(
            "pre_thinking_cache_miss_total".into(),
            json!(cache_miss_total),
        );
        extras.insert(
            "pre_thinking_latency_ms".into(),
            json!({ "count": count, "p50": p50, "p95": p95, "p99": p99 }),
        );
        HealthSnapshot {
            state: HealthState::Ok,
            last_error: None,
            extras,
        }
    })
}

async fn wait_until_listening(addr: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("healthz listener never bound at {addr}");
}

/// Boots the real `cortex_health::server::router` (the same router
/// `main.rs::run_daemon` mounts on the admin port) on a free loopback
/// port, wired to `sync`'s live pre-thinking metrics. Returns the
/// `host:port` an operator-style `curl`/`reqwest` GET can reach.
async fn boot_healthz(sync: Arc<SyncClient>) -> String {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let bind = format!("127.0.0.1:{port}");

    let app = router(
        "cortex-adapter",
        env!("CARGO_PKG_VERSION"),
        "test".into(),
        snapshot_provider(sync),
    );
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    wait_until_listening(&bind).await;
    bind
}

async fn get_healthz(bind: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client
        .get(format!("http://{bind}/healthz"))
        .send()
        .await
        .expect("GET /healthz")
        .json()
        .await
        .expect("healthz body json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_cache_counters_follow_only_valid_server_memory_responses() {
    use wiremock::{
        matchers::{method, path},
        Mock, ResponseTemplate,
    };

    let mock_server = mock_query_server().await;
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/current\n").unwrap();
    std::fs::write(
        repo.path().join("cortex.toml"),
        "[cortex]\nid = \"Exact/Repo\"\n",
    )
    .unwrap();
    let sync = build_sync_client(mock_server.uri());
    let bind = boot_healthz(sync.clone()).await;

    let before = get_healthz(&bind).await;
    let hit_before = before["extras"]["pre_thinking_cache_hit_total"]
        .as_u64()
        .expect("pre_thinking_cache_hit_total present on /healthz extras");

    let miss_before = before["extras"]["pre_thinking_cache_miss_total"]
        .as_u64()
        .expect("pre_thinking_cache_miss_total present on /healthz extras");

    let prepared = |delivery_id: &str, cache_hit: bool| {
        json!({
            "context": "[memory ADR-42 v1 event source-event]\nDurable correction.\n",
            "memories": [{
                "id": "ADR-42", "version": 1, "event_id": "source-event",
                "scope": {"repository": "Exact/Repo"}
            }],
            "revision": 9, "global_revision": null,
            "delivery_id": delivery_id, "stage": "prepared", "cache_hit": cache_hit
        })
    };
    let miss = prepared("prepared-first", false);
    let hit = prepared("prepared-second", true);
    assert_ne!(miss["delivery_id"], hit["delivery_id"]);
    assert_eq!(miss["revision"], hit["revision"]);

    let mut invalid_stage = prepared("prepared-invalid-stage", true);
    invalid_stage["stage"] = json!("consumed");
    let invalid_receipt = prepared("", true);
    let mut invalid_scope = prepared("prepared-invalid-scope", true);
    invalid_scope["memories"][0]["scope"]["repository"] = json!("Other/Repo");
    let mut invalid_revision = prepared("prepared-invalid-revision", true);
    invalid_revision["revision"] = json!("9");

    let cases = [
        (
            "miss",
            ResponseTemplate::new(200).set_body_json(miss),
            true,
            false,
        ),
        (
            "hit",
            ResponseTemplate::new(200).set_body_json(hit),
            true,
            true,
        ),
        (
            "invalid stage",
            ResponseTemplate::new(200).set_body_json(invalid_stage),
            false,
            false,
        ),
        (
            "invalid receipt",
            ResponseTemplate::new(200).set_body_json(invalid_receipt),
            false,
            false,
        ),
        (
            "invalid scope",
            ResponseTemplate::new(200).set_body_json(invalid_scope),
            false,
            false,
        ),
        (
            "invalid revision",
            ResponseTemplate::new(200).set_body_json(invalid_revision),
            false,
            false,
        ),
        ("outage", ResponseTemplate::new(503), false, false),
    ];
    let mut hits = 0;
    let mut misses = 0;
    for (name, response, valid, cache_hit) in cases {
        let _guard = Mock::given(method("POST"))
            .and(path("/v1/memory/query"))
            .respond_with(response)
            .expect(1)
            .mount_as_scoped(&mock_server)
            .await;
        let result = sync
            .pre_thinking(
                "confirm cache hit live",
                "sess-live-it",
                None,
                repo.path().to_str(),
            )
            .await;
        assert_eq!(result.cache_hit, cache_hit, "{name}");
        assert_eq!(
            result.bundle.contains("Durable correction."),
            valid,
            "{name}"
        );
        if valid {
            if cache_hit {
                hits += 1;
            } else {
                misses += 1;
            }
        }
        let after = get_healthz(&bind).await;
        assert_eq!(
            after["extras"]["pre_thinking_cache_hit_total"]
                .as_u64()
                .unwrap()
                - hit_before,
            hits,
            "{name}: only a validated server hit increments hits"
        );
        assert_eq!(
            after["extras"]["pre_thinking_cache_miss_total"]
                .as_u64()
                .unwrap()
                - miss_before,
            misses,
            "{name}: unavailable or invalid responses do not increment misses"
        );
    }

    let requests = mock_server.received_requests().await.unwrap();
    let memory_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/memory/query")
        .collect();
    assert_eq!(
        memory_requests.len(),
        7,
        "every identical prompt must reach the local API for fresh authorization and receipt"
    );
    for request in memory_requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["repo"], "Exact/Repo");
        assert_eq!(body["branch"], "current");
        assert_eq!(body["include_global"], false);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_latency_p95_stays_under_200ms_under_normal_session_load() {
    let mock_server = mock_query_server().await;
    // `/repo` is drive-root-relative on Windows and correctly rejected by
    // the hook origin guard. Exercise a real absolute repository instead.
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    std::fs::write(
        repo.path().join("cortex.toml"),
        "[cortex]\nid = \"Latency/Repo\"\n",
    )
    .unwrap();
    assert!(repo.path().is_absolute());
    let sync = build_sync_client(mock_server.uri());
    let bind = boot_healthz(sync.clone()).await;

    for i in 0..LOAD_QUERY_COUNT {
        sync.pre_thinking(
            &format!("distinct session query {i}"),
            "sess-load-it",
            None,
            repo.path().to_str(),
        )
        .await;
    }

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/v1/query")
            .count(),
        LOAD_QUERY_COUNT,
        "normal session load must exercise optional HTTP retrieval"
    );
    let snap = get_healthz(&bind).await;
    let latency = &snap["extras"]["pre_thinking_latency_ms"];
    let count = latency["count"]
        .as_u64()
        .expect("pre_thinking_latency_ms.count present on /healthz extras");
    let p95 = latency["p95"]
        .as_u64()
        .expect("pre_thinking_latency_ms.p95 present on /healthz extras");
    assert_eq!(
        count, LOAD_QUERY_COUNT as u64,
        "every distinct query records one pre-thinking latency sample"
    );
    assert!(
        p95 < 200,
        "pre_thinking_latency_ms.p95 must stay under 200ms under normal \
         session load, got {p95}ms"
    );
}
