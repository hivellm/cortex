//! Phase26f §4.1 — live confirmation of the phase26e §2/§3 pre-thinking
//! observability surface.
//!
//! phase26e §2.1/§3 shipped two `/healthz` extras fields
//! (`pre_thinking_cache_hit_total` / `pre_thinking_cache_miss_total` and
//! `pre_thinking_latency_ms{count,p50,p95,p99}`) and pinned them with a
//! unit test that reads `SyncClient::prethink_metrics()` directly
//! (`sync_paths::tests::prethink_metrics_surfaces_cache_hit_after_repeat_query`).
//! That test never goes over the wire — it calls the metrics handle
//! in-process. This suite closes the gap: it boots the exact
//! production wiring (`SyncClient` → `BundleCache` →
//! `cortex_pre_thinking::pipeline::run` → `PreThinkingMetrics`) behind a
//! real HTTP `/healthz` listener built from the same
//! `cortex_health::server::router` the daemon mounts in
//! `main.rs::run_daemon`, then confirms both tasks.md §4.1 claims via a
//! real HTTP `GET /healthz`:
//!
//! - `pre_thinking_cache_hit_total` increments by exactly one after a
//!   repeated identical query lands within the 60 s bundle-cache TTL.
//! - `pre_thinking_latency_ms.p95` stays under 200 ms across a burst of
//!   distinct queries (normal session load).
//!
//! Redeploying the actual long-lived host adapter daemon and pointing
//! real Claude Code hook traffic at it is an operator action (tasks.md
//! §4.1's own precondition) that this sub-agent must not trigger
//! unilaterally — doing so would restart the process currently relaying
//! hooks for whatever session is attached to it. What this suite
//! provides instead is a repeatable, automated confirmation harness an
//! operator re-runs (`cargo test -p cortex-adapter-claude-code --test
//! prethink_health_live_it`) against the exact production code path
//! immediately after redeploying, rather than hand-curling `/healthz`
//! and eyeballing counters.

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
/// Mounted at `/v1/query` so the pipeline takes the success path and
/// records a `cortex.prethink.latency_ms` sample (the fail-open path
/// used by the other adapter tests never calls `observe_latency_ms`).
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
async fn healthz_cache_hit_total_increments_on_repeated_query_within_ttl() {
    let mock_server = mock_query_server().await;
    let sync = build_sync_client(mock_server.uri());
    let bind = boot_healthz(sync.clone()).await;

    let before = get_healthz(&bind).await;
    let hit_before = before["extras"]["pre_thinking_cache_hit_total"]
        .as_u64()
        .expect("pre_thinking_cache_hit_total present on /healthz extras");

    let r1 = sync
        .pre_thinking(
            "confirm cache hit live",
            "sess-live-it",
            None,
            Some("/repo"),
        )
        .await;
    let r2 = sync
        .pre_thinking(
            "confirm cache hit live",
            "sess-live-it",
            None,
            Some("/repo"),
        )
        .await;
    assert!(!r1.cache_hit, "first call is a cache miss");
    assert!(
        r2.cache_hit,
        "second identical call within the 60s TTL is a cache hit"
    );

    let after = get_healthz(&bind).await;
    let hit_after = after["extras"]["pre_thinking_cache_hit_total"]
        .as_u64()
        .expect("pre_thinking_cache_hit_total present on /healthz extras");
    assert_eq!(
        hit_after - hit_before,
        1,
        "repeated identical query within the TTL must land exactly one \
         cache hit on the live /healthz surface"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_latency_p95_stays_under_200ms_under_normal_session_load() {
    let mock_server = mock_query_server().await;
    let sync = build_sync_client(mock_server.uri());
    let bind = boot_healthz(sync.clone()).await;

    for i in 0..LOAD_QUERY_COUNT {
        sync.pre_thinking(
            &format!("distinct session query {i}"),
            "sess-load-it",
            None,
            Some("/repo"),
        )
        .await;
    }

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
        "every distinct query is a cache miss and records one bundle-assembly \
         latency sample"
    );
    assert!(
        p95 < 200,
        "pre_thinking_latency_ms.p95 must stay under 200ms under normal \
         session load, got {p95}ms"
    );
}
