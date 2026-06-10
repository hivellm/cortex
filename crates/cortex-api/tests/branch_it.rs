//! Phase18 §4.7 — in-process integration tests exercising the branch HTTP
//! surface: validation guards and unconfigured-nexus guards.
//!
//! Each test builds a fresh router backed by an in-memory `QueryService`
//! where `ApiState.nexus` is `None` (no live Nexus backend). This is
//! sufficient to reach every 400 validation path because input validation
//! runs *before* the nexus check. A valid-input request therefore falls
//! through validation and hits the nexus guard, returning 503
//! `nexus_unconfigured`.
//!
//! The live-stack variant (real Nexus row assertions, actual branch
//! creation/merge/abandon) is gated behind a separate IT and is out of
//! scope here. The validation-before-nexus ordering is what makes all
//! 400 paths reachable without a live backend.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, MemoryAuditPublisher, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryService, RateConfig, RateLimiter,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn build_service() -> Arc<QueryService> {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(v, k, g);
    let audit = Arc::new(MemoryAuditPublisher::new());
    let svc = QueryService {
        orchestrator,
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit,
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,
    };
    Arc::new(svc)
}

async fn send(method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let app = build_router(build_service());
    let req = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&b).unwrap()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

// ---------------------------------------------------------------------------
// branch create
// ---------------------------------------------------------------------------

#[tokio::test]
async fn branch_create_rejects_invalid_name() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex",
        Some(json!({ "name": "Bad_Name", "from": "main" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400; body={json}");
    assert_eq!(json["reason"].as_str(), Some("bad_input"), "body={json}");
    assert!(
        json["detail"].as_str().unwrap_or("").starts_with("name:"),
        "detail should start with 'name:'; body={json}"
    );
}

#[tokio::test]
async fn branch_create_valid_name_but_nexus_unconfigured() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex",
        Some(json!({ "name": "feature-x", "from": "main" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "expected 503; body={json}"
    );
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

// ---------------------------------------------------------------------------
// branch merge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn branch_merge_rejects_bad_strategy() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex/feature-x/merge",
        Some(json!({ "strategy": "squash" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400; body={json}");
    assert_eq!(json["reason"].as_str(), Some("bad_input"), "body={json}");
}

#[tokio::test]
async fn branch_merge_valid_strategy_but_nexus_unconfigured() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex/feature-x/merge",
        Some(json!({ "strategy": "accept" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "expected 503; body={json}"
    );
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

// ---------------------------------------------------------------------------
// branch abandon
// ---------------------------------------------------------------------------

#[tokio::test]
async fn branch_abandon_rejects_empty_reason() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex/feature-x/abandon",
        Some(json!({ "reason": "  " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400; body={json}");
    assert_eq!(json["reason"].as_str(), Some("bad_input"), "body={json}");
}

#[tokio::test]
async fn branch_abandon_valid_but_nexus_unconfigured() {
    let (status, json) = send(
        "POST",
        "/v1/branch/cortex/feature-x/abandon",
        Some(json!({ "reason": "superseded by feature-y" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "expected 503; body={json}"
    );
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

// ---------------------------------------------------------------------------
// branch show
// ---------------------------------------------------------------------------

#[tokio::test]
async fn branch_show_rejects_invalid_name() {
    let (status, json) = send("GET", "/v1/branch/cortex/Bad_Name", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400; body={json}");
    assert_eq!(json["reason"].as_str(), Some("bad_input"), "body={json}");
}

#[tokio::test]
async fn branch_show_main_but_nexus_unconfigured() {
    let (status, json) = send("GET", "/v1/branch/cortex/main", None).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "expected 503; main bypasses name validation; body={json}"
    );
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}

// ---------------------------------------------------------------------------
// branch list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn branch_list_nexus_unconfigured() {
    let (status, json) = send("GET", "/v1/branch/cortex", None).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "expected 503; body={json}"
    );
    assert_eq!(
        json["reason"].as_str(),
        Some("nexus_unconfigured"),
        "body={json}"
    );
}
