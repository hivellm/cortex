//! Phase21 §5.7 — raw search proxy ACL integration tests.
//!
//! Verifies that `/v1/search/keyword`, `/v1/search/vector`, and
//! `/v1/search/graph` do NOT leak classified rows to low-clearance
//! principals. These handlers bypass the fusion orchestrator, so each
//! has its own independent enforcement point tested here.

use std::sync::{Arc, Mutex, RwLock};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Json;
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, MemoryAuditPublisher, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, Orchestrator, PrincipalStore, QueryService, RateConfig,
    RateLimiter,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Serialize all env-var-mutating tests so mock server ports don't
/// conflict. Async-aware (`tokio::sync::Mutex`) because each guard is
/// deliberately held across the whole test body's await points.
static ENV_MX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn min_service(ps: PrincipalStore) -> Arc<QueryService> {
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
        principal_store: Some(Arc::new(RwLock::new(ps))),
        api_key_store: None,
    })
}

/// Spawn a bare axum server that captures POST body JSON and responds with
/// `response` for every request. Returns the bound port.
async fn mock_backend(response: Value, captured: Arc<Mutex<Vec<Value>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cap = Arc::clone(&captured);
    let app = axum::Router::new().route(
        "/{*path}",
        post(move |Json(body): Json<Value>| {
            let cap = Arc::clone(&cap);
            let resp = response.clone();
            async move {
                cap.lock().unwrap().push(body);
                Json(resp)
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

async fn read_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).expect("response body must be JSON")
}

/// Phase21 §5.7 — keyword proxy: a public-only principal must cause the
/// handler to inject an ACL level-0 filter into the Meili request body.
///
/// The mock Meili captures the request JSON; we assert `filter` contains
/// `class_level <= 0` — the gate that prevents classified rows from being
/// returned even if they exist in the index.
#[tokio::test]
async fn keyword_proxy_injects_acl_filter_for_non_admin() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let port = mock_backend(
        json!({ "hits": [], "processingTimeMs": 1, "estimatedTotalHits": 0 }),
        Arc::clone(&captured),
    )
    .await;

    let _lock = ENV_MX.lock().await;
    std::env::set_var("CORTEX_FULLTEXT_MEILI_URL", format!("http://127.0.0.1:{port}"));

    let svc = min_service(PrincipalStore::deny_by_default());
    let app = build_router(svc);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search/keyword")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "index": "cortex-consolidations",
                        "q": "secret"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "handler must reach Meili and return 200");

    let reqs = captured.lock().unwrap();
    assert!(!reqs.is_empty(), "mock Meili must have received at least one request");
    // The last request should be the search (not a login call).
    let search_body = reqs.last().unwrap();
    let filter = search_body["filter"]
        .as_str()
        .expect("ACL filter must be injected into the Meili request body");
    assert!(
        filter.contains("class_level"),
        "ACL level clause must appear in filter; got: {filter:?}"
    );
    assert!(
        filter.contains("class_level <= 0"),
        "public-only principal (clearance=0) must restrict to level 0; got: {filter:?}"
    );
}

/// Phase21 §5.7 — vector proxy: classified hits returned by Vectorizer are
/// dropped client-side before the response reaches the caller.
///
/// The mock Vectorizer returns a public hit (level=0) and a restricted hit
/// (level=3). A public-only principal must receive only the public hit.
#[tokio::test]
async fn vector_proxy_drops_classified_hits_for_non_admin() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let port = mock_backend(
        json!({
            "results": [
                {
                    "id": "public-doc",
                    "score": 0.9,
                    "payload": { "class_level": 0, "class_compartments": [] }
                },
                {
                    "id": "restricted-doc",
                    "score": 0.8,
                    "payload": { "class_level": 3, "class_compartments": ["security"] }
                }
            ]
        }),
        Arc::clone(&captured),
    )
    .await;

    let _lock = ENV_MX.lock().await;
    std::env::set_var(
        "CORTEX_EMBEDDER_VECTORIZER_URL",
        format!("http://127.0.0.1:{port}"),
    );
    // Clear credential env vars so the handler doesn't attempt a /auth/login
    // round-trip to the mock server before the search request.
    std::env::remove_var("CORTEX_EMBEDDER_VECTORIZER_JWT");
    std::env::remove_var("CORTEX_EMBEDDER_VECTORIZER_USER");
    std::env::remove_var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD");

    let svc = min_service(PrincipalStore::deny_by_default());
    let app = build_router(svc);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search/vector")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "collection": "cortex.fp32",
                        "query_vector": [0.1f32, 0.2, 0.3]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = read_json(resp).await;
    let hits = body["hits"].as_array().expect("response must have `hits` array");
    let ids: Vec<&str> = hits.iter().filter_map(|h| h["id"].as_str()).collect();
    assert!(ids.contains(&"public-doc"), "public hit must pass through; ids={ids:?}");
    assert!(
        !ids.contains(&"restricted-doc"),
        "restricted hit MUST NOT leak to a public-only principal; ids={ids:?}"
    );
}

/// Phase21 §5.7 — graph proxy: raw Cypher mode must return 403 for any
/// non-admin principal, even when `CORTEX_GRAPH_CYPHER_ENABLED=true`.
///
/// WHERE injection into arbitrary Cypher is not safe; require acl_admin.
#[tokio::test]
async fn graph_proxy_cypher_blocked_for_non_admin() {
    let _lock = ENV_MX.lock().await;
    std::env::set_var("CORTEX_GRAPH_CYPHER_ENABLED", "true");

    // nexus is None (no Nexus URL configured) — the ACL gate fires before
    // the nexus-unconfigured check, so we still get 403, not 503.
    let svc = min_service(PrincipalStore::deny_by_default());
    let app = build_router(svc);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search/graph")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "mode": "cypher",
                        "statement": "MATCH (n) RETURN n LIMIT 1"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "raw Cypher must be 403 for non-admin principals"
    );

    let body = read_json(resp).await;
    assert_eq!(
        body["reason"], "acl_required",
        "response reason must be `acl_required`; body={body}"
    );
}
