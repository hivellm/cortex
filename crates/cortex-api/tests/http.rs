//! HTTP-layer integration tests using `axum::Router::oneshot` so we
//! never bind a real port. Drives the spec-11 acceptance criteria
//! that need full request/response shape.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cortex_api::{
    build_router, AclStore, CALLER_HEADER, InMemoryCache, LaneHit, MemoryAuditPublisher,
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
    build_test_service_with_indexed_repos(None)
}

fn build_test_service_with_indexed_repos(
    lane: impl Into<Option<Arc<MemoryKeywordLane>>>,
) -> TestHandles {
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
        indexed_repos: lane.into(),
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
    let req = pre_change_request("same query", Some("vectorizer"));
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
    let req = pre_change_request("   ", Some("vectorizer"));
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
        indexed_repos: None,
    };
    let svc = Arc::new(svc);
    let app = build_router(svc);
    let req = pre_change_request("x", Some("vectorizer"));
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
    // Scope = "vectorizer" ⇒ orchestrator hits `cortex-vectorizer-{family}`.
    let mut keyword_extras = std::collections::BTreeMap::new();
    keyword_extras.insert(
        "source".to_string(),
        serde_json::Value::String("keyword".to_string()),
    );
    k.seed(
        "cortex-vectorizer-code",
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
        indexed_repos: None,
    });
    let app = build_router(svc);
    let req = pre_change_request("x", Some("vectorizer"));
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
        indexed_repos: None,
    });
    let app = build_router(svc);
    let mut req = pre_change_request("x", Some("vectorizer"));
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
    let req = pre_change_request("auditable", Some("vectorizer"));
    let _ = svc.handle("c", req).await;
    let snap = audit.snapshot();
    assert_eq!(snap.len(), 1);
    let env = &snap[0];
    assert_eq!(env["intent"], "pre_change_context");
    // Phase6a — `scope_resolution` lands on every envelope so the
    // dashboard can flag misconfigured callers.
    assert!(
        env.get("scope_resolution").is_some(),
        "scope_resolution missing from audit envelope: {env}"
    );
    // Phase6c — fusion-tuning fields land on every envelope so
    // the harness in phase6e can attribute relevance regressions
    // to fusion-config changes.
    let alpha = env
        .get("fusion_alpha")
        .and_then(|v| v.as_f64())
        .expect("audit envelope must carry fusion_alpha");
    let k = env
        .get("fusion_k")
        .and_then(|v| v.as_u64())
        .expect("audit envelope must carry fusion_k");
    // Default config is alpha=0.7, k=60 (DEFAULT_RRF_ALPHA, RRF_K).
    assert!(
        (alpha - 0.7_f64).abs() < 1e-5,
        "expected default fusion_alpha 0.7, got {alpha}"
    );
    assert_eq!(k, 60, "expected default fusion_k=60, got {k}");
}

