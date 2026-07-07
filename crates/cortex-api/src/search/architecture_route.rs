//! Phase27c §2 — GraphRAG global query route.
//!
//! GraphRAG's "global" query class ("what are the subsystems and how
//! do they relate", architecture/onboarding questions) is not
//! answerable by per-chunk fusion — no single code/doc chunk holds
//! the answer. The map-reduce split follows GraphRAG's own shape:
//! the expensive **map** step runs OFFLINE in the consolidator
//! (phase27c §1 — one Haiku summary per graph community per Leiden
//! level), and the query-time **reduce** is a budgeted top-N
//! selection over those pre-computed community summaries, assembled
//! by the pre-thinking renderer's existing `Consolidated context`
//! section within its byte budget. A query-time LLM reduce is
//! deliberately NOT run here: the sync query path carries an 800 ms
//! default budget and a Haiku call costs ~1.5 s — the offline-map /
//! budgeted-assembly split is the design, not a shortcut.
//!
//! Detection is a pure heuristic over the query text
//! ([`is_architecture_query`]) wired into `build_plan` for
//! `Intent::FreeSearch` only — the `pre_change_context` agent hot
//! path keeps its per-chunk plan so a false positive can never
//! degrade the primary pre-thinking flow.
//!
//! Live reality (2026-07): zero `grain = community` consolidations
//! exist until phase27b §2.5's writeback populates the graph
//! (ADR-027) and the §1 grain runs over it — the route then returns
//! whatever other consolidations rank, degrading gracefully to the
//! ordinary consolidation corpus.

use cortex_storage::names::{COLLECTION_CONSOLIDATION_FP32, INDEX_CONSOLIDATIONS};

use crate::lanes::{KeywordRequest, VectorRequest};
use crate::types::QueryRequest;

use super::strategies::{BudgetSplit, Plan};

/// Phrases that mark a query as architecture-level ("global" in
/// GraphRAG terms). Lowercase; matched as substrings against the
/// lowercased query. Kept deliberately conservative — a false
/// positive reroutes a search away from the code/docs corpus, so
/// each entry should be hard to type by accident when hunting for a
/// specific symbol.
const ARCHITECTURE_MARKERS: &[&str] = &[
    "architecture",
    "subsystem",
    "system overview",
    "overview of the system",
    "how is the system structured",
    "how is the codebase structured",
    "how is the project structured",
    "structure of the codebase",
    "structure of the project",
    "major components",
    "main components",
    "how do the modules relate",
    "how do the components relate",
    "module map",
    "component map",
    "high-level design",
    "high level design",
    "birds-eye view",
    "bird's-eye view",
];

/// `true` when the query reads as an architecture-level question the
/// per-chunk fusion lanes cannot answer.
pub fn is_architecture_query(query: &str) -> bool {
    let q = query.to_lowercase();
    ARCHITECTURE_MARKERS.iter().any(|m| q.contains(m))
}

/// Build the global-route plan: vector + keyword over the
/// consolidation corpus (where the §1 community summaries live),
/// no graph fan-out, no post-fusion overlays. `limit`/`k` from the
/// request bound the top-N; the pre-thinking renderer's byte budget
/// caps the assembled output.
pub fn architecture_plan(req: &QueryRequest) -> Plan {
    Plan {
        vectors: vec![VectorRequest {
            collection: COLLECTION_CONSOLIDATION_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
            acl: None,
        }],
        keywords: vec![KeywordRequest {
            index: INDEX_CONSOLIDATIONS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
            acl: None,
        }],
        graphs: Vec::new(),
        overlays: Vec::new(),
        split_pct: BudgetSplit {
            vector: 50,
            keyword: 50,
            graph: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Intent, QueryRequest, Scope};

    fn req(query: &str) -> QueryRequest {
        QueryRequest {
            intent: Intent::FreeSearch,
            query: query.into(),
            scope: Scope::default(),
            limit: 20,
            k: 50,
            include: Vec::new(),
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

    #[test]
    fn detector_matches_architecture_phrasings() {
        for q in [
            "What is the architecture of this system?",
            "give me a system overview",
            "what are the major components and how do they interact",
            "How is the codebase structured?",
            "Explain the subsystems",
            "high-level design of cortex",
        ] {
            assert!(is_architecture_query(q), "should match: {q}");
        }
    }

    #[test]
    fn detector_ignores_symbol_and_bug_queries() {
        for q in [
            "where is render_edge_merge defined",
            "fix the failing unknown_language_falls_back test",
            "how do I call the vectorizer sdk",
            "grep for community_id",
            "why does the classifier return None",
        ] {
            assert!(!is_architecture_query(q), "should NOT match: {q}");
        }
    }

    #[test]
    fn architecture_plan_targets_consolidations_only_no_overlays() {
        let r = req("what is the architecture here");
        let plan = architecture_plan(&r);
        assert_eq!(plan.vectors.len(), 1);
        assert_eq!(plan.vectors[0].collection, COLLECTION_CONSOLIDATION_FP32);
        assert_eq!(plan.keywords.len(), 1);
        assert_eq!(plan.keywords[0].index, INDEX_CONSOLIDATIONS);
        assert!(
            plan.graphs.is_empty(),
            "no graph fan-out on the global route"
        );
        assert!(plan.overlays.is_empty(), "no post-fusion overlays");
        assert_eq!(plan.split_pct.vector, 50);
        assert_eq!(plan.split_pct.keyword, 50);
        assert_eq!(plan.split_pct.graph, 0);
    }
}
