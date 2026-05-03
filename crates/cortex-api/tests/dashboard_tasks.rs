//! Integration tests for `/v1/dashboard/tasks*`. Builds the dashboard
//! router against a fixture `.rulebook/` tree on a tempdir and drives
//! the three endpoints with `axum::Router::oneshot` so the test never
//! binds a real port.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{build_dashboard_router, DashboardState, MemoryKeywordLane, TaskLoader};
use serde_json::Value;
use tower::ServiceExt;

fn write_task(dir: &std::path::Path, proposal: &str, tasks_md: &str, metadata: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("proposal.md"), proposal).unwrap();
    fs::write(dir.join("tasks.md"), tasks_md).unwrap();
    fs::write(dir.join(".metadata.json"), metadata).unwrap();
}

fn fixture_root() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    // Active in-progress
    write_task(
        &root.join("tasks/phase2g_dashboard_enriched_metrics"),
        "# Proposal: phase2g_dashboard_enriched_metrics\n\n## Why\n\nWiden the dashboard backend.\n",
        "## 1. Backend\n- [x] one\n- [ ] two\n## 2. Frontend\n- [ ] three\n",
        r#"{"status":"in-progress","createdAt":"2026-04-27T00:12:58.590Z","updatedAt":"2026-04-28T17:00:57.841Z"}"#,
    );

    // Active pending
    write_task(
        &root.join("tasks/phase4a_fulltext_fanout"),
        "# Proposal: phase4a_fulltext_fanout\n\n## Why\n\nLand the fan-out.\n",
        "## 1. Stuff\n- [ ] one\n",
        r#"{"status":"pending","createdAt":"2026-04-27T22:45:46.075Z","updatedAt":"2026-04-28T00:15:33.925Z"}"#,
    );

    // Archived completed
    write_task(
        &root.join("archive/2026-04-27-phase2_keyword_lane_live_meilisearch"),
        "# Proposal: phase2_keyword_lane_live_meilisearch\n\n## Why\n\nKeyword lane live wiring.\n",
        "## 1. Foo\n- [x] only\n",
        r#"{"status":"completed","createdAt":"2026-04-27T15:53:44.580Z","updatedAt":"2026-04-27T23:46:18.893Z"}"#,
    );

    tmp
}

fn build_router(root: &std::path::Path) -> axum::Router {
    let state = DashboardState {
        lane: Arc::new(MemoryKeywordLane::new()),
        nexus: None,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(cortex_api::MultiTaskLoader::new(vec![TaskLoader::new(
            root,
        )
        .with_ttl(Duration::from_millis(0))])),
        metadata: None,
        loader_metrics: Arc::new(cortex_api::LoaderMetrics::new()),
        events_bus: cortex_api::dashboard_watcher::DashboardEventBus::new(),
    };
    build_dashboard_router(state)
}

async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, body)
}

#[tokio::test]
async fn tasks_list_returns_active_and_archived_with_breakdowns() {
    let tmp = fixture_root();
    let router = build_router(tmp.path());

    let (status, body) = get_json(&router, "/v1/dashboard/tasks").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 3);
    let ids: Vec<String> = body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids
        .iter()
        .any(|i| i == "phase2g_dashboard_enriched_metrics"));
    assert!(ids.iter().any(|i| i == "phase4a_fulltext_fanout"));
    assert!(ids
        .iter()
        .any(|i| i == "phase2_keyword_lane_live_meilisearch"));

    let by_status = body["by_status"].as_object().unwrap();
    assert_eq!(by_status["in-progress"].as_u64().unwrap(), 1);
    assert_eq!(by_status["pending"].as_u64().unwrap(), 1);
    assert_eq!(by_status["archived"].as_u64().unwrap(), 1);

    let by_phase = body["by_phase"].as_array().unwrap();
    let phases: Vec<&str> = by_phase
        .iter()
        .map(|p| p["phase"].as_str().unwrap())
        .collect();
    assert!(phases.contains(&"phase2"));
    assert!(phases.contains(&"phase2g"));
    assert!(phases.contains(&"phase4a"));
}

#[tokio::test]
async fn tasks_list_filters_by_status_and_archived_flag() {
    let tmp = fixture_root();
    let router = build_router(tmp.path());

    let (status, body) = get_json(&router, "/v1/dashboard/tasks?include_archived=false").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 2);
    let archived_ids: Vec<&str> = body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["status"] == "archived")
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(archived_ids.is_empty());

    let (_, body) = get_json(&router, "/v1/dashboard/tasks?status=archived").await;
    assert_eq!(body["total"].as_u64().unwrap(), 1);
    assert_eq!(body["tasks"][0]["status"].as_str().unwrap(), "archived");

    let (_, body) = get_json(&router, "/v1/dashboard/tasks?phase=phase4a").await;
    assert_eq!(body["total"].as_u64().unwrap(), 1);
    assert_eq!(
        body["tasks"][0]["id"].as_str().unwrap(),
        "phase4a_fulltext_fanout"
    );
}

#[tokio::test]
async fn tasks_summary_aggregates_completion_percentage() {
    let tmp = fixture_root();
    let router = build_router(tmp.path());

    let (status, body) = get_json(&router, "/v1/dashboard/tasks/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 3);
    assert_eq!(body["archived"].as_u64().unwrap(), 1);
    assert_eq!(body["in_progress"].as_u64().unwrap(), 1);
    assert_eq!(body["pending"].as_u64().unwrap(), 1);
    let pct = body["completion_pct"].as_f64().unwrap();
    assert!((pct - 33.3).abs() < 0.5);
}

#[tokio::test]
async fn tasks_detail_returns_proposal_and_sectioned_checklist() {
    let tmp = fixture_root();
    let router = build_router(tmp.path());

    let (status, body) = get_json(
        &router,
        "/v1/dashboard/tasks/phase2g_dashboard_enriched_metrics",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["id"].as_str().unwrap(),
        "phase2g_dashboard_enriched_metrics"
    );
    assert!(body["proposal_md"]
        .as_str()
        .unwrap()
        .contains("Widen the dashboard"));
    let checklist = body["checklist"].as_array().unwrap();
    assert_eq!(checklist.len(), 2);
    assert_eq!(checklist[0]["section"].as_str().unwrap(), "1. Backend");
    assert_eq!(checklist[0]["items"].as_array().unwrap().len(), 2);
    assert!(!body["also_archived"].as_bool().unwrap());
}

#[tokio::test]
async fn tasks_detail_returns_404_for_unknown_id() {
    let tmp = fixture_root();
    let router = build_router(tmp.path());

    let req = Request::builder()
        .uri("/v1/dashboard/tasks/no_such_task")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: Value =
        serde_json::from_slice(&to_bytes(resp.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["error"].as_str().unwrap(), "task_not_found");
}

#[tokio::test]
async fn tasks_endpoints_yield_empty_when_root_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let router = build_router(&tmp.path().join("absent"));

    let (status, body) = get_json(&router, "/v1/dashboard/tasks").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 0);

    let (status, body) = get_json(&router, "/v1/dashboard/tasks/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 0);
    assert_eq!(body["completion_pct"].as_f64().unwrap(), 0.0);
}