#[tokio::test]
async fn law_check_returns_only_violations_field() {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    let mut extras = cortex_api::Props::new();
    extras.insert("source".into(), json!("keyword"));
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
        "scope": { "repo": "vectorizer" },
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
    // Scope = "vectorizer" ⇒ orchestrator hits `cortex-vectorizer-{family}`.
    v.seed(
        "cortex-vectorizer-code",
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
    let req = pre_change_request("any", Some("vectorizer"));
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

// ---- phase6a — `/v1/query` HTTP-layer scope resolution ----

#[tokio::test]
async fn missing_scope_repo_returns_422_scope_repo_required() {
    // F-003 — the resolver must reject when no lane fires.
    std::env::remove_var("CORTEX_ALLOW_UNKNOWN_SCOPE");
    let (svc, _, _, _, _) = build_test_service();
    let app = build_router(svc);
    let req = pre_change_request("anything", None);
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
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = read_json(resp).await;
    assert_eq!(body["reason"], "scope_repo_required");
}

#[tokio::test]
async fn x_cortex_cwd_header_resolves_repo_from_basename() {
    // The MCP server / dashboard inject `x-cortex-cwd`; the daemon
    // slugifies the basename and runs the request with that scope.
    std::env::remove_var("CORTEX_ALLOW_UNKNOWN_SCOPE");
    let (svc, v, _, _, _) = build_test_service();
    v.seed(
        "cortex-vectorizer-code",
        vec![cortex_api::LaneHit {
            doc_id: "h".into(),
            text: "ef_search".into(),
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
    let app = build_router(svc);
    let req = pre_change_request("ef_search", None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .header("x-cortex-cwd", "/home/user/work/Vectorizer")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["scope_resolved"]["repo"], "vectorizer");
    assert!(!body["results"]["snippets"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn x_cortex_repo_header_resolves_when_body_omits_scope() {
    std::env::remove_var("CORTEX_ALLOW_UNKNOWN_SCOPE");
    let (svc, v, _, _, _) = build_test_service();
    v.seed(
        "cortex-cortex-code",
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
    let app = build_router(svc);
    let req = pre_change_request("hi", None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .header("x-cortex-repo", "Cortex")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    assert_eq!(body["scope_resolved"]["repo"], "cortex");
}

#[tokio::test]
async fn audit_envelope_records_scope_resolution_lane() {
    // The audit trail must show how scope was derived — this is the
    // signal the dashboard's `query_audit` view uses to flag
    // misconfigured callers and the harness uses to prove F-003 is
    // closed.
    std::env::remove_var("CORTEX_ALLOW_UNKNOWN_SCOPE");
    let (svc, v, _, _, audit) = build_test_service();
    v.seed(
        "cortex-vectorizer-code",
        vec![cortex_api::LaneHit {
            doc_id: "h".into(),
            text: "x".into(),
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
    let app = build_router(svc);
    let req = pre_change_request("x", None);
    let _ = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "c")
                .header("x-cortex-cwd", "/tmp/Vectorizer")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    let envelopes = audit.snapshot();
    assert!(
        !envelopes.is_empty(),
        "every successful request must publish exactly one envelope"
    );
    let last = envelopes.last().unwrap();
    assert_eq!(
        last["scope_resolution"], "cwd",
        "envelope must record the resolution lane that fired"
    );
}

#[tokio::test]
async fn legacy_unknown_scope_hatch_passes_through_when_env_is_set() {
    // The deprecation hatch keeps today's behaviour for one window;
    // when set, a missing scope falls through with `scope_resolution = rejected_legacy`
    // and the request still reaches the orchestrator.
    std::env::set_var("CORTEX_ALLOW_UNKNOWN_SCOPE", "1");
    let (svc, _, _, _, _) = build_test_service();
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
    std::env::remove_var("CORTEX_ALLOW_UNKNOWN_SCOPE");
    // The body lacks scope.repo and no lane fires, but the hatch
    // rescues the request — the daemon answers OK with an empty
    // result rather than 422.
    assert_eq!(resp.status(), StatusCode::OK);
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
    assert!(
        body["indexed_repos"].is_array(),
        "status must always carry an indexed_repos array (issue hivellm/cortex#1)",
    );
}

#[tokio::test]
async fn status_indexed_repos_reports_seeded_repo_slugs() {
    // Seed a snapshot lane with mixed-casing repo values — the
    // status lookup must canonicalise through `slug_for_repo` so the
    // emitted list matches the form `notice.repo_not_indexed` checks
    // against.
    let lane = Arc::new(MemoryKeywordLane::new());
    lane.seed(
        "cortex-code",
        vec![
            LaneHit {
                doc_id: "h1".into(),
                text: "indexed".into(),
                repo: Some("Cortex".into()),
                path: None,
                symbol: None,
                content_hash: None,
                score: 0.9,
                ts: 0,
                severity: None,
                extras: Default::default(),
            },
            LaneHit {
                doc_id: "h2".into(),
                text: "indexed".into(),
                repo: Some("Vectorizer".into()),
                path: None,
                symbol: None,
                content_hash: None,
                score: 0.5,
                ts: 0,
                severity: None,
                extras: Default::default(),
            },
        ],
    );
    let (svc, _v, _k, _g, _) = build_test_service_with_indexed_repos(Some(lane));
    let app = build_router(svc);
    let request = Request::builder()
        .method("GET")
        .uri("/v1/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(request).await.unwrap();
    let body = read_json(resp).await;
    let repos: Vec<String> = body["indexed_repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(repos, vec!["cortex".to_string(), "vectorizer".to_string()]);
}

// ----------------------------------------------------------------
// Phase6b §5 — overlay end-to-end tests.
//
// Pre-phase6b, the keyword + vector lanes never stamped the
// projection-contract keys (`decision_id`, `turn_id`, …) into
// `LaneHit.extras`, so `derive_decisions` / `derive_similar_turns`
// produced empty arrays in production even when the matching rows
// existed upstream. These tests pin the post-fix behaviour: when
// the seed carries the contract keys, the corresponding overlay
// surfaces them on the response body.
// ----------------------------------------------------------------

fn decision_request(query: &str, repo: Option<&str>) -> QueryRequest {
    let value = json!({
        "intent": "decision_lookup",
        "scope": { "repo": repo },
        "query": query,
        "include": ["decisions"],
        "budget_ms": 500,
    });
    serde_json::from_value(value).unwrap()
}

fn similar_problems_request(query: &str, repo: Option<&str>) -> QueryRequest {
    let value = json!({
        "intent": "similar_problems",
        "scope": { "repo": repo },
        "query": query,
        "include": ["similar_turns"],
        "budget_ms": 500,
    });
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn decision_overlay_surfaces_decision_id_from_extras() {
    let (svc, _, k, _, _) = build_test_service();
    let mut extras = std::collections::BTreeMap::new();
    extras.insert(
        "source".to_string(),
        Value::String("keyword".to_string()),
    );
    extras.insert(
        "decision_id".to_string(),
        Value::String("DEC-0042".to_string()),
    );
    extras.insert(
        "decision_status".to_string(),
        Value::String("accepted".to_string()),
    );
    k.seed(
        "cortex-cortex-decisions",
        vec![LaneHit {
            doc_id: "dec-1".into(),
            text: "Adopt CLAUDE_CONFIG_DIR for classifier subprocess isolation".into(),
            repo: Some("Cortex".into()),
            path: Some("decisions/0042-classifier.md".into()),
            symbol: Some("DEC-0042 Classifier subprocess hookless config".into()),
            content_hash: Some("sha256:dec0042".into()),
            score: 0.71,
            ts: 1_777_400_000,
            severity: None,
            extras,
        }],
    );
    let app = build_router(svc);
    let req = decision_request("classifier subprocess hooks", Some("cortex"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "phase6b-decision")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let decisions = body["results"]["decisions"]
        .as_array()
        .expect("decisions array on response");
    assert!(
        !decisions.is_empty(),
        "expected at least one decision overlay row when the keyword lane carries decision_id; \
         got empty array (the phase6b regression that this test guards against)"
    );
    assert_eq!(decisions[0]["id"], "DEC-0042");
    assert_eq!(decisions[0]["status"], "accepted");
}

#[tokio::test]
async fn similar_turns_overlay_surfaces_turn_id_from_extras() {
    let (svc, v, _, _, _) = build_test_service();
    let mut extras = std::collections::BTreeMap::new();
    extras.insert("source".to_string(), Value::String("vector".to_string()));
    extras.insert(
        "turn_id".to_string(),
        Value::String("01HTURNFIXTURE0000000000XX".to_string()),
    );
    extras.insert(
        "model".to_string(),
        Value::String("claude-sonnet-4-6".to_string()),
    );
    extras.insert(
        "summary".to_string(),
        Value::String("planned the phase6b lane projection contract".into()),
    );
    v.seed(
        "cortex-cortex-turns",
        vec![LaneHit {
            doc_id: "turn-1".into(),
            text: "Discussed lane projection contract for cortex-api".into(),
            repo: Some("Cortex".into()),
            path: None,
            symbol: None,
            content_hash: Some("sha256:turn0001".into()),
            score: 0.83,
            ts: 1_777_400_000,
            severity: None,
            extras,
        }],
    );
    let app = build_router(svc);
    let req = similar_problems_request("phase6b lane projection", Some("cortex"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "phase6b-similar")
                .body(body_for(&req))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_json(resp).await;
    let turns = body["results"]["similar_turns"]
        .as_array()
        .expect("similar_turns array on response");
    assert!(
        !turns.is_empty(),
        "expected at least one similar_turns overlay row when the vector lane carries turn_id; \
         got empty array (the phase6b regression that this test guards against)"
    );
    assert_eq!(turns[0]["turn_id"], "01HTURNFIXTURE0000000000XX");
    assert_eq!(turns[0]["model"], "claude-sonnet-4-6");
    assert_eq!(
        turns[0]["summary"],
        "planned the phase6b lane projection contract"
    );
}
