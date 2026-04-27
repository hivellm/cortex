//! HTTP-layer integration tests using `axum::Router::oneshot` so we
//! never bind a real port. Drives the spec-11 acceptance criteria
//! that need full request/response shape.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{
    build_router, AclStore, CALLER_HEADER, InMemoryCache, MemoryAuditPublisher,
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryRequest,
    QueryResponse, QueryService, RateConfig, RateLimiter,
};
use serde_json::{json, Value};
use tower::ServiceExt;

type TestHandles = (
    Arc<QueryService>,
    Arc<MemoryVectorLane>,
    Arc<MemoryKeywordLane>,
    Arc<MemoryGraphLane>,
    Arc<MemoryAuditPublisher>,
);

fn build_test_service() -> TestHandles {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let orchestrator = Orchestrator::new(v.clone(), k.clone(), g.clone());
    let audit = Arc::new(MemoryAuditPublisher::new());
    let svc = QueryService {
        orchestrator,
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: audit.clone(),
    };
    (Arc::new(svc), v, k, g, audit)
}

fn body_for(req: &QueryRequest) -> Body {
    Body::from(serde_json::to_vec(req).unwrap())
}

async fn read_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).expect("body json")
}

fn pre_change_request(query: &str, repo: Option<&str>) -> QueryRequest {
    let value = json!({
        "intent": "pre_change_context",
        "scope": { "repo": repo },
        "query": query,
        "include": ["snippets", "decisions", "violations", "graph_neighbors", "similar_turns"],
        "budget_ms": 500,
    });
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn pre_change_context_returns_snippet_within_budget() {
    let (svc, v, _, _, _) = build_test_service();
    v.seed(
        "cortex-vectorizer-code",
        vec![cortex_api::LaneHit {
            doc_id: "doc-1".into(),
            text: "hnsw_search impl".into(),
            repo: Some("Vectorizer".into()),
            path: Some("src/index/hnsw/mod.rs".into()),
            symbol: Some("hnsw_search".into()),
            content_hash: Some("sha256:aaa".into()),
            score: 0.9,
            ts: 100,
            severity: None,
            extras: Default::default(),
        }],
    );
    let req = pre_change_request("ef_search tuning", Some("Vectorizer"));
    let request = Request::builder()
        .method("POST")
        .uri("/v1/query")
        .header("content-type", "application/json")
        .header(CALLER_HEADER, "tester")
        .body(body_for(&req))
        .unwrap();
    let app = build_router(svc);
    let resp = app.oneshot(request).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["intent"], "pre_change_context");
    assert!(!body["results"]["snippets"].as_array().unwrap().is_empty());
    assert_eq!(body["budget"]["cap_ms"], 500);
}

#[tokio::test]
async fn cache_hit_marks_cache_hit_and_skips_lanes() {
    let (svc, v, _, _, _) = build_test_service();
    v.seed(
        "cortex-vectorizer-code",
        vec![cortex_api::LaneHit {
            doc_id: "h".into(),
            text: "hi".into(),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 0.5,
            ts: 0,
            severity: None,
            extras: Default::default(),
        }],
    );
    let app = build_router(svc.clone());
    let req = pre_change_request("same query", None);
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(resp).await;
    assert_eq!(body["budget"]["cache"], "hit");
}

#[tokio::test]
async fn empty_query_returns_400_with_reason() {
    let (svc, _, _, _, _) = build_test_service();
    let app = build_router(svc);
    let req = pre_change_request("   ", None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = read_json(resp).await;
    assert_eq!(body["reason"], "empty_query");
}

#[tokio::test]
async fn acl_denied_returns_403_scope_forbidden() {
    let (svc, _, _, _, _) = build_test_service();
    svc.acl.set_allowed("dash", vec!["OnlyA".into()]);
    let app = build_router(svc);
    let req = pre_change_request("x", Some("ForbiddenRepo"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "dash")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = read_json(resp).await;
    assert_eq!(body["reason"], "scope_forbidden");
}

#[tokio::test]
async fn rate_limited_caller_gets_429_with_retry_after_header() {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let svc = QueryService {
        orchestrator: Orchestrator::new(v, k, g),
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig {
            rps_sustained: 1,
            rps_burst: 1,
        })),
        audit: Arc::new(MemoryAuditPublisher::new()),
    };
    let svc = Arc::new(svc);
    let app = build_router(svc);
    let req = pre_change_request("x", None);
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "burst")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "burst")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key("retry-after"));
}

#[tokio::test]
async fn lane_failure_does_not_block_other_lanes() {
    let v = Arc::new(MemoryVectorLane::new().with_fail());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    // Scope is None below ⇒ orchestrator picks `cortex-unknown-code`.
    let mut keyword_extras = std::collections::BTreeMap::new();
    keyword_extras.insert(
        "source".to_string(),
        serde_json::Value::String("keyword".to_string()),
    );
    k.seed(
        "cortex-unknown-code",
        vec![cortex_api::LaneHit {
            doc_id: "kk".into(),
            text: "keyword hit".into(),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 0.4,
            ts: 1,
            severity: None,
            extras: keyword_extras,
        }],
    );
    let svc = Arc::new(QueryService {
        orchestrator: Orchestrator::new(v, k, g),
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: Arc::new(MemoryAuditPublisher::new()),
    });
    let app = build_router(svc);
    let req = pre_change_request("x", None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let snippets = body["results"]["snippets"].as_array().unwrap();
    assert!(!snippets.is_empty(), "keyword lane still produced results");
    assert!(body["debug"]["errors"]["vector"].is_string());
}

#[tokio::test]
async fn budget_exceeded_truncates_response() {
    let v = Arc::new(MemoryVectorLane::new().with_delay(Duration::from_millis(800)));
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let svc = Arc::new(QueryService {
        orchestrator: Orchestrator::new(v, k, g),
        cache: Arc::new(InMemoryCache::new()),
        acl: Arc::new(AclStore::new()),
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: Arc::new(MemoryAuditPublisher::new()),
    });
    let app = build_router(svc);
    let mut req = pre_change_request("x", None);
    req.budget_ms = 100;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["debug"]["truncated"], true);
}

