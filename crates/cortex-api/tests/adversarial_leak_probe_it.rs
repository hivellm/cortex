//! Phase21 §9.2 — Adversarial leak probe — hard CI gate.
//!
//! A principal with `clearance_level = 0` and no compartment grants
//! exercises the post-fusion ACL wedge against a corpus seeded
//! **entirely** with classified facts (level ≥ 1 or compartment-gated).
//! Zero hits MUST be returned from every surface exercised here.
//!
//! Any failure in this file represents a security regression and MUST
//! block CI. Do not add `#[ignore]` or conditional skip logic.
//!
//! ## Surfaces probed
//!
//! | Surface | File | Probe |
//! |---|---|---|
//! | Post-fusion wedge | `orchestrator.rs::apply_acl_wedge` | `run_with_principal` returns 0 hits |
//! | `can_read` level gate | `principal.rs` | Predicate returns `false` for every labelled case |
//! | `can_read` compartment gate | `principal.rs` | Missing grant → `false` regardless of level |
//!
//! The pre-thinking bundle filter and raw proxy surfaces are exercised
//! in their own dedicated ITs (`bundle_acl_it.rs`, `raw_proxy_acl_it.rs`).

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use cortex_api::{can_read, Principal};
use cortex_config::AccessControlConfig;
use serde_json::json;

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

struct RestrictedCorpusLane(Vec<LaneHit>);

#[async_trait]
impl KeywordLane for RestrictedCorpusLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.0.clone())
    }
}

fn classified_hit(id: &str, level: u8, compartments: &[&str]) -> LaneHit {
    let mut extras = Props::new();
    extras.insert("class_level".to_string(), json!(level));
    if !compartments.is_empty() {
        let comps: Vec<serde_json::Value> = compartments.iter().map(|c| json!(c)).collect();
        extras.insert("class_compartments".to_string(), json!(comps));
    }
    LaneHit {
        doc_id: id.to_string(),
        text: format!("classified fact {id}"),
        repo: Some("test-repo".into()),
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
        query: "secret facts".into(),
        limit: 100,
        k: 100,
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

/// Adversarial principal: lowest possible clearance, no compartment grants.
fn zero_clearance_principal() -> Principal {
    Principal {
        id: "adversary".into(),
        clearance_level: 0,
        compartment_grants: vec![],
        roles: vec![],
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Seeded corpus of exclusively classified facts (every level ≥ 1 or with
/// a compartment requirement). A zero-clearance principal must retrieve
/// NONE of these.
fn restricted_corpus() -> Vec<LaneHit> {
    vec![
        classified_hit("internal-plain", 1, &[]),
        classified_hit("confidential-plain", 2, &[]),
        classified_hit("restricted-plain", 3, &[]),
        classified_hit("public-financial", 0, &["financial"]),
        classified_hit("public-hr", 0, &["hr"]),
        classified_hit("public-legal", 0, &["legal"]),
        classified_hit("public-security", 0, &["security"]),
        classified_hit("public-pii", 0, &["customer_pii"]),
        classified_hit("internal-financial", 1, &["financial"]),
        classified_hit("confidential-hr", 2, &["hr"]),
        classified_hit("restricted-all", 3, &["financial", "hr", "legal", "security"]),
    ]
}

// ---------------------------------------------------------------------------
// Surface 1 — Post-fusion ACL wedge (orchestrator path)
// ---------------------------------------------------------------------------

/// Phase21 §9.2 — zero-clearance principal must retrieve ZERO hits from a
/// corpus seeded entirely with classified facts when AC is enabled.
#[test]
fn adversarial_probe_post_fusion_wedge_returns_zero_hits() {
    let corpus = restricted_corpus();
    let corpus_len = corpus.len();

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(RestrictedCorpusLane(corpus)),
        Arc::new(MemoryGraphLane::new()),
    )
    .with_access_control(AccessControlConfig {
        enabled: true,
        ..AccessControlConfig::default()
    });

    let principal = zero_clearance_principal();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (resp, _) = rt.block_on(async {
        orch.run_with_principal(&base_request(), &principal).await
    });

    let hits: Vec<_> = resp
        .results
        .snippets
        .iter()
        .filter(|s| s.path.as_deref().map(|p| !p.is_empty()).unwrap_or(false))
        .collect();

    assert_eq!(
        hits.len(),
        0,
        "adversarial probe: zero-clearance principal must retrieve 0 hits \
         from {corpus_len} classified facts; got {} hits: {:?}",
        hits.len(),
        hits.iter()
            .map(|s| s.path.as_deref().unwrap_or("?"))
            .collect::<Vec<_>>()
    );
}

/// Phase21 §9.2 — confirm the same corpus yields hits when AC is disabled
/// (verifies the corpus itself is non-empty and the probe would detect
/// a disabled-AC regression).
#[test]
fn adversarial_probe_corpus_is_non_empty_when_ac_disabled() {
    let corpus = restricted_corpus();

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(RestrictedCorpusLane(corpus)),
        Arc::new(MemoryGraphLane::new()),
    );
    // AC NOT enabled — pass-through.

    let principal = zero_clearance_principal();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (resp, _) = rt.block_on(async {
        orch.run_with_principal(&base_request(), &principal).await
    });

    // When AC is disabled, run_with_principal early-returns run() — all
    // hits pass through. The corpus has 11 classified facts so the
    // snippet bag must be non-empty.
    assert!(
        !resp.results.snippets.is_empty(),
        "adversarial probe sanity: disabled AC must pass classified hits through \
         so a re-enable regression is detectable"
    );
}

// ---------------------------------------------------------------------------
// Surface 2 — can_read predicate exhaustive adversarial check
// ---------------------------------------------------------------------------

/// Phase21 §9.2 — the `can_read` predicate MUST return `false` for every
/// fact in the restricted corpus when evaluated against the adversarial
/// principal.
#[test]
fn adversarial_probe_can_read_denies_every_classified_fact() {
    let principal = zero_clearance_principal();

    let cases: Vec<(u8, Vec<String>)> = vec![
        (1, vec![]),
        (2, vec![]),
        (3, vec![]),
        (0, vec!["financial".into()]),
        (0, vec!["hr".into()]),
        (0, vec!["legal".into()]),
        (0, vec!["security".into()]),
        (0, vec!["customer_pii".into()]),
        (1, vec!["financial".into()]),
        (2, vec!["hr".into()]),
        (3, vec!["financial".into(), "hr".into(), "legal".into(), "security".into()]),
    ];

    let mut leaks: Vec<(u8, &[String])> = vec![];
    for (level, comps) in &cases {
        if can_read(&principal, *level, comps) {
            leaks.push((*level, comps));
        }
    }

    assert!(
        leaks.is_empty(),
        "adversarial probe: can_read returned true for {} classified case(s): {:?}",
        leaks.len(),
        leaks
    );
}
