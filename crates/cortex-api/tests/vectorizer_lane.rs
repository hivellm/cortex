//! Integration tests for the live `VectorizerLane` (spec-06 read
//! path). Drives the lane against a `wiremock` Vectorizer double
//! so per-query semantic-search behaviour is provable without a
//! live server — same shape that caught the keyword lane's
//! query-collapse regression.

use std::sync::Arc;

use cortex_api::{
    KeywordLane, KeywordRequest, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, QueryRequest, VectorLane, VectorRequest, VectorizerLane,
};
use cortex_api::types::{IncludeField, Intent, Scope};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn vec_request(query: &str) -> VectorRequest {
    VectorRequest {
        collection: "cortex-cortex-code".into(),
        query: query.into(),
        k: 5,
        scope: Scope::default(),
    }
}

#[tokio::test]
async fn live_lane_passes_query_text_through_to_vectorizer_search() {
    // The lane must forward `req.query` verbatim. The mock matches
    // only when the request body carries `query: "embedder lane"`.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-cortex-code/search/text"))
        .and(body_partial_json(json!({ "query": "embedder lane" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-1",
                "score": 0.91_f32,
                "content": "embedder lane wired",
                "metadata": {
                    "repo": "Cortex",
                    "path": "src/lib.rs",
                    "kind": "turn",
                    "ts": 1714200000000_i64,
                },
            }],
            "query_time_ms": 0.0_f64,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), None).unwrap();
    let hits = lane.search(&vec_request("embedder lane")).await.unwrap();
    assert_eq!(hits.len(), 1, "mock matches only when query is forwarded");
    assert_eq!(hits[0].text, "embedder lane wired");
    assert!((hits[0].score - 0.91).abs() < 1e-5);
    assert_eq!(
        hits[0].extras.get("source").and_then(|v| v.as_str()),
        Some("vector"),
        "live lane stamps the source-attribution invariant",
    );
}

#[tokio::test]
async fn live_lane_returns_empty_when_collection_missing() {
    // Vectorizer surfaces missing collections as 404 / "not found".
    // The lane must turn that into an empty hit set rather than a
    // hard error so the orchestrator's other lanes still flow.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-cortex-code/search/text"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "{\"error\":\"collection not found\"}",
        ))
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), None).unwrap();
    let hits = lane.search(&vec_request("anything")).await.unwrap();
    assert!(
        hits.is_empty(),
        "404 / \"not found\" → empty hit set, not LaneError",
    );
}

#[tokio::test]
async fn distinct_queries_through_orchestrator_produce_distinct_vector_hits() {
    // Orchestrator-level proof. Two queries against the same live
    // Vectorizer double yield different `results.snippets`. Combined
    // with the keyword-lane integration, this completes the dual-lane
    // retrieval the orchestrator was designed for.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-unknown-code/search/text"))
        .and(body_partial_json(json!({ "query": "alpha" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-alpha",
                "score": 0.8_f32,
                "content": "alpha vector",
                "metadata": {},
            }],
            "query_time_ms": 0.0_f64,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-unknown-code/search/text"))
        .and(body_partial_json(json!({ "query": "beta" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-beta",
                "score": 0.7_f32,
                "content": "beta vector",
                "metadata": {},
            }],
            "query_time_ms": 0.0_f64,
        })))
        .mount(&server)
        .await;

    let live = Arc::new(VectorizerLane::new(server.uri(), None).unwrap());
    let keyword: Arc<dyn KeywordLane> = Arc::new(MemoryKeywordLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orch = Orchestrator::new(live, keyword, graph);

    let req_alpha = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "alpha".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
    };
    let req_beta = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "beta".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
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

    assert_eq!(alpha_text, vec!["alpha vector"]);
    assert_eq!(beta_text, vec!["beta vector"]);
    for snip in &resp_alpha.results.snippets {
        assert_eq!(
            snip.source, "vector",
            "vector lane snippets carry source = \"vector\""
        );
    }
    // `debug.lanes.vector_ms` must be Some(_) — the audit's killer
    // symptom was vector_ms = 0 across every probe.
    assert!(
        resp_alpha.debug.lanes.vector_ms.is_some(),
        "vector lane actually ran (the regression had vector_ms = 0)",
    );
}

#[tokio::test]
async fn fail_open_when_vectorizer_unreachable_through_orchestrator() {
    // Spec scenario "Vectorizer down → fail-open": a request to a
    // dead endpoint still returns a populated response — empty
    // results, `debug.errors["vector"]` populated, no panic.
    let dead_lane = Arc::new(
        VectorizerLane::new("http://127.0.0.1:1", None).unwrap(),
    );
    let keyword: Arc<dyn KeywordLane> = Arc::new(MemoryKeywordLane::new());
    let graph = Arc::new(MemoryGraphLane::new());
    let orch = Orchestrator::new(dead_lane, keyword, graph);

    let req = QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: "anything".into(),
        limit: 5,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 1000,
    };
    let (resp, _rewritten) = orch.run(&req).await;
    assert!(resp.results.snippets.is_empty());
    assert!(
        resp.debug.errors.contains_key("vector"),
        "vector lane error must surface in debug.errors",
    );
}

#[tokio::test]
async fn live_lane_keeps_memory_lane_traits_unchanged() {
    // Smoke test that the trait-object swap doesn't change the
    // public API: a `MemoryVectorLane` still implements `VectorLane`
    // and is still usable as the fallback type. The boot wiring
    // depends on this — `Arc<dyn VectorLane>` must accept either
    // implementation.
    let mem = Arc::new(MemoryVectorLane::new());
    let dyn_ref: Arc<dyn VectorLane> = mem.clone();
    let hits = dyn_ref.search(&vec_request("x")).await.unwrap();
    assert!(hits.is_empty(), "empty memory lane returns no hits");
}

#[tokio::test]
async fn keyword_request_compiles_without_changes() {
    // Sanity: this file imports KeywordLane / KeywordRequest only
    // to exercise the orchestrator's three-lane shape — the vector
    // lane's swap in cortex-api/src/main.rs must not have broken
    // the keyword surface. Compiling proves the type signatures
    // still align across modules.
    let _ = KeywordRequest {
        index: "cortex-cortex-code".into(),
        query: "test".into(),
        limit: 5,
        scope: Scope::default(),
    };
}
