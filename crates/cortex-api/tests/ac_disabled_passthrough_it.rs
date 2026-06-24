//! Phase21 §7.2 — pinned IT: when `access_control.enabled = false` (the
//! default), `run_with_principal` is a no-op pass-through regardless of the
//! principal's clearance level or compartment grants.
//!
//! A principal with clearance=0 (public-only) querying against a corpus that
//! contains `restricted` (level=3) and compartment-gated facts MUST receive
//! all hits when AC is disabled — the feature flag gate is the invariant.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, QueryRequest, Scope};
use cortex_api::Principal;
use cortex_config::AccessControlConfig;
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
        let comps: Vec<serde_json::Value> = compartments.iter().map(|c| json!(c)).collect();
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

/// Build an orchestrator with AC **disabled** (the default) containing the
/// given static keyword hits.
fn orch_ac_disabled(hits: Vec<LaneHit>) -> Orchestrator {
    let v = Arc::new(EmptyVectorLane);
    let k: Arc<dyn KeywordLane> = Arc::new(StaticKeywordLane(hits));
    let g = Arc::new(MemoryGraphLane::new());
    // Default AccessControlConfig has `enabled = false` — the orch starts AC-off.
    Orchestrator::new(v, k, g)
}

/// Phase21 §7.2 — when AC is disabled, a principal with the minimum
/// possible clearance (0 = public) MUST receive all hits including those
/// at `restricted` (level 3) and behind compartment gates. The feature
/// flag is the only control; no lattice check runs.
#[tokio::test]
async fn disabled_ac_passes_all_hits_to_low_clearance_principal() {
    let hits = vec![
        hit_classified("public-doc", 0, &[]),
        hit_classified("internal-doc", 1, &[]),
        hit_classified("confidential-doc", 2, &["financial"]),
        hit_classified("restricted-doc", 3, &["security", "hr"]),
    ];
    let orch = orch_ac_disabled(hits);
    assert!(
        !orch.current_access_control().enabled,
        "pre-condition: AC must be disabled"
    );

    let public_principal = Principal {
        id: "unprivileged".into(),
        clearance_level: 0,
        compartment_grants: vec![],
        roles: vec![],
    };

    let (resp, _) = orch.run_with_principal(&base_request(), &public_principal).await;
    let paths: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .map(|s| s.path.as_deref().unwrap_or(""))
        .collect();

    assert_eq!(
        paths.len(),
        4,
        "all 4 hits must reach the response when AC is disabled; got: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("public-doc")),
        "public-doc missing; got: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("internal-doc")),
        "internal-doc missing; got: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("confidential-doc")),
        "confidential-doc must pass when AC is disabled; got: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("restricted-doc")),
        "restricted-doc must pass when AC is disabled; got: {paths:?}"
    );
}

/// Phase21 §7.2 — enabling AC on the same orchestrator (via
/// `with_access_control`) flips the gate live: the same low-clearance
/// principal is now subject to the Bell-LaPadula lattice filter and can
/// no longer see `confidential` or `restricted` hits.
#[tokio::test]
async fn enabling_ac_restores_filtering_for_low_clearance_principal() {
    let hits = vec![
        hit_classified("public-doc", 0, &[]),
        hit_classified("internal-doc", 1, &[]),
        hit_classified("confidential-doc", 2, &[]),
        hit_classified("restricted-doc", 3, &[]),
    ];

    let v = Arc::new(EmptyVectorLane);
    let k: Arc<dyn KeywordLane> = Arc::new(StaticKeywordLane(hits));
    let g = Arc::new(MemoryGraphLane::new());
    let orch = Orchestrator::new(v, k, g).with_access_control(AccessControlConfig {
        enabled: true,
        ..AccessControlConfig::default()
    });
    assert!(
        orch.current_access_control().enabled,
        "pre-condition: AC must be enabled"
    );

    let clearance_one = Principal {
        id: "clearance-one".into(),
        clearance_level: 1,
        compartment_grants: vec![],
        roles: vec![],
    };

    let (resp, _) = orch.run_with_principal(&base_request(), &clearance_one).await;
    let paths: Vec<&str> = resp
        .results
        .snippets
        .iter()
        .map(|s| s.path.as_deref().unwrap_or(""))
        .collect();

    assert_eq!(
        paths.len(),
        2,
        "only public + internal must reach the response when AC is enabled; got: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("confidential-doc")),
        "confidential-doc must be filtered when AC is enabled; got: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("restricted-doc")),
        "restricted-doc must be filtered when AC is enabled; got: {paths:?}"
    );
}
