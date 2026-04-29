//! Phase8b — integration coverage for `/v1/health/freshness` and
//! `/v1/health/divergence`.
//!
//! The fan-out helpers in `cortex_api::health` probe the same target
//! list `/v1/health` uses; running the actual probes from the test
//! requires a live stack. Instead these tests exercise the pure
//! pieces — severity bucketing, freshness-row construction, divergence
//! pair derivation — through the `cortex_api::health` module's
//! public API surface. The behaviour-level pieces (the HTTP routes
//! mounted on the router) are validated indirectly by the unit tests
//! inside `health.rs`.
//!
//! The high-value integration test here is the route-mount: the
//! cortex-api router built with `build_router_with(_, Some(dash))`
//! MUST expose `/v1/health/freshness` and `/v1/health/divergence`
//! and answer with a JSON body that parses as the documented shape.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_api::{
    build_router_with, DashboardState, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryService, TaskLoader,
};
use serde_json::Value;
use tower::ServiceExt;

fn build_test_router() -> Router {
    let lane = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(vector, lane.clone(), graph);
    let service = Arc::new(
        QueryService::with_memory_defaults(orchestrator).with_indexed_repos(lane.clone()),
    );
    let dashboard = DashboardState {
        lane,
        nexus: None,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))),
        metadata: None,
        loader_metrics: Arc::new(LoaderMetrics::new()),
    };
    build_router_with(service, Some(dashboard))
}

async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, parsed)
}

#[tokio::test]
async fn freshness_endpoint_is_mounted_on_router_with_dashboard() {
    // The freshness aggregator probes 17011..17024 — none of which are
    // bound under tests — so every fan-out hit times out and the
    // adapter / ingestion / worker rows are absent. The cortex-api
    // self-rows (api.archive_loader.last_refresh +
    // api.meili_loader.last_refresh) come from the in-process
    // LoaderMetrics, so they're always present even on a cold stack.
    let router = build_test_router();
    let (status, body) = get_json(router, "/v1/health/freshness").await;
    assert_eq!(status, StatusCode::OK, "freshness must answer 200");
    let arr = body.as_array().expect("freshness body is a JSON array");
    let keys: Vec<&str> = arr
        .iter()
        .filter_map(|row| row.get("key").and_then(|v| v.as_str()))
        .collect();
    assert!(
        keys.contains(&"api.archive_loader.last_refresh"),
        "self-row for archive_loader must always be present, got: {keys:?}"
    );
    assert!(
        keys.contains(&"api.meili_loader.last_refresh"),
        "self-row for meili_loader must always be present, got: {keys:?}"
    );
    // Each row carries the documented shape.
    for row in arr {
        assert!(row.get("key").is_some());
        assert!(row.get("last_event_ts_ms").is_some());
        assert!(row.get("gap_seconds").is_some());
        assert!(row.get("severity").is_some());
    }
}

#[tokio::test]
async fn divergence_endpoint_is_mounted_on_router_with_dashboard() {
    let router = build_test_router();
    let (status, body) = get_json(router, "/v1/health/divergence").await;
    assert_eq!(status, StatusCode::OK, "divergence must answer 200");
    let arr = body.as_array().expect("divergence body is a JSON array");
    // Without a live adapter / ingestion the probe times out and
    // produces empty extras, so the pair table is empty. That's the
    // expected shape on a cold stack — the test's invariant is
    // "always returns an array, never crashes".
    for row in arr {
        assert!(row.get("pair").is_some());
        assert!(row.get("upstream").is_some());
        assert!(row.get("downstream").is_some());
        assert!(row.get("delta").is_some());
        assert!(row.get("delta_growth").is_some());
        assert!(row.get("severity").is_some());
    }
}

#[tokio::test]
async fn versions_endpoint_carries_self_row_with_compile_baked_sha() {
    // Phase8c — the versions handler self-reports cortex-api's own
    // version block (read from the cortex-build env vars stamped at
    // compile time). Even on a cold stack with no fan-out targets
    // up, the self-row MUST be present.
    let router = build_test_router();
    let (status, body) = get_json(router, "/v1/health/versions").await;
    assert_eq!(status, StatusCode::OK, "versions must answer 200");
    let running = body
        .get("running_binaries")
        .and_then(|v| v.as_array())
        .expect("running_binaries is an array");
    let names: Vec<&str> = running
        .iter()
        .filter_map(|row| row.get("name").and_then(|v| v.as_str()))
        .collect();
    assert!(
        names.contains(&"cortex-api"),
        "self-row for cortex-api must always be present, got: {names:?}"
    );
    let self_row = running
        .iter()
        .find(|row| row.get("name").and_then(|v| v.as_str()) == Some("cortex-api"))
        .expect("self row");
    // Each row MUST carry the documented version-block fields.
    for field in [
        "git_sha",
        "git_sha_short",
        "build_ts",
        "git_dirty",
        "profile",
        "crate_version",
        "matches_head",
    ] {
        assert!(
            self_row.get(field).is_some(),
            "self row missing field: {field}"
        );
    }
    // Top-level shape invariants.
    assert!(body.get("head_sha").is_some());
    assert!(body.get("head_sha_short").is_some());
    assert!(body.get("drift").and_then(|v| v.as_array()).is_some());
    assert!(body.get("all_in_sync").and_then(|v| v.as_bool()).is_some());
}

#[tokio::test]
async fn metrics_endpoint_renders_loader_metrics_in_prom_text() {
    // Phase8b — `/metrics` on cortex-api carries the LoaderMetrics
    // counters so an external scraper picks them up alongside the
    // workers' /metrics.
    let router = build_test_router();
    let resp = router
        .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let txt = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(txt.contains("cortex_archive_loader_last_refresh_ts_ms"));
    assert!(txt.contains("cortex_meili_loader_last_refresh_ts_ms"));
}