#[tokio::test]
async fn cache_invalidation_drops_repo_entries() {
    let (svc, _, _, _, _) = build_test_service();
    let req = pre_change_request("hit-once", Some("ToInvalidate"));
    // Warm the cache.
    let _ = svc.handle("c", req.clone()).await;
    // Issue same request — should cache-hit.
    let outcome = svc.handle("c", req.clone()).await;
    let resp: QueryResponse = match outcome {
        cortex_api::ServiceOutcome::Ok(r) => *r,
        other => panic!("expected Ok, got {other:?}"),
    };
    assert_eq!(resp.budget.cache, "hit");
    // Invalidate.
    svc.invalidate_repo("ToInvalidate").await;
    let outcome2 = svc.handle("c", req).await;
    let resp2: QueryResponse = match outcome2 {
        cortex_api::ServiceOutcome::Ok(r) => *r,
        other => panic!("expected Ok, got {other:?}"),
    };
    assert_eq!(resp2.budget.cache, "miss");
}

#[tokio::test]
async fn audit_publisher_emits_one_envelope_per_request() {
    let (svc, _, _, _, audit) = build_test_service();
    let req = pre_change_request("auditable", None);
    let _ = svc.handle("c", req).await;
    let snap = audit.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0]["intent"], "pre_change_context");
}

#[tokio::test]
async fn law_check_returns_only_violations_field() {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let mut extras = cortex_api::Props::new();
    extras.insert("law_id".into(), json!("LAW-007"));
    extras.insert("violation_id".into(), json!("VIO-1"));
    extras.insert("observed_in".into(), json!("turn:01HX"));
    let violation_hit = cortex_api::LaneHit {
        doc_id: "lv".into(),
        text: "violation message".into(),
        repo: None,
        path: None,
        symbol: Some("LAW-007".into()),
        content_hash: None,
        score: 0.0,
        ts: 1,
        severity: Some("critical".into()),
        extras,
    };
    g.seed("law_violations_last_30d", vec![violation_hit.clone()]);
    k.seed("cortex-vectorizer-governance", vec![violation_hit]);
    let svc = Arc::new(QueryService::with_memory_defaults(Orchestrator::new(v, k, g)));
    let app = build_router(svc);
    let req: QueryRequest = serde_json::from_value(json!({
        "intent": "law_check",
        "query": "no skip hooks",
        "include": ["snippets", "decisions", "violations", "graph_neighbors", "similar_turns"],
    }))
    .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(resp).await;
    assert!(body["results"]["snippets"]
        .as_array()
        .map(|v| v.is_empty())
        .unwrap_or(true));
    assert!(body["results"]["decisions"]
        .as_array()
        .map(|v| v.is_empty())
        .unwrap_or(true));
    assert!(!body["results"]["violations"]
        .as_array()
        .map(|v| v.is_empty())
        .unwrap_or(true));
}

#[tokio::test]
async fn redaction_strips_aws_key_from_snippet_text_in_response() {
    let (svc, v, _, _, _) = build_test_service();
    // Scope is None below ⇒ orchestrator picks `cortex-unknown-code`.
    v.seed(
        "cortex-unknown-code",
        vec![cortex_api::LaneHit {
            doc_id: "k".into(),
            text: "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000".into(),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 0.0,
            ts: 0,
            severity: None,
            extras: Default::default(),
        }],
    );
    let app = build_router(svc);
    let req = pre_change_request("any", None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(resp).await;
    let text = body["results"]["snippets"][0]["text"].as_str().unwrap();
    assert!(!text.contains("AKIAIOSFODNN7EXAMPLE0000"));
}

#[tokio::test]
async fn mcp_tool_descriptor_advertises_query_schema() {
    let descriptor = cortex_api::tool_descriptor();
    assert_eq!(descriptor["name"], "cortex_query");
    assert!(descriptor["inputSchema"]["properties"]["intent"].is_object());
    assert!(
        descriptor.get("input_schema").is_none(),
        "snake_case input_schema must not be emitted"
    );
}

#[tokio::test]
async fn mcp_invoke_routes_through_the_same_service() {
    let (svc, v, _, _, _) = build_test_service();
    // mcp_invoke below has no `scope` field ⇒ orchestrator picks `cortex-unknown-code`.
    v.seed(
        "cortex-unknown-code",
        vec![cortex_api::LaneHit {
            doc_id: "x".into(),
            text: "hello".into(),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras: Default::default(),
        }],
    );
    let result = cortex_api::mcp_invoke(
        svc,
        "mcp-host",
        json!({
            "intent": "free_search",
            "query": "hello",
            "include": ["snippets"],
        }),
    )
    .await
    .expect("ok");
    assert_eq!(result.intent, "free_search");
    assert!(!result.results.snippets.is_empty());
}

#[tokio::test]
async fn status_endpoint_returns_service_pid_and_uptime() {
    let (svc, _v, _k, _g, _) = build_test_service();
    let app = build_router(svc);
    let request = Request::builder()
        .method("GET")
        .uri("/v1/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(request).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["service"], "cortex-api");
    assert!(body["pid"].as_u64().unwrap_or(0) > 0);
    assert!(body["uptime_ms"].is_u64());
    assert!(!body["version"].as_str().unwrap_or("").is_empty());
}
