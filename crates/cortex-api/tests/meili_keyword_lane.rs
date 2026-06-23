//! Integration tests for the live `MeiliKeywordLane` (spec-08 read
//! path). Drives the lane against a `wiremock` Meili double so the
//! per-query filter behaviour is provable without a live server —
//! the same shape the 2026-04-27 audit caught the test double
//! collapsing.

use std::sync::Arc;

use cortex_api::types::{IncludeField, Intent, Scope};
use cortex_api::{
    KeywordLane, KeywordRequest, MeiliKeywordLane, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryRequest,
};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn keyword_request(query: &str) -> KeywordRequest {
    KeywordRequest {
        index: "cortex-cortex-code".into(),
        query: query.into(),
        limit: 5,
        scope: Scope::default(),
        acl: None,
    }
}

#[tokio::test]
async fn live_lane_passes_query_string_through_to_meili() {
    // The whole point of this lane: `req.query` must reach Meili.
    // The MemoryKeywordLane double ignored it; the live lane must
    // not. We verify by registering a mock that only matches when
    // the request body carries `q: "embedder lane"` and asserting
    // the lane's response carries the corresponding hit.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .and(body_partial_json(json!({ "q": "embedder lane" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [{
                "id": "evt-1",
                "event_id": "evt-1",
                "kind": "turn",
                "repo": "Cortex",
                "path": "src/lib.rs",
                "summary": "embedder lane wired",
                "ts": 1714200000000_i64,
                "_rankingScore": 0.91,
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let hits = lane
        .search(&keyword_request("embedder lane"))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "mock only matches when q is forwarded");
    assert_eq!(hits[0].text, "embedder lane wired");
    assert_eq!(
        hits[0].extras.get("source").and_then(|v| v.as_str()),
        Some("keyword"),
        "live lane stamps the source-attribution invariant",
    );
}

#[tokio::test]
async fn live_lane_returns_empty_on_404_lazy_index() {
    // Per-project Meili indexes (`cortex-{slug}-{family}`) are
    // materialised lazily by the spec-08 worker on first upsert.
    // A 404 is the legitimate empty-index case — the lane must
    // return zero hits, not a hard error.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let hits = lane.search(&keyword_request("anything")).await.unwrap();
    assert!(hits.is_empty(), "404 → empty hit set, never an error");
}

#[tokio::test]
async fn live_lane_surfaces_5xx_as_lane_error_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .respond_with(ResponseTemplate::new(503).set_body_string("backpressure"))
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let err = lane.search(&keyword_request("query")).await.unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("503") && msg.contains("backpressure"),
        "lane bubbles status + body so debug.errors can spot the failure: {msg}"
    );
}

