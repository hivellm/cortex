//! Phase21 §6.3 — deny-on-missing-principal integration tests.
//!
//! Verifies that `/v1/query` and the three raw search routes return
//! `403 forbidden_classified` when:
//! - a `PrincipalStore` is active (AC enabled), and
//! - `CORTEX_ACCESS_CONTROL_DENY_ON_MISSING_PRINCIPAL=1` is set, and
//! - the caller carries no explicit auth credential.
//!
//! When the gate env var is unset or when the principal_store is None
//! (backward-compat), requests proceed normally.

use std::sync::{Arc, RwLock};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{
    build_router, build_router_with_auth_and_cfg, AclStore, AuditStore, InMemoryCache,
    MemoryAuditPublisher, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator,
    PrincipalStore, QueryService, RateConfig, RateLimiter, ENV_DENY_ON_MISSING_PRINCIPAL,
};
use cortex_config::{AccessControlConfig, Config};
use serde_json::Value;
use tower::ServiceExt;

/// Serialize all tests that mutate env vars. Async-aware
/// (`tokio::sync::Mutex`) because each guard is deliberately held
/// across the whole test body's await points; tokio's mutex has no
/// poisoning, so a panicking test can't block the others either.
static ENV_MX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_MX.lock().await
}

fn make_router_with_ac(svc: Arc<QueryService>) -> axum::Router {
    let cfg = Arc::new(Config {
        access_control: AccessControlConfig {
            enabled: true,
            ..AccessControlConfig::default()
        },
        ..Config::default()
    });
    build_router_with_auth_and_cfg(svc, None, None, cfg)
}

fn make_service_with_ac() -> Arc<QueryService> {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(v, k, g);
    let audit = Arc::new(MemoryAuditPublisher::new());
    Arc::new(QueryService {
        orchestrator,
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: audit.clone(),
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,
        // AC active: deny-by-default store
        principal_store: Some(Arc::new(RwLock::new(PrincipalStore::deny_by_default()))),
        api_key_store: None,
    })
}

fn make_service_no_ac() -> Arc<QueryService> {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(v, k, g);
    let audit = Arc::new(MemoryAuditPublisher::new());
    Arc::new(QueryService {
        orchestrator,
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: audit.clone(),
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,
        // AC inactive: no principal store
        principal_store: None,
        api_key_store: None,
    })
}

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn post_json(uri: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Phase21 §6.3 — /v1/query with AC active + gate on + no auth → 403.
#[tokio::test]
async fn query_denies_unauthenticated_when_gate_active() {
    let _lock = env_lock().await;
    std::env::set_var(ENV_DENY_ON_MISSING_PRINCIPAL, "1");

    let svc = make_service_with_ac();
    // Use AC-enabled config directly so we don't rely on env-var config loading.
    let app = make_router_with_ac(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/query",
            &serde_json::json!({
                "query": "anything",
                "intent": "free_search"
            }),
        ))
        .await
        .unwrap();

    std::env::remove_var(ENV_DENY_ON_MISSING_PRINCIPAL);

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "/v1/query must return 403 when gate is active and caller is unauthenticated"
    );
    let body = json_body(resp).await;
    assert_eq!(
        body["reason"].as_str().unwrap_or(""),
        "forbidden_classified",
        "reason must be forbidden_classified; body={body}"
    );
}

/// Phase21 §6.3 — /v1/query with AC active + gate OFF → request proceeds
/// (200 or any non-403, depending on service outcome).
#[tokio::test]
async fn query_passes_unauthenticated_when_gate_off() {
    let _lock = env_lock().await;
    std::env::remove_var(ENV_DENY_ON_MISSING_PRINCIPAL);

    let svc = make_service_with_ac();
    let app = build_router(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/query",
            &serde_json::json!({
                "query": "anything",
                "intent": "free_search"
            }),
        ))
        .await
        .unwrap();

    // Gate is off — the request is dispatched; empty results → 200.
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "/v1/query must NOT 403 when deny_on_missing_principal gate is off; status={:?}",
        resp.status()
    );
}

/// Phase21 §6.3 — /v1/query with no principal_store (backward compat)
/// + gate on → request still proceeds (gate requires AC to be active).
#[tokio::test]
async fn query_passes_when_no_principal_store_even_with_gate() {
    let _lock = env_lock().await;
    std::env::set_var(ENV_DENY_ON_MISSING_PRINCIPAL, "1");

    let svc = make_service_no_ac();
    let app = build_router(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/query",
            &serde_json::json!({
                "query": "anything",
                "intent": "free_search"
            }),
        ))
        .await
        .unwrap();

    std::env::remove_var(ENV_DENY_ON_MISSING_PRINCIPAL);

    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "backward-compat (no principal_store) must never 403 on query; status={:?}",
        resp.status()
    );
}

/// Phase21 §6.3 — keyword search proxy with AC + gate + no auth → 403.
#[tokio::test]
async fn keyword_proxy_denies_unauthenticated_when_gate_active() {
    let _lock = env_lock().await;
    std::env::set_var(ENV_DENY_ON_MISSING_PRINCIPAL, "1");

    let svc = make_service_with_ac();
    let app = make_router_with_ac(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/search/keyword",
            &serde_json::json!({ "index": "cortex_consolidations", "q": "anything" }),
        ))
        .await
        .unwrap();

    std::env::remove_var(ENV_DENY_ON_MISSING_PRINCIPAL);

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "keyword proxy must 403 unauthenticated calls when gate is active"
    );
    let body = json_body(resp).await;
    assert_eq!(body["reason"].as_str().unwrap_or(""), "forbidden_classified");
}

/// Phase21 §6.3 — graph search proxy with AC + gate + no auth → 403.
#[tokio::test]
async fn graph_proxy_denies_unauthenticated_when_gate_active() {
    let _lock = env_lock().await;
    std::env::set_var(ENV_DENY_ON_MISSING_PRINCIPAL, "1");

    let svc = make_service_with_ac();
    let app = make_router_with_ac(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/search/graph",
            &serde_json::json!({ "node_id": "node-1", "mode": "neighbors" }),
        ))
        .await
        .unwrap();

    std::env::remove_var(ENV_DENY_ON_MISSING_PRINCIPAL);

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "graph proxy must 403 unauthenticated calls when gate is active"
    );
    let body = json_body(resp).await;
    assert_eq!(body["reason"].as_str().unwrap_or(""), "forbidden_classified");
}
