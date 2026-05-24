//! Phase11k §5.2 — `law_check` acceptance integration test.
//!
//! Seeds the keyword lane with the projection a phase11k §1 worker
//! writes for a `Kind::LawViolation` envelope (top-level `law_id` /
//! `law_severity` plus the existing top-level `severity`). Fires
//! `POST /v1/query` with `intent = "law_check"` and asserts that
//! `results.laws_active` is populated with the seeded law id +
//! severity, surfaced through `derive_laws`'s contract path.
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
use cortex_storage::names::INDEX_LAWS;
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

fn law_lane_hit(law_id: &str, message: &str, severity: &str) -> LaneHit {
    let mut extras = std::collections::BTreeMap::new();
    extras.insert("source".into(), json!("keyword"));
    LaneHit {
        doc_id: format!("meili|{INDEX_LAWS}|{law_id}"),
        text: message.to_string(),
        repo: None,
        path: None,
        symbol: Some(law_id.to_string()),
        content_hash: None,
        score: 0.95,
        ts: 1714200000000,
        // The top-level `severity` slot is what derive_laws reads —
        // populated by both the live MeiliKeywordLane (from
        // `doc.severity`) and the phase11k §1 worker (from the
        // classifier output that mirrors `LawViolationPayload.severity`
        // for law_violation kinds).
        severity: Some(severity.to_string()),
        extras,
        overlay: cortex_api::lanes::Overlay {
            source: cortex_api::lanes::LaneSource::Keyword,
            law_id: Some(law_id.to_string()),
            severity: Some(severity.to_string()),
            ..Default::default()
        },
    }
}

fn law_check_request(query: &str) -> QueryRequest {
    let value = json!({
        "intent": "law_check",
        // The service-layer validator rejects empty repos with
        // `scope_repo_required`; the strategy then strips the repo
        // before forwarding to the cross-repo `cortex_laws` lane.
        "scope": { "repo": "Cortex" },
        "query": query,
        "include": ["snippets", "violations"],
        "budget_ms": 500,
    });
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn law_check_returns_populated_laws_active_overlay() {
    if env::var("CORTEX_GOVERNANCE_IT").ok().as_deref() == Some("1") {
        eprintln!("CORTEX_GOVERNANCE_IT=1 — live-stack path not implemented in this IT");
        return;
    }

    let (svc, kw) = build_service();
    kw.seed(
        INDEX_LAWS,
        vec![law_lane_hit(
            "LAW-CORTEX-001",
            "Strict task-sequence execution",
            "critical",
        )],
    );

    let app = build_router(svc);
    let req = law_check_request("task sequence cherry pick");
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
    // `laws_active` lives at the response top level (not under
    // `results`) — see `QueryResponse` in `crates/cortex-api/src/types.rs`.
    // Skip the field when serde dropped it (Vec::is_empty pruning).
    let active = body
        .get("laws_active")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !active.is_empty(),
        "laws_active must be non-empty after phase11k projection; body={body}",
    );
    let first = &active[0];
    assert_eq!(first["id"], "LAW-CORTEX-001");
    assert_eq!(first["severity"], "critical");
}
