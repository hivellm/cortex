//! The daily HTTP boundary also covers merged dashboard and health routers.

use std::{path::Path, process::Command, sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use cortex_api::{
    analyzer::{Analyzer, AnalyzerConfig},
    storage::api_keys::ApiKeyStore,
    DashboardState, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    MultiTaskLoader, Orchestrator, QueryService, TaskLoader, TemporalMetrics,
};
use cortex_config::{Config, RuntimeProfile};
use serde_json::Value;
use tower::ServiceExt;

const CHILD_FLAG: &str = "CORTEX_DAILY_DASHBOARD_TEST_CHILD";
const TEST_NAME: &str = "daily_dashboard_boundary_preserves_auth_without_optional_probes";

fn app(cfg: Config, root: &Path, optional_url: &str) -> (Router, String) {
    let lane = Arc::new(MemoryKeywordLane::new());
    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        lane.clone(),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ));
    let dashboard = DashboardState {
        lane,
        nexus: None,
        analyzer: Arc::new(Analyzer::new(AnalyzerConfig {
            claude_bin: root.join("absent-claude").to_string_lossy().into_owned(),
            model: "unused-test-model".into(),
            timeout: Duration::from_millis(100),
            max_events: 1,
            api_key: Some("synthetic-unused-test-key".into()),
            api_base: optional_url.into(),
        })),
        tasks: Arc::new(MultiTaskLoader::new(vec![TaskLoader::new(
            root.join(".rulebook"),
        )])),
        metadata: None,
        loader_metrics: Arc::new(LoaderMetrics::new()),
        temporal_metrics: Arc::new(TemporalMetrics::new()),
        events_bus: cortex_api::dashboard_watcher::DashboardEventBus::new(),
        acl_metrics: None,
    };
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let key = keys.issue("dashboard", "daily-dashboard-test").unwrap();
    (
        cortex_api::http::build_router_with_auth_and_cfg(
            Arc::new(service),
            Some(dashboard),
            Some(keys),
            Arc::new(cfg),
        ),
        key.cleartext,
    )
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (StatusCode, Value) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    })
    .await
    .unwrap_or_else(|_| panic!("daily route blocked on optional work: {path}"))
}

async fn assert_child_routes() {
    assert!(cortex_api::auth::dashboard_auth_enabled());
    let cfg = Config::load().unwrap();
    assert!(cfg.is_daily());
    let root = std::env::current_dir().unwrap();
    let optional_url = cfg.meili.meili_url.as_deref().unwrap().to_owned();
    let (app, key) = app(cfg, &root, &optional_url);
    for path in [
        "/v1/health/freshness",
        "/v1/health/divergence",
        "/v1/health/versions",
        "/v1/health/stream",
        "/v1/health/config",
        "/v1/health/coverage",
        "/v1/dashboard/overview",
        "/v1/dashboard/timeline/recent",
        "/v1/dashboard/timeline/stream",
        "/v1/dashboard/stream",
        "/v1/dashboard/memory",
        "/v1/dashboard/decisions",
        "/v1/dashboard/decisions/test-decision",
        "/v1/dashboard/laws",
        "/v1/dashboard/violations",
        "/v1/dashboard/analyses",
        "/v1/dashboard/analyses/test-analysis",
        "/v1/dashboard/consolidations",
        "/v1/dashboard/consolidations/test-consolidation",
        "/v1/dashboard/tools/stats",
        "/v1/dashboard/graph",
        "/v1/dashboard/graph/communities",
        "/v1/dashboard/graph/path",
        "/v1/dashboard/graph/compare",
        "/v1/dashboard/sessions",
        "/v1/dashboard/trust",
        "/v1/dashboard/conversations",
        "/v1/dashboard/conversations/test-session",
        "/v1/dashboard/conversations/test-session/summary",
        "/v1/dashboard/handoffs",
        "/v1/dashboard/classifications",
        "/v1/dashboard/tasks",
        "/v1/dashboard/tasks/summary",
        "/v1/dashboard/tasks/test-task",
        "/v1/dashboard/coverage",
        "/v1/dashboard/producers",
        "/v1/dashboard/canary",
        "/v1/dashboard/acl-stats",
        "/v1/retention/state",
        "/v1/retention/sweeps",
        "/v1/admin/list-events",
    ] {
        // Profile rejection precedes even the dashboard's auth middleware.
        for token in [None, Some(key.as_str())] {
            let (status, body) = request(&app, "GET", path, token, "").await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}: {body}");
            assert_eq!(body["reason"], "disabled_by_profile", "{path}: {body}");
            assert_eq!(body["profile"], "daily", "{path}: {body}");
        }
    }
    let (status, body) = request(&app, "POST", "/v1/search/vector", None, "{invalid JSON").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["reason"], "disabled_by_profile");

    for token in [None, Some("invalid-dashboard-key")] {
        let (status, body) = request(&app, "GET", "/v1/dashboard/active-work", token, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["reason"], "missing_or_invalid_api_key");
    }
    let (status, body) = request(&app, "GET", "/v1/dashboard/active-work", Some(&key), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["active_tasks"][0]["id"], "phase1_daily_guard");
    assert_eq!(body["active_tasks"][0]["status"], "in-progress");
    assert_eq!(
        body["active_tasks"][0]["next_unchecked_item"],
        "Retain daily context"
    );
}

#[test]
fn daily_dashboard_boundary_preserves_auth_without_optional_probes() {
    if std::env::var(CHILD_FLAG).as_deref() == Ok("1") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(assert_child_routes());
        return;
    }

    // Isolate all legacy environment readers in a child process. Parent test
    // threads never mutate environment, and every optional target is a trap.
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let task = root.join(".rulebook/tasks/phase1_daily_guard");
    std::fs::create_dir_all(&task).unwrap();
    std::fs::write(task.join(".metadata.json"), r#"{"status":"in-progress"}"#).unwrap();
    std::fs::write(task.join("tasks.md"), "- [ ] Retain daily context\n").unwrap();
    let sentinel = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    sentinel.set_nonblocking(true).unwrap();
    let url = format!("http://{}", sentinel.local_addr().unwrap());
    let mut cfg = Config::default();
    cfg.runtime.profile = RuntimeProfile::Daily;
    cfg.ingestion.metadata_db = Some(root.join("metadata.sqlite").to_string_lossy().into_owned());
    cfg.rulebook.root = Some(root.join(".rulebook").to_string_lossy().into_owned());
    cfg.embedder.vectorizer_url = Some(url.clone());
    cfg.meili.meili_url = Some(url.clone());
    cfg.nexus.nexus_url = Some(url.clone());
    let config_path = root.join("runtime.toml");
    std::fs::write(&config_path, toml::to_string(&cfg).unwrap()).unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("CORTEX_") {
            child.env_remove(name);
        }
    }
    for name in [
        "CORTEX_ADAPTER_ADMIN_URL",
        "CORTEX_INGESTION_URL",
        "CORTEX_CLASSIFIER_WORKER_URL",
        "CORTEX_EMBEDDER_WORKER_URL",
        "CORTEX_FULLTEXT_WORKER_URL",
        "CORTEX_GRAPH_WORKER_URL",
        "VECTORIZER_URL",
    ] {
        child.env(name, &url);
    }
    let output = child
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_FLAG, "1")
        .env("CORTEX_CONFIG_FILE", config_path)
        .env("CORTEX_DASHBOARD_AUTH", "1")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated daily dashboard test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        sentinel.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "daily endpoints must not connect to optional services"
    );
}
