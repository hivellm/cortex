//! Integration tests for the live `VectorizerLane` (spec-06 read
//! path). Drives the lane against a `wiremock` Vectorizer double
//! so per-query semantic-search behaviour is provable without a
//! live server — same shape that caught the keyword lane's
//! query-collapse regression.

use std::sync::Arc;

use cortex_api::types::{IncludeField, Intent, Scope};
use cortex_api::{
    KeywordLane, KeywordRequest, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, QueryRequest, VectorLane, VectorRequest, VectorizerLane,
};
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
    // phase11d — wire shape carries `payload`/`vector` (not the
    // SDK's `content`/`metadata`); body lives under `payload.body`.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-cortex-code/search/text"))
        .and(body_partial_json(json!({ "query": "embedder lane" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-1",
                "score": 0.91_f32,
                "vector": [],
                "payload": {
                    "repo": "Cortex",
                    "path": "src/lib.rs",
                    "kind": "turn",
                    "ts": 1714200000000_i64,
                    "body": "embedder lane wired",
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
    assert_eq!(hits[0].path.as_deref(), Some("src/lib.rs"));
    assert!((hits[0].score - 0.91).abs() < 1e-5);
    assert_eq!(
        hits[0].extras.get("source").and_then(|v| v.as_str()),
        Some("vector"),
        "live lane stamps the source-attribution invariant",
    );
}

#[tokio::test]
async fn live_lane_drops_text_when_payload_omitted() {
    // phase11d regression guard — when the upstream wire response
    // skips `payload` entirely (the case that bit phase11a's e2e
    // test), the lane must NOT silently project an empty hit; it
    // produces a `LaneHit` with empty text/path so the bundle
    // renderer collapses to a header-only line per phase10b §1.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-cortex-code/search/text"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-empty",
                "score": 0.5_f32,
            }],
            "query_time_ms": 0.0_f64,
        })))
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), None).unwrap();
    let hits = lane.search(&vec_request("anything")).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "", "no body in payload → empty text");
    assert!(hits[0].path.is_none());
}

#[tokio::test]
async fn live_lane_rejects_legacy_sdk_shape_as_empty_hits() {
    // phase11d — the regression: if the server (or the SDK) ever
    // re-emits the legacy `{content, metadata}` shape, the new
    // wire deserializer treats both as unknown fields, leaving
    // `payload = {}`. The lane must return hits with empty
    // text/path rather than half-projected `LaneHit`s. This pins
    // the failure mode so a future upstream regression bubbles up
    // immediately.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collections/cortex-cortex-code/search/text"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "vec-legacy",
                "score": 0.7_f32,
                "content": "would have been the body",
                "metadata": { "path": "src/x.rs" },
            }],
            "query_time_ms": 0.0_f64,
        })))
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), None).unwrap();
    let hits = lane.search(&vec_request("anything")).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].text, "",
        "legacy shape's `content` must NOT be picked up by the new projection",
    );
    assert!(
        hits[0].path.is_none(),
        "legacy shape's `metadata.path` must NOT be picked up either",
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
        .respond_with(
            ResponseTemplate::new(404).set_body_string("{\"error\":\"collection not found\"}"),
        )
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
                "vector": [],
                "payload": { "body": "alpha vector" },
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
                "vector": [],
                "payload": { "body": "beta vector" },
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
        budget_bytes: None,
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
    let dead_lane = Arc::new(VectorizerLane::new("http://127.0.0.1:1", None).unwrap());
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
        budget_bytes: None,
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

// ────────────────────────────────────────────────────────────────────
// phase11a — VectorizerLane::probe_authenticated tests
// ────────────────────────────────────────────────────────────────────

/// Helper: minimal `/auth/login` mock returning a three-segment JWT
/// (the SDK sniffs that shape and sends `Authorization: Bearer`).
async fn mount_login_ok(server: &MockServer, token: &str) {
    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": token,
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn probe_authenticated_succeeds_when_list_collections_returns_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/collections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "collections": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), Some("static-key".into())).unwrap();
    lane.probe_authenticated()
        .await
        .expect("probe_authenticated must accept a 200 from /collections");
}

#[tokio::test]
async fn probe_authenticated_surfaces_401_when_no_creds_cached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/collections"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "unauthorized",
            "message": "Authentication required",
        })))
        .mount(&server)
        .await;

    let lane = VectorizerLane::new(server.uri(), None).unwrap();
    let err = lane
        .probe_authenticated()
        .await
        .expect_err("401 with no cached creds must surface");
    assert!(
        err.contains("no cached credentials"),
        "actionable hint must point at the env vars: {err}",
    );
    assert!(
        err.contains("CORTEX_VECTORIZER_USER"),
        "error names the env keys to set: {err}",
    );
}

#[tokio::test]
async fn probe_authenticated_refreshes_jwt_on_401_and_retries() {
    // First /collections call (using the boot-time JWT) returns 401.
    // The lane re-mints the JWT via /auth/login, then retries
    // /collections once and the second call returns 200.
    let server = MockServer::start().await;
    let segment = "eyJhbGciOiJIUzI1NiJ9";
    let initial_jwt = format!("{segment}.{segment}.initial");
    let refreshed_jwt = format!("{segment}.{segment}.refreshed");

    // /auth/login responds with the refreshed JWT.
    mount_login_ok(&server, &refreshed_jwt).await;

    // First /collections — only the initial JWT should reach this
    // mock. Returns 401.
    Mock::given(method("GET"))
        .and(path("/collections"))
        .and(wiremock::matchers::header(
            "authorization",
            format!("Bearer {initial_jwt}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "unauthorized",
        })))
        .expect(1)
        .mount(&server)
        .await;
    // Retry /collections — only the refreshed JWT should reach this
    // mock. Returns 200.
    Mock::given(method("GET"))
        .and(path("/collections"))
        .and(wiremock::matchers::header(
            "authorization",
            format!("Bearer {refreshed_jwt}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "collections": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Bootstrap: stand up the lane with a hand-crafted "initial" JWT
    // so we can drive the refresh path deterministically. We can't
    // use `with_login` here because it would consume one /auth/login
    // call before our 401 fires; the refresh path needs the only
    // /auth/login mock to still be untouched when probe_authenticated
    // runs. Building via `new(api_key=initial_jwt)` and patching the
    // creds in is the cleanest way to set both up.
    let lane =
        VectorizerLane::with_initial_jwt_for_test(server.uri(), &initial_jwt, "admin", "secret");
    lane.probe_authenticated()
        .await
        .expect("refresh-and-retry path returns Ok when retry succeeds");
}

#[tokio::test]
async fn probe_authenticated_surfaces_persistent_401_after_refresh() {
    let server = MockServer::start().await;
    let segment = "eyJhbGciOiJIUzI1NiJ9";
    let initial_jwt = format!("{segment}.{segment}.initial");
    let refreshed_jwt = format!("{segment}.{segment}.refreshed");

    mount_login_ok(&server, &refreshed_jwt).await;

    // Both attempts return 401 regardless of which token is sent —
    // simulates wrong credentials (the upstream rejects both the
    // initial and the post-refresh token).
    Mock::given(method("GET"))
        .and(path("/collections"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "unauthorized",
        })))
        .mount(&server)
        .await;

    let lane = VectorizerLane::with_initial_jwt_for_test(
        server.uri(),
        &initial_jwt,
        "admin",
        "wrong-password",
    );
    let err = lane
        .probe_authenticated()
        .await
        .expect_err("persistent 401 after refresh must surface");
    assert!(
        err.contains("refresh-retry failed"),
        "error must distinguish refresh-then-still-401 from initial 401: {err}",
    );
}
