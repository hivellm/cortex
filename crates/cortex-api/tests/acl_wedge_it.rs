//! Phase21 §5.5 — post-fusion ACL drop-wedge integration tests.
//!
//! Drives `Orchestrator::run_with_principal` with a seeded
//! `MemoryKeywordLane` containing hits at mixed classification levels
//! and compartments. Verifies that only hits the principal may read
//! (per Bell-LaPadula) reach the response.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, QueryRequest, Scope};
use cortex_api::Principal;
use serde_json::json;

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

struct StaticKeywordLane(Vec<LaneHit>);

#[async_trait]
impl KeywordLane for StaticKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.0.clone())
    }
}

fn hit_classified(id: &str, level: u8, compartments: &[&str]) -> LaneHit {
    let mut extras = std::collections::BTreeMap::new();
    extras.insert("class_level".to_string(), json!(level));
    if !compartments.is_empty() {
        let comps: Vec<serde_json::Value> =
            compartments.iter().map(|c| json!(c)).collect();
        extras.insert("class_compartments".to_string(), json!(comps));
    }
    LaneHit {
        doc_id: id.to_string(),
        text: id.to_string(),
        repo: Some("repo".into()),
        path: Some(format!("{id}.rs")),
        symbol: None,
        content_hash: None,
        score: 1.0,
        ts: 0,
        severity: None,
        extras,
        overlay: Overlay {
            source: cortex_api::lanes::LaneSource::Keyword,
            ..Default::default()
        },
    }
}

fn base_request() -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope::default(),
        query: "test".into(),
        limit: 20,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 2000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
        principal: None,
    }
}

fn orch(hits: Vec<LaneHit>) -> Orchestrator {
    let v = Arc::new(EmptyVectorLane);
    let k: Arc<dyn KeywordLane> = Arc::new(StaticKeywordLane(hits));
    let g = Arc::new(MemoryGraphLane::new());
    Orchestrator::new(v, k, g)
}

/// Phase21 §5.5 — principal with clearance=1 must receive only
/// `public` (level=0) and `internal` (level=1) hits; `confidential`
/// (level=2) and `restricted` (level=3) must be dropped.
#[tokio::test]
async fn acl_wedge_drops_hits_above_principal_clearance() {
    let hits = vec![
        hit_classified("public-doc", 0, &[]),
        hit_classified("internal-doc", 1, &[]),
        hit_classified("confidential-doc", 2, &[]),
        hit_classified("restricted-doc", 3, &[]),
    ];
    let orch = orch(hits);
    let principal = Principal {
        id: "tester".into(),
        clearance_level: 1,
        compartment_grants: vec![],
        roles: vec![],
    };
    let (resp, _) = orch.run_with_principal(&base_request(), &principal).await;
    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .map(|s| s.path.as_deref().unwrap_or(""))
        .collect();
    assert!(
        ids.iter().any(|p| p.contains("public-doc")),
        "public-doc must pass; got: {ids:?}"
    );
    assert!(
        ids.iter().any(|p| p.contains("internal-doc")),
        "internal-doc must pass; got: {ids:?}"
    );
    assert!(
        !ids.iter().any(|p| p.contains("confidential-doc")),
        "confidential-doc must be dropped; got: {ids:?}"
    );
    assert!(
        !ids.iter().any(|p| p.contains("restricted-doc")),
        "restricted-doc must be dropped; got: {ids:?}"
    );
}

/// Phase21 §5.5 — principal with required compartment must not see hits
/// that carry a compartment it does not hold, even when the level is
/// within clearance.
#[tokio::test]
async fn acl_wedge_drops_hits_with_missing_compartment() {
    let hits = vec![
        hit_classified("no-compart", 1, &[]),
        hit_classified("financial-doc", 1, &["financial"]),
        hit_classified("hr-doc", 1, &["hr"]),
        hit_classified("both-doc", 1, &["financial", "hr"]),
    ];
    let orch = orch(hits);
    let principal = Principal {
        id: "finance-analyst".into(),
        clearance_level: 2,
        compartment_grants: vec!["financial".into()],
        roles: vec![],
    };
    let (resp, _) = orch.run_with_principal(&base_request(), &principal).await;
    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .map(|s| s.path.as_deref().unwrap_or(""))
        .collect();
    assert!(
        ids.iter().any(|p| p.contains("no-compart")),
        "compartment-free doc must pass; got: {ids:?}"
    );
    assert!(
        ids.iter().any(|p| p.contains("financial-doc")),
        "financial-doc must pass (grant held); got: {ids:?}"
    );
    assert!(
        !ids.iter().any(|p| p.contains("hr-doc")),
        "hr-doc must be dropped (grant not held); got: {ids:?}"
    );
    assert!(
        !ids.iter().any(|p| p.contains("both-doc")),
        "both-doc must be dropped (hr grant not held); got: {ids:?}"
    );
}

/// Phase21 §5.5 — when ACL is absent (`run` instead of
/// `run_with_principal`) all hits pass through regardless of their
/// classification level, preserving backward compatibility.
#[tokio::test]
async fn no_acl_context_passes_all_hits_through() {
    let hits = vec![
        hit_classified("public-doc", 0, &[]),
        hit_classified("restricted-doc", 3, &["security"]),
    ];
    let orch = orch(hits);
    let (resp, _) = orch.run(&base_request()).await;
    let ids: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .map(|s| s.path.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(ids.len(), 2, "both hits must pass when ACL is absent; got: {ids:?}");
}
