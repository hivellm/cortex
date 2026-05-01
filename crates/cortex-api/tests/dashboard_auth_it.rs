//! Phase3 §8 — dashboard auth integration tests.
//!
//! Drives the `require_api_key` middleware via Axum's
//! `Router::oneshot` so we never bind a real port. Covers the four
//! spec scenarios in
//! `.rulebook/tasks/phase3_gui_multi_connection/specs/gui-connections/spec.md`:
//!
//! - 401 without an `Authorization` header when auth is enabled
//! - 200 with a valid `Authorization: Bearer <key>` header
//! - 401 with a revoked key
//! - 200 anonymous when `CORTEX_DASHBOARD_AUTH=0` (default)
//! - SSE escape-hatch — `?api_key=…` query param accepted identically
//!   to the header
//! - Constant-time compare regression guard (verify uses Argon2id's
//!   built-in compare; the test asserts the path is taken via the
//!   "wrong key" 401 branch)
//!
//! Tests serialise on the `CORTEX_DASHBOARD_AUTH` env var via a
//! per-process mutex — the var is global state, so concurrent tests
//! that set / unset it would race. Each test toggles the var inside
//! a guard and restores the previous value on drop.

use std::sync::{Arc, Mutex, OnceLock};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::auth::{dashboard_auth_enabled, dashboard_cors_layer, ENV_DASHBOARD_AUTH};
use cortex_api::storage::api_keys::ApiKeyStore;
use serde_json::Value;
use tower::ServiceExt;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    prev: Option<String>,
    _lease: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(value: &str) -> Self {
        let lease = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(ENV_DASHBOARD_AUTH).ok();
        std::env::set_var(ENV_DASHBOARD_AUTH, value);
        Self {
            prev,
            _lease: lease,
        }
    }

    fn unset() -> Self {
        let lease = env_lock().lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(ENV_DASHBOARD_AUTH).ok();
        std::env::remove_var(ENV_DASHBOARD_AUTH);
        Self {
            prev,
            _lease: lease,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(ENV_DASHBOARD_AUTH, v),
            None => std::env::remove_var(ENV_DASHBOARD_AUTH),
        }
    }
}

/// Build a tiny Axum router that mounts a single `/v1/dashboard/probe`
/// route returning 200 + a small JSON body. The router is wrapped
/// with the same middleware the daemon's `build_router_with_auth`
/// applies, so requests routed through this fixture exercise the
/// real layer.
fn build_dashboard_with_auth(store: Arc<ApiKeyStore>) -> axum::Router {
    use axum::{routing::get, Json, Router};
    let dashboard = Router::new().route(
        "/v1/dashboard/probe",
        get(|| async { (StatusCode::OK, Json(serde_json::json!({"ok": true}))) }),
    );
    let dashboard = cortex_api::auth::wrap_dashboard_router(dashboard, store);
    let dashboard = dashboard.layer(dashboard_cors_layer());
    Router::new().merge(dashboard)
}

async fn read_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn dashboard_returns_200_anonymously_when_auth_is_disabled() {
    let _g = EnvGuard::unset();
    assert!(!dashboard_auth_enabled());
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["ok"], serde_json::json!(true));
}

#[tokio::test]
async fn dashboard_rejects_anonymous_when_auth_is_enabled() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = read_json(resp).await;
    assert_eq!(
        body["reason"],
        serde_json::json!("missing_or_invalid_api_key")
    );
}

#[tokio::test]
async fn dashboard_accepts_valid_bearer_when_auth_is_enabled() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let issued = store.issue("dashboard", "vt").unwrap();
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .header("authorization", format!("Bearer {}", issued.cleartext))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dashboard_rejects_revoked_bearer() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let issued = store.issue("dashboard", "rv").unwrap();
    store.revoke(&issued.id).unwrap();
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .header("authorization", format!("Bearer {}", issued.cleartext))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = read_json(resp).await;
    assert_eq!(
        body["reason"],
        serde_json::json!("missing_or_invalid_api_key")
    );
}

#[tokio::test]
async fn dashboard_rejects_unknown_bearer() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    store.issue("dashboard", "decoy").unwrap();
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .header(
                    "authorization",
                    "Bearer cortex_dash_zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn dashboard_accepts_api_key_query_param_for_sse_escape_hatch() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let issued = store.issue("dashboard", "sse").unwrap();
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/dashboard/probe?api_key={}", issued.cleartext))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn header_wins_over_query_param_when_both_set() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let valid = store.issue("dashboard", "valid").unwrap();
    let router = build_dashboard_with_auth(store);

    // Header is the valid key; query param is a wrong key.
    // Because the middleware prefers the header, the call succeeds.
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe?api_key=cortex_dash_wrong")
                .header("authorization", format!("Bearer {}", valid.cleartext))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dashboard_rejects_authorization_without_bearer_scheme() {
    let _g = EnvGuard::set("1");
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let issued = store.issue("dashboard", "scheme").unwrap();
    let router = build_dashboard_with_auth(store);

    // Drop the "Bearer " prefix — middleware must refuse.
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/dashboard/probe")
                .header("authorization", issued.cleartext.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn dashboard_cors_layer_responds_to_preflight_for_localhost_origin() {
    // CORS preflight succeeds for the Vite dev origin even when
    // auth is disabled — preflight runs before the auth middleware
    // (it never carries credentials), so the response is 200 with
    // Access-Control-Allow-Origin echoed.
    let _g = EnvGuard::unset();
    let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let router = build_dashboard_with_auth(store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/dashboard/probe")
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Preflight should not be 401 (auth layer) and not 405 (router).
    // tower-http returns 200 with the access-control headers.
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::NO_CONTENT,
        "expected preflight 200/204; got {}",
        resp.status()
    );
    let allow_origin = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(allow_origin, "http://localhost:5173");
}
