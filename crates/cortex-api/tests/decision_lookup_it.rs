//! Phase11k §5.1 — `decision_lookup` acceptance integration test.
//!
//! Seeds the keyword lane with what the spec-08 fulltext-worker
//! writes after phase11k §1 (top-level `decision_id` /
//! `decision_title` / `decision_status` projection) and fires
//! `POST /v1/query` with `intent = "decision_lookup"`. Asserts that
//! `results.decisions[]` is non-empty and the row's `decision_id`
//! matches the seeded ADR slug.
//!
//! The live-stack form (full bootstrap → Meili → query) is gated
//! behind `CORTEX_GOVERNANCE_IT=1`. The default cargo test path
//! runs the in-process variant against the `MemoryKeywordLane`
//! double, which exercises the same orchestrator code path that
//! `derive_decisions` reads — `LANE_EXTRAS_KEYS` projection holds
//! identically whether the source is a real Meili search or the
//! seeded double.

use std::env;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, LaneHit, MemoryAuditPublisher,
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryRequest, QueryService,
    RateConfig, RateLimiter, CALLER_HEADER,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn decision_lookup_request(query: &str, repo: Option<&str>) -> QueryRequest {
    let value = json!({
        "intent": "decision_lookup",
        "scope": { "repo": repo },
        "query": query,
        "include": ["snippets", "decisions"],
        "budget_ms": 500,
    });
    serde_json::from_value(value).unwrap()
}

fn build_service() -> (Arc<QueryService>, Arc<MemoryKeywordLane>) {
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
        audit,
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,
    };
    (Arc::new(svc), k)
}

fn lane_hit_for_decision(decision_id: &str, title: &str, status: &str, repo: &str) -> LaneHit {
    let mut extras = std::collections::BTreeMap::new();
    // The lane projection contract reads decision_id / decision_title
    // / decision_status / supersedes out of `extras`. Phase11k §1
    // stamps these at the top of the Meili document; the live
    // `MeiliKeywordLane.project` flattens them through `extras_raw`
    // into `extras` keyed by the same names.
    extras.insert("source".into(), json!("keyword"));
    extras.insert("decision_id".into(), json!(decision_id));
    extras.insert("decision_title".into(), json!(title));
    extras.insert("decision_status".into(), json!(status));
    LaneHit {
        doc_id: format!("meili|cortex_decisions|{decision_id}"),
        text: format!("Body of {decision_id}"),
        repo: Some(repo.to_string()),
        path: Some(format!(".rulebook/decisions/{decision_id}.md")),
        symbol: Some("decision".into()),
        content_hash: None,
        score: 0.9,
        ts: 1714200000000,
        severity: None,
        extras,
        overlay: Default::default(),
    }
}

#[tokio::test]
async fn decision_lookup_returns_populated_decisions_overlay() {
    if env::var("CORTEX_GOVERNANCE_IT").ok().as_deref() == Some("1") {
        eprintln!("CORTEX_GOVERNANCE_IT=1 — live-stack path not implemented in this IT");
        return;
    }

    let (svc, kw) = build_service();
    // Seed the per-repo lane (the canonical write target post-phase11k
    // §1) and the global lane (the §2 dual-write target). Both are
    // wired into the `decision_lookup` plan in
    // `crates/cortex-api/src/strategies.rs`.
    kw.seed(
        "cortex-cortex-decisions",
        vec![lane_hit_for_decision(
            "ADR-0042",
            "Adopt Meili over Lexum",
            "accepted",
            "Cortex",
        )],
    );
    kw.seed(
        "cortex_decisions",
        vec![lane_hit_for_decision(
            "ADR-0042",
            "Adopt Meili over Lexum",
            "accepted",
            "Cortex",
        )],
    );

    let app = build_router(svc);
    let req = decision_lookup_request("adopt meili", Some("Cortex"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/query")
                .header("content-type", "application/json")
                .header(CALLER_HEADER, "phase11k-it")
                .body(Body::from(serde_json::to_vec(&req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let decisions = body["results"]["decisions"]
        .as_array()
        .expect("results.decisions is an array");
    assert!(
        !decisions.is_empty(),
        "results.decisions must be non-empty after phase11k projection; body={body}",
    );
    let row = &decisions[0];
    assert_eq!(row["id"], "ADR-0042");
    assert_eq!(row["status"], "accepted");
    assert_eq!(row["title"], "Adopt Meili over Lexum");
}
