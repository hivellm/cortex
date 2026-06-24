//! Phase21 §6.2 — ACL routes integration tests.
//!
//! Verifies the five `/v1/acl/*` handlers end-to-end through the axum router:
//!
//! 1. `whoami_no_principal_store_returns_super_admin` — backward-compat
//!    path: when no AC store is wired, whoami returns the system super-admin.
//! 2. `whoami_deny_by_default_returns_public_only` — a deny-by-default store
//!    makes unauthenticated callers public-only.
//! 3. `role_list_returns_registered_bindings` — GET /v1/acl/roles with a
//!    populated store returns those bindings sorted by name.
//! 4. `role_create_forbidden_for_non_admin` — POST /v1/acl/roles with a
//!    public-only principal → 403.
//! 5. `role_create_and_list_for_acl_admin` — POST then GET with a pass-through
//!    (super-admin) store: the new role appears in the list.

use std::sync::{Arc, RwLock};

use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use axum::body::Body;
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, MemoryAuditPublisher, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, Orchestrator, PrincipalStore, QueryService, RateConfig,
    RateLimiter,
};
use serde_json::Value;
use tower::ServiceExt;

fn make_service(ps: Option<PrincipalStore>) -> Arc<QueryService> {
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
        principal_store: ps.map(|s| Arc::new(RwLock::new(s))),
        api_key_store: None,
    })
}

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).expect("response body must be valid JSON")
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
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

/// Phase21 §6.2 — backward-compat path: when no AC store is configured,
/// `GET /v1/acl/whoami` returns the system super-admin principal.
#[tokio::test]
async fn whoami_no_principal_store_returns_super_admin() {
    let svc = make_service(None);
    let app = build_router(svc);
    let resp = app.oneshot(get("/v1/acl/whoami")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    // clearance_level=3 and roles contains "acl_admin" (super_admin)
    assert_eq!(
        body["clearance_level"].as_u64().unwrap_or(0),
        3,
        "super-admin must have clearance_level=3; body={body}"
    );
    assert!(
        body["roles"]
            .as_array()
            .map(|r| r.iter().any(|v| v.as_str() == Some("acl_admin")))
            .unwrap_or(false),
        "super-admin must carry the acl_admin role; body={body}"
    );
}

/// Phase21 §6.2 — deny-by-default store: an unauthenticated caller gets
/// clearance_level=0 and no compartments.
#[tokio::test]
async fn whoami_deny_by_default_returns_public_only() {
    let svc = make_service(Some(PrincipalStore::deny_by_default()));
    let app = build_router(svc);
    let resp = app.oneshot(get("/v1/acl/whoami")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(
        body["clearance_level"].as_u64().unwrap_or(99),
        0,
        "deny-by-default returns clearance_level=0; body={body}"
    );
    let comps = body["compartment_grants"].as_array().cloned().unwrap_or_default();
    assert!(comps.is_empty(), "deny-by-default: no compartments; body={body}");
}

/// Phase21 §6.2 — GET /v1/acl/roles returns all bindings sorted by name.
#[tokio::test]
async fn role_list_returns_registered_bindings() {
    use cortex_api::RoleBinding;
    let mut store = PrincipalStore::deny_by_default();
    store.upsert_binding(
        "finance".to_string(),
        RoleBinding {
            clearance_level: 2,
            compartments: vec!["financial".to_string()],
        },
    );
    store.upsert_binding(
        "viewer".to_string(),
        RoleBinding {
            clearance_level: 0,
            compartments: vec![],
        },
    );
    let svc = make_service(Some(store));
    let app = build_router(svc);
    let resp = app.oneshot(get("/v1/acl/roles")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let roles = body["roles"].as_array().expect("roles must be an array");
    assert_eq!(roles.len(), 2, "must return both bindings; body={body}");
    // Should be sorted: finance < viewer
    assert_eq!(
        roles[0]["name"].as_str().unwrap_or(""),
        "finance",
        "first role should be 'finance' (sorted); body={body}"
    );
    assert_eq!(roles[0]["clearance_level"].as_u64().unwrap_or(99), 2);
    assert_eq!(
        roles[1]["name"].as_str().unwrap_or(""),
        "viewer",
        "second role should be 'viewer'; body={body}"
    );
}

/// Phase21 §6.2 — POST /v1/acl/roles with a public-only (deny-by-default)
/// principal returns 403 forbidden.
#[tokio::test]
async fn role_create_forbidden_for_non_admin() {
    let svc = make_service(Some(PrincipalStore::deny_by_default()));
    let app = build_router(svc);
    let resp = app
        .oneshot(post_json(
            "/v1/acl/roles",
            &serde_json::json!({
                "name": "analyst",
                "clearance_level": 1,
                "compartments": ["financial"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "non-admin must be rejected with 403"
    );
    let body = json_body(resp).await;
    assert_eq!(
        body["reason"].as_str().unwrap_or(""),
        "forbidden",
        "reason must be 'forbidden'; body={body}"
    );
}

/// Phase21 §6.2 — POST /v1/acl/roles with a pass-through (acl_admin) store
/// creates the role, which then appears in GET /v1/acl/roles.
#[tokio::test]
async fn role_create_and_list_for_acl_admin() {
    let svc = make_service(Some(PrincipalStore::pass_through()));
    let router = build_router(svc);

    // Create the role.
    let create_resp = router
        .clone()
        .oneshot(post_json(
            "/v1/acl/roles",
            &serde_json::json!({
                "name": "security-eng",
                "clearance_level": 3,
                "compartments": ["security"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        create_resp.status(),
        StatusCode::OK,
        "acl_admin must be able to create roles"
    );
    let create_body = json_body(create_resp).await;
    assert_eq!(
        create_body["ok"].as_bool().unwrap_or(false),
        true,
        "create response must carry ok=true; body={create_body}"
    );
    assert_eq!(
        create_body["name"].as_str().unwrap_or(""),
        "security-eng",
        "create response must echo the role name; body={create_body}"
    );

    // The new role must appear in the list.
    let list_resp = router.oneshot(get("/v1/acl/roles")).await.unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body = json_body(list_resp).await;
    let roles = list_body["roles"].as_array().expect("roles must be an array");
    let found = roles.iter().any(|r| r["name"].as_str() == Some("security-eng"));
    assert!(
        found,
        "'security-eng' must appear in the role list after creation; body={list_body}"
    );
}
