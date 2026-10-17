//! Daily runtime keeps local memory and authorization, without bulk HTTP work.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use cortex_api::{
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryService,
};
use cortex_config::{Config, RuntimeProfile};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};
use tower::ServiceExt;

fn app(path: &Path, profile: RuntimeProfile, bulk_url: &str) -> Router {
    let mut cfg = Config::default();
    cfg.runtime.profile = profile;
    cfg.ingestion.metadata_db = Some(path.to_string_lossy().into_owned());
    cfg.embedder.vectorizer_url = Some(bulk_url.into());
    cfg.meili.meili_url = Some(bulk_url.into());
    cfg.nexus.nexus_url = Some(bulk_url.into());
    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        Arc::new(MemoryKeywordLane::new().with_fail()),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ));
    cortex_api::http::build_router_with_auth_and_cfg(Arc::new(service), None, None, Arc::new(cfg))
}

async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

#[tokio::test]
async fn daily_keeps_capture_read_feedback_restart_and_reports_optional_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let sentinel = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    sentinel.set_nonblocking(true).unwrap();
    let url = format!("http://{}", sentinel.local_addr().unwrap());
    let first = app(&path, RuntimeProfile::Daily, &url);
    let (status, capture) = request(&first, "POST", "/v1/ingest", json!({
        "repo":"runtime-test", "kind":"decision", "body":"dailyprofiletoken exact decision", "memory_id":"daily-decision"
    })).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{capture}");
    let (status, delivery) = request(
        &first,
        "POST",
        "/v1/memory/query",
        json!({
            "repo":"runtime-test", "query":"dailyprofiletoken"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{delivery}");
    assert_eq!(delivery["memories"][0]["id"], "daily-decision");
    let (status, feedback) = request(
        &first,
        "POST",
        "/v1/pre-thinking/feedback",
        json!({
            "repo":"runtime-test", "query_id":delivery["delivery_id"], "helpful":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{feedback}");
    let (status, health) = request(&first, "GET", "/healthz", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{health}");
    let (_, status) = request(&first, "GET", "/v1/status", Value::Null).await;
    assert_eq!(status["runtime_profile"], "daily");
    assert_eq!(status["capabilities"]["daily_memory"]["state"], "ready");
    assert_eq!(
        status["capabilities"]["bulk_indexers"]["state"],
        "disabled_by_profile"
    );
    assert_eq!(
        status["capabilities"]["claude_adapter"]["state"],
        "unprobed"
    );
    assert_eq!(
        status["capabilities"]["daily_memory"]["delivery_consumption"],
        "not_observed"
    );
    let (status, health) = request(&first, "GET", "/v1/health", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{health}");
    drop(first);
    let restarted = app(&path, RuntimeProfile::Daily, &url);
    let (status, delivery) = request(
        &restarted,
        "POST",
        "/v1/memory/query",
        json!({
            "repo":"runtime-test", "query":"dailyprofiletoken"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{delivery}");
    assert_eq!(delivery["memories"][0]["id"], "daily-decision");
    assert_eq!(
        sentinel.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn daily_disables_bulk_routes_before_payload_parsing_and_backend_access() {
    let directory = tempfile::tempdir().unwrap();
    let app = app(
        &directory.path().join("metadata.sqlite"),
        RuntimeProfile::Daily,
        "http://127.0.0.1:1",
    );
    for (method, path) in [
        ("POST", "/v1/search/keyword"),
        ("POST", "/v1/search/vector"),
        ("POST", "/v1/search/graph"),
        ("POST", "/v1/search/events"),
        ("POST", "/v1/topic-cards/search"),
        ("POST", "/v1/decisions/search"),
        ("POST", "/v1/laws/violations"),
        ("GET", "/v1/health/coverage"),
        ("POST", "/v1/admin/forget"),
        ("GET", "/v1/consolidations/recent"),
        ("GET", "/v1/timeline/runtime-test"),
        ("GET", "/v1/sessions/test/timeline"),
    ] {
        let (status, body) = request(&app, method, path, json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}: {body}");
        assert_eq!(body["reason"], "disabled_by_profile", "{path}: {body}");
    }
}

#[tokio::test]
async fn daily_reports_storage_failure_instead_of_false_health() {
    let directory = tempfile::tempdir().unwrap();
    let app = app(
        directory.path(),
        RuntimeProfile::Daily,
        "http://127.0.0.1:1",
    );
    let (status, body) = request(&app, "GET", "/healthz", Value::Null).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let (_, body) = request(&app, "GET", "/v1/status", Value::Null).await;
    assert_eq!(body["capabilities"]["daily_memory"]["state"], "unavailable");
}

#[tokio::test]
async fn daily_profile_keeps_invalid_credential_and_global_capture_denials() {
    let directory = tempfile::tempdir().unwrap();
    for profile in [RuntimeProfile::Full, RuntimeProfile::Daily] {
        let app = app(
            &directory.path().join("metadata.sqlite"),
            profile,
            "http://127.0.0.1:1",
        );
        let (status, body) = request(&app, "POST", "/v1/ingest", json!({
            "repo":"runtime-test", "kind":"decision", "body":"cannot grant global", "global":true
        })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/memory/query")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer invalid")
                    .body(Body::from(r#"{"repo":"runtime-test","query":"token"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn full_profile_retains_bulk_route_payload_contract() {
    let directory = tempfile::tempdir().unwrap();
    let app = app(
        &directory.path().join("metadata.sqlite"),
        RuntimeProfile::Full,
        "http://127.0.0.1:1",
    );
    let (status, _) = request(&app, "POST", "/v1/search/keyword", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
