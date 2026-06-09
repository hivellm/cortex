//! Phase15f §4.2 — integration coverage for `GET /v1/dashboard/canary`.
//!
//! Tests that the endpoint:
//! - Returns 200 with `{ runs: [], last_failure: null }` when no metadata
//!   store is wired.
//! - Returns the runs newest-first with `last_failure` pointing at the
//!   most-recent error row when the metadata store has error ticks.
//! - Returns `last_failure: null` when all ticks in the window are "ok".
//!
//! Also exercises the §2 alarm condition indirectly: two error rows in
//! `canary_runs` are the precondition for `run_pre_thinking_health_canary_loop`
//! to emit the structured WARN. The endpoint surfacing both rows in `runs`
//! and `last_failure` confirms the storage path is wired end-to-end.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_api::{
    build_router_with, DashboardState, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryService, TaskLoader,
};
use cortex_storage::metadata::MetadataStore;
use serde_json::Value;
use tower::ServiceExt;

fn build_test_router_no_metadata() -> Router {
    let lane = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(vector, lane.clone(), graph);
    let service =
        Arc::new(QueryService::with_memory_defaults(orchestrator).with_indexed_repos(lane.clone()));
    let dashboard = DashboardState {
        lane,
        nexus: None,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(cortex_api::MultiTaskLoader::new(vec![TaskLoader::new(
            std::path::PathBuf::from("__tests_no_rulebook__"),
        )])),
        metadata: None,
        loader_metrics: Arc::new(LoaderMetrics::new()),
        temporal_metrics: Arc::new(cortex_api::TemporalMetrics::new()),
        events_bus: cortex_api::dashboard_watcher::DashboardEventBus::new(),
    };
    build_router_with(service, Some(dashboard))
}

fn build_test_router_with_metadata(store: MetadataStore) -> Router {
    let lane = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(vector, lane.clone(), graph);
    let service =
        Arc::new(QueryService::with_memory_defaults(orchestrator).with_indexed_repos(lane.clone()));
    let dashboard = DashboardState {
        lane,
        nexus: None,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(cortex_api::MultiTaskLoader::new(vec![TaskLoader::new(
            std::path::PathBuf::from("__tests_no_rulebook__"),
        )])),
        metadata: Some(Arc::new(Mutex::new(store))),
        loader_metrics: Arc::new(LoaderMetrics::new()),
        temporal_metrics: Arc::new(cortex_api::TemporalMetrics::new()),
        events_bus: cortex_api::dashboard_watcher::DashboardEventBus::new(),
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
async fn canary_endpoint_returns_empty_when_no_metadata() {
    let router = build_test_router_no_metadata();
    let (status, body) = get_json(router, "/v1/dashboard/canary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["runs"].as_array().unwrap().len(), 0);
    assert!(body["last_failure"].is_null());
}

#[tokio::test]
async fn canary_endpoint_returns_empty_when_no_runs() {
    let store = MetadataStore::open_in_memory().unwrap();
    let router = build_test_router_with_metadata(store);
    let (status, body) = get_json(router, "/v1/dashboard/canary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["runs"].as_array().unwrap().len(), 0);
    assert!(body["last_failure"].is_null());
}

#[tokio::test]
async fn canary_endpoint_surfaces_error_ticks_as_last_failure() {
    let store = MetadataStore::open_in_memory().unwrap();
    // Insert two ok ticks followed by two error ticks (newest first = DESC order).
    // Timestamps are intentionally ordered so the newest runs appear first.
    store
        .insert_canary_run("2026-06-09T12:03:00Z", "error", None, Some("http 503"))
        .unwrap();
    store
        .insert_canary_run("2026-06-09T12:02:00Z", "error", None, Some("http 500"))
        .unwrap();
    store
        .insert_canary_run("2026-06-09T12:01:00Z", "ok", Some(12), None)
        .unwrap();
    store
        .insert_canary_run("2026-06-09T12:00:00Z", "ok", Some(10), None)
        .unwrap();

    let router = build_test_router_with_metadata(store);
    let (status, body) = get_json(router, "/v1/dashboard/canary").await;
    assert_eq!(status, StatusCode::OK);

    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 4);

    // Newest first: the two error rows come before the ok rows.
    assert_eq!(runs[0]["status"].as_str().unwrap(), "error");
    assert_eq!(runs[1]["status"].as_str().unwrap(), "error");
    assert_eq!(runs[2]["status"].as_str().unwrap(), "ok");
    assert_eq!(runs[3]["status"].as_str().unwrap(), "ok");

    // last_failure should point at the most-recent error (runs[0]).
    let lf = &body["last_failure"];
    assert!(!lf.is_null(), "last_failure must be set when errors exist");
    assert_eq!(lf["status"].as_str().unwrap(), "error");
    assert_eq!(lf["ts"].as_str().unwrap(), "2026-06-09T12:03:00Z");
    assert_eq!(lf["error_message"].as_str().unwrap(), "http 503");
}

#[tokio::test]
async fn canary_endpoint_last_failure_null_when_all_ok() {
    let store = MetadataStore::open_in_memory().unwrap();
    store
        .insert_canary_run("2026-06-09T12:01:00Z", "ok", Some(9), None)
        .unwrap();
    store
        .insert_canary_run("2026-06-09T12:00:00Z", "ok", Some(11), None)
        .unwrap();

    let router = build_test_router_with_metadata(store);
    let (status, body) = get_json(router, "/v1/dashboard/canary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["runs"].as_array().unwrap().len(), 2);
    assert!(
        body["last_failure"].is_null(),
        "last_failure must be null when all ticks are ok"
    );
}
