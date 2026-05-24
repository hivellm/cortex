//! Phase11k §5.3 — global governance lane integration test.
//!
//! Fires `decision_lookup` and asserts hits land from at least two
//! different repos, surfaced through the global `cortex_decisions`
//! lane introduced by phase11k §2 (workers dual-write per-repo +
//! global). Demonstrates that a cross-repo decision lookup answers
//! "have we ever decided X?" without forcing the caller to enumerate
//! every repo in `scope.repo`.
//!
//! Live-stack form gated behind `CORTEX_GOVERNANCE_IT=1`.

use std::env;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use cortex_api::{
    build_router, AclStore, AuditStore, InMemoryCache, LaneHit, MemoryAuditPublisher,
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryRequest, QueryService,
    RateConfig, RateLimiter, CALLER_HEADER,
};
use cortex_storage::names::INDEX_DECISIONS;
use serde_json::{json, Value};
use tower::ServiceExt;

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

fn decision_lane_hit(decision_id: &str, title: &str, repo: &str) -> LaneHit {
    let mut extras = std::collections::BTreeMap::new();
    extras.insert("source".into(), json!("keyword"));
    extras.insert("decision_id".into(), json!(decision_id));
    extras.insert("decision_title".into(), json!(title));
    extras.insert("decision_status".into(), json!("accepted"));
    LaneHit {
        doc_id: format!("meili|{INDEX_DECISIONS}|{decision_id}"),
        text: format!("ADR body for {decision_id}"),
        repo: Some(repo.to_string()),
        path: Some(format!(".rulebook/decisions/{decision_id}.md")),
        symbol: Some("decision".into()),
        content_hash: None,
        score: 0.85,
        ts: 1714200000000,
        severity: None,
        extras,
        overlay: Default::default(),
    }
}

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

#[tokio::test]
async fn cross_repo_decision_lookup_surfaces_hits_from_two_repos_via_global_lane() {
    if env::var("CORTEX_GOVERNANCE_IT").ok().as_deref() == Some("1") {
        eprintln!("CORTEX_GOVERNANCE_IT=1 — live-stack path not implemented in this IT");
        return;
    }

    let (svc, kw) = build_service();
    // Phase11k §2 — workers dual-write per-repo + global. The global
    // `cortex_decisions` index carries ADRs from EVERY repo; the
    // strategy in `cortex-api/src/strategies.rs::decision_lookup`
    // fans out to the global index AND each per-repo index. A
    // caller asking with `scope.repo = Some("Cortex")` still hits
    // the global lane and discovers ADRs from Vectorizer too.
    kw.seed(
        INDEX_DECISIONS,
        vec![
            decision_lane_hit("ADR-A-1", "Adopt Meili", "Cortex"),
            decision_lane_hit("ADR-B-1", "Adopt HNSW", "Vectorizer"),
        ],
    );

    let app = build_router(svc);
    let req = decision_lookup_request("adopt", Some("Cortex"));
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
    let snippets = body["results"]["snippets"]
        .as_array()
        .expect("snippets is an array");

    let mut repos: Vec<String> = snippets
        .iter()
        .filter_map(|s| s["repo"].as_str().map(str::to_string))
        .collect();
    repos.sort();
    repos.dedup();
    assert!(
        repos.len() >= 2,
        "global lane must surface hits from at least 2 repos; got {repos:?} body={body}",
    );

    let decisions = body["results"]["decisions"]
        .as_array()
        .expect("results.decisions is an array");
    let mut adr_ids: Vec<String> = decisions
        .iter()
        .filter_map(|d| d["id"].as_str().map(str::to_string))
        .collect();
    adr_ids.sort();
    assert_eq!(
        adr_ids,
        vec!["ADR-A-1".to_string(), "ADR-B-1".to_string()],
        "decisions overlay must list ADRs from both repos",
    );
}
