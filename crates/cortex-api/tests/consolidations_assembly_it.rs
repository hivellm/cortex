//! Phase30b §1.2 — `results.consolidations` assembly, end-to-end
//! through the real `Orchestrator` with in-process lane doubles.
//!
//! Two invariants, both learned the hard way against the live stack:
//!
//! 1. **Assembly happens at all.** Until phase30b nothing in
//!    cortex-api constructed a `ConsolidationRef`, so a prior
//!    session's distillate could never reach a fresh session's
//!    pre-thinking bundle (the renderer's "Consolidated context"
//!    section rendered only from hand-built unit fixtures).
//! 2. **Assembly runs BEFORE `truncate(req.limit)`.** `req.limit`
//!    bounds the snippet stream; consolidations carry their own caps.
//!    The first live run of this fix still returned an empty
//!    `results.consolidations` because higher-ranked code/docs hits
//!    consumed the whole limit before the partition saw the list.
//!
//! No network I/O — the keyword lane is an in-process double.

use std::sync::Arc;

use cortex_api::lanes::{
    LaneHit, LaneSource, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Overlay,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, QueryRequest, Scope};

fn code_hit(doc_id: &str, path: &str, score: f64) -> LaneHit {
    LaneHit {
        doc_id: doc_id.to_string(),
        text: format!("code body — {doc_id}"),
        repo: Some("cortex".to_string()),
        path: Some(path.to_string()),
        symbol: Some("some_fn".to_string()),
        content_hash: None,
        score,
        ts: 0,
        severity: None,
        extras: Default::default(),
        overlay: Overlay {
            source: LaneSource::Keyword,
            ..Overlay::default()
        },
    }
}

/// A consolidation hit shaped exactly like the Meili lane projects one:
/// `symbol` carries the kind label, and the `ext.consolidation.*` bag
/// is flattened into extras by `meili_lane::project`.
fn consolidation_hit(doc_id: &str, cons_id: &str, title: &str, score: f64) -> LaneHit {
    let mut extras = cortex_api::types::Props::new();
    extras.insert(
        "consolidation_id".into(),
        serde_json::Value::String(cons_id.into()),
    );
    extras.insert(
        "consolidation_title".into(),
        serde_json::Value::String(title.into()),
    );
    LaneHit {
        doc_id: doc_id.to_string(),
        text: format!("consolidation body — {doc_id}"),
        repo: Some("cortex".to_string()),
        path: None,
        symbol: Some("consolidation".to_string()),
        content_hash: None,
        score,
        ts: 1_782_100_588_131,
        severity: None,
        extras,
        overlay: Overlay {
            source: LaneSource::Keyword,
            consolidation_grain: Some(cortex_core::events::ConsolidationGrain::Session),
            ..Overlay::default()
        },
    }
}

fn req_with_limit(limit: usize) -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("cortex".into()),
            ..Scope::default()
        },
        query: "hook latency fix consolidator resilience".into(),
        limit,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 500,
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

/// Seeds the code index with `code_count` hits that all outrank the
/// single consolidation, then runs the real orchestrator.
async fn run_with_seeded_lanes(
    code_count: usize,
    limit: usize,
) -> cortex_api::types::QueryResponse {
    let keyword = MemoryKeywordLane::new();
    keyword.seed(
        "cortex-cortex-code",
        (0..code_count)
            .map(|i| {
                code_hit(
                    &format!("k-code-{i}"),
                    &format!("crates/cortex-api/src/file_{i}.rs"),
                    10.0 - i as f64,
                )
            })
            .collect(),
    );
    // The consolidations lane the phase30b §1.1 fix repointed at the
    // per-repo uid. Score deliberately below every code hit.
    keyword.seed(
        "cortex-cortex-consolidations",
        vec![consolidation_hit(
            "k-cons-1",
            "cons-ses-2a417d3c7c87fe945e513b57",
            "Hook latency fix & consolidator resilience",
            0.1,
        )],
    );

    let orch = Orchestrator::new(
        Arc::new(MemoryVectorLane::new()),
        Arc::new(keyword),
        Arc::new(MemoryGraphLane::new()),
    );
    let (resp, _) = orch.run(&req_with_limit(limit)).await;
    resp
}

#[tokio::test]
async fn consolidation_hits_assemble_into_results_consolidations() {
    let resp = run_with_seeded_lanes(1, 20).await;

    assert_eq!(
        resp.results.consolidations.len(),
        1,
        "the consolidations lane must reach results.consolidations, \
         not vanish into the snippet stream"
    );
    let c = &resp.results.consolidations[0];
    assert_eq!(c.consolidation_id, "cons-ses-2a417d3c7c87fe945e513b57");
    assert_eq!(c.grain, "session");
    assert_eq!(c.title, "Hook latency fix & consolidator resilience");
    assert_eq!(c.ts, 1_782_100_588_131);

    assert!(
        !resp
            .results
            .snippets
            .iter()
            .any(|s| s.text.contains("consolidation body")),
        "a consolidation must not double-list as a snippet"
    );
}

#[tokio::test]
async fn consolidations_survive_a_limit_smaller_than_the_fused_candidate_list() {
    // The live regression: 5 code hits all outrank the consolidation
    // and `limit: 5` consumes the whole budget. Truncating before the
    // partition drops the consolidation entirely; partitioning first
    // keeps it, because `req.limit` bounds SNIPPETS, not this section.
    let resp = run_with_seeded_lanes(5, 5).await;

    assert_eq!(
        resp.results.consolidations.len(),
        1,
        "a low-ranked consolidation must survive a small req.limit — \
         partition runs before truncate"
    );
    assert_eq!(
        resp.results.snippets.len(),
        5,
        "the snippet stream still honours req.limit"
    );
}