#[tokio::test]
async fn distinct_queries_through_orchestrator_return_distinct_snippets() {
    // The orchestrator-level proof. Two `/v1/query`-shaped requests
    // with different `query` strings against the same Meili double
    // must yield different `results.snippets` — the failure mode
    // the audit caught (every prompt → same five smoke envelopes).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-unknown-code/search"))
        .and(body_partial_json(json!({ "q": "alpha" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [{
                "id": "evt-alpha",
                "event_id": "evt-alpha",
                "kind": "turn",
                "summary": "alpha hit",
                "ts": 1_i64,
                "_rankingScore": 0.5,
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-unknown-code/search"))
        .and(body_partial_json(json!({ "q": "beta" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [{
                "id": "evt-beta",
                "event_id": "evt-beta",
                "kind": "turn",
                "summary": "beta hit",
                "ts": 2_i64,
                "_rankingScore": 0.5,
            }]
        })))
        .mount(&server)
        .await;

    let live = Arc::new(MeiliKeywordLane::new(server.uri(), None).unwrap());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orch = Orchestrator::new(vector, live, graph);

    let req_alpha = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "alpha".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,

        principal: None,
    };
    let req_beta = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "beta".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,

        principal: None,
    };

    let (resp_alpha, _) = orch.run(&req_alpha).await;
    let (resp_beta, _) = orch.run(&req_beta).await;

    let alpha_text: Vec<_> = resp_alpha
        .results
        .snippets
        .iter()
        .map(|s| s.text.clone())
        .collect();
    let beta_text: Vec<_> = resp_beta
        .results
        .snippets
        .iter()
        .map(|s| s.text.clone())
        .collect();

    assert_eq!(alpha_text, vec!["alpha hit"]);
    assert_eq!(beta_text, vec!["beta hit"]);
    assert_ne!(
        alpha_text, beta_text,
        "live keyword lane must filter by query — \
         the regression is two prompts returning the same snippets",
    );
    // Source-attribution invariant — every snippet from a keyword
    // lane carries `source: "keyword"`. The previous test double
    // left the field empty and the orchestrator's lane_label()
    // defaulted to "vector".
    for snip in &resp_alpha.results.snippets {
        assert_eq!(snip.source, "keyword");
    }
    for snip in &resp_beta.results.snippets {
        assert_eq!(snip.source, "keyword");
    }
}

#[tokio::test]
async fn fail_open_when_meili_unreachable_through_orchestrator() {
    // Spec scenario "Meili down → fail-open": a request to a dead
    // server still returns a populated response — empty results,
    // `debug.errors["keyword"]` populated, no panic.
    let dead_lane = Arc::new(MeiliKeywordLane::new("http://127.0.0.1:1", None).unwrap());
    let _unused: Arc<MemoryKeywordLane> = Arc::new(MemoryKeywordLane::new());
    let vector = Arc::new(MemoryVectorLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orch = Orchestrator::new(vector, dead_lane, graph);

    let req = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "anything".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,

        principal: None,
    };
    let (resp, _rewritten) = orch.run(&req).await;
    assert!(
        resp.results.snippets.is_empty(),
        "no live data + no other lane = empty results"
    );
    assert!(
        resp.debug.errors.contains_key("keyword"),
        "keyword lane error must surface in debug.errors"
    );
}

// ── Phase21 §5.2 — ACL filter injection tests ──────────────────────────────

/// The lane must include the Bell-LaPadula ACL clause in the Meili filter when
/// `req.acl` is set. We verify by registering a mock that only matches when the
/// request body carries the expected filter string.
#[tokio::test]
async fn acl_context_injects_level_and_compartment_filter_into_meili_body() {
    let server = MockServer::start().await;
    // The mock matches only when the request body contains the expected
    // ACL filter clause (class_level <= 2 + compartment allowlist).
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .and(body_partial_json(json!({
            "filter": "(class_level IS EMPTY OR class_level <= 2) AND (class_compartments IS EMPTY OR class_compartments IN ['financial', 'hr'])"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [{
                "id": "acl-hit-1",
                "event_id": "acl-hit-1",
                "kind": "turn",
                "repo": "Cortex",
                "summary": "classified internal doc",
                "_rankingScore": 0.85,
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let mut req = keyword_request("classify");
    req.acl = Some(cortex_api::AclContext {
        clearance_level: 2,
        compartment_grants: vec!["financial".to_string(), "hr".to_string()],
    });
    let hits = lane.search(&req).await.unwrap();
    assert_eq!(hits.len(), 1, "mock only matches when ACL filter is injected");
    assert_eq!(hits[0].text, "classified internal doc");
}

/// A wildcard grant (`"*"`) must emit only the level clause — no compartment
/// list — so super-admin callers do not generate an unnecessarily verbose filter.
#[tokio::test]
async fn acl_wildcard_grant_emits_level_only_filter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .and(body_partial_json(json!({
            "filter": "(class_level IS EMPTY OR class_level <= 3)"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "hits": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let mut req = keyword_request("wildcard");
    req.acl = Some(cortex_api::AclContext {
        clearance_level: 3,
        compartment_grants: vec!["*".to_string()],
    });
    let hits = lane.search(&req).await.unwrap();
    assert!(hits.is_empty());
}

/// When `req.acl` is `None` (backward-compat), the lane must NOT inject any
/// ACL clause — the filter should only contain scope clauses (or be absent).
#[tokio::test]
async fn no_acl_context_produces_no_acl_filter() {
    let server = MockServer::start().await;
    // The mock matches any POST body WITHOUT the class_level filter.
    // We verify via wiremock's negation: mount a mock that DOES match the
    // ACL filter and assert it was NEVER called (0 invocations).
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .and(body_partial_json(json!({ "filter": "(class_level IS EMPTY OR class_level <= 3)" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "hits": [] })))
        .expect(0) // must NOT be called
        .mount(&server)
        .await;
    // Fallback mock — matches any POST to the index.
    Mock::given(method("POST"))
        .and(path("/indexes/cortex-cortex-code/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "hits": [] })))
        .mount(&server)
        .await;

    let lane = MeiliKeywordLane::new(server.uri(), None).unwrap();
    let req = keyword_request("no-acl"); // acl: None by default
    let _ = lane.search(&req).await.unwrap();
}
