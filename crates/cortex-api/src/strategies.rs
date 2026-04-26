//! Intent → execution-plan mapper.
//!
//! Mirrors `docs/specs/11-query-api.md` §Intent → strategy table.
//! Every intent owns a pure `fn` that builds a [`Plan`] from the
//! request — adding a new intent is a compile error until a builder
//! exists, so the dispatcher's exhaustive match catches drift at
//! build time rather than at runtime.

use serde_json::json;

use crate::lanes::{GraphRequest, KeywordRequest, VectorRequest};
use crate::types::{IncludeField, Intent, QueryRequest};

/// One execution plan produced from an intent + request. Carries the
/// pre-built lane requests + the overlay set the orchestrator runs
/// after fusion.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Vector lane requests (one per collection).
    pub vectors: Vec<VectorRequest>,
    /// Keyword lane requests (one per index).
    pub keywords: Vec<KeywordRequest>,
    /// Graph lane requests (one per Cypher template).
    pub graphs: Vec<GraphRequest>,
    /// Overlays the orchestrator runs post-fusion.
    pub overlays: Vec<Overlay>,
    /// Sub-budget split for vector / keyword / graph as a percentage
    /// of the request's `budget_ms`.
    pub split_pct: BudgetSplit,
}

/// Overlay flag bag — drives the post-fusion enrichment phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// Decision overlay (Turn → LINKED_TO → Decision).
    Decisions,
    /// Active laws + recent violations within scope.
    LawsAndViolations,
    /// Graph neighbours of the top fused result.
    GraphNeighbors,
    /// Similar-turn KNN against the seed.
    SimilarTurns,
}

/// Sub-budget split used by the orchestrator.
#[derive(Debug, Clone, Copy)]
pub struct BudgetSplit {
    /// Vector lane share (0..=100).
    pub vector: u8,
    /// Keyword lane share (0..=100).
    pub keyword: u8,
    /// Graph lane share (0..=100).
    pub graph: u8,
}

impl BudgetSplit {
    /// Spec-11 default: 40 / 40 / 20.
    pub const fn default_split() -> Self {
        Self {
            vector: 40,
            keyword: 40,
            graph: 20,
        }
    }
}

/// Build the execution plan for the given request.
pub fn build_plan(req: &QueryRequest) -> Plan {
    match req.intent {
        Intent::PreChangeContext => pre_change_context(req),
        Intent::DecisionLookup => decision_lookup(req),
        Intent::SimilarProblems => similar_problems(req),
        Intent::LawCheck => law_check(req),
        Intent::FreeSearch => free_search(req),
    }
}

fn pre_change_context(req: &QueryRequest) -> Plan {
    let collections = vec!["cortex-code".to_string(), "cortex-docs".to_string()];
    let vectors = collections
        .into_iter()
        .map(|c| VectorRequest {
            collection: c,
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        })
        .collect();
    let keywords = vec!["cortex-code", "cortex-docs", "cortex-decisions"]
        .into_iter()
        .map(|i| KeywordRequest {
            index: i.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        })
        .collect();
    let graphs = vec![GraphRequest {
        template: "edge_artifact_touched_neighbours".to_string(),
        params: json!({ "query": req.query }),
        max_hops: 2,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors,
        keywords,
        graphs,
        overlays: overlays_from_include(
            &req.include,
            &[
                Overlay::Decisions,
                Overlay::LawsAndViolations,
                Overlay::GraphNeighbors,
                Overlay::SimilarTurns,
            ],
        ),
        split_pct: BudgetSplit::default_split(),
    }
}

fn decision_lookup(req: &QueryRequest) -> Plan {
    let vectors = vec![VectorRequest {
        collection: "cortex-decisions".into(),
        query: req.query.clone(),
        k: req.k,
        scope: req.scope.clone(),
    }];
    let keywords = vec![KeywordRequest {
        index: "cortex-decisions".into(),
        query: req.query.clone(),
        limit: req.limit,
        scope: req.scope.clone(),
    }];
    let graphs = vec![GraphRequest {
        template: "decision_supersedes_chain".into(),
        params: json!({ "query": req.query }),
        max_hops: 2,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors,
        keywords,
        graphs,
        overlays: overlays_from_include(&req.include, &[Overlay::Decisions]),
        split_pct: BudgetSplit::default_split(),
    }
}

fn similar_problems(req: &QueryRequest) -> Plan {
    let vectors = vec![VectorRequest {
        collection: "cortex-turns".into(),
        query: req.query.clone(),
        k: req.k,
        scope: req.scope.clone(),
    }];
    let graphs = vec![GraphRequest {
        template: "turn_analysis_decision_chain".into(),
        params: json!({ "query": req.query }),
        max_hops: 2,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors,
        keywords: Vec::new(),
        graphs,
        overlays: overlays_from_include(&req.include, &[Overlay::SimilarTurns]),
        split_pct: BudgetSplit {
            vector: 60,
            keyword: 0,
            graph: 40,
        },
    }
}

fn law_check(req: &QueryRequest) -> Plan {
    let keywords = vec![KeywordRequest {
        index: "cortex-governance".into(),
        query: req.query.clone(),
        limit: req.limit,
        scope: req.scope.clone(),
    }];
    let graphs = vec![GraphRequest {
        template: "law_violations_last_30d".into(),
        params: json!({ "query": req.query }),
        max_hops: 1,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors: Vec::new(),
        keywords,
        graphs,
        overlays: overlays_from_include(&req.include, &[Overlay::LawsAndViolations]),
        split_pct: BudgetSplit {
            vector: 0,
            keyword: 60,
            graph: 40,
        },
    }
}

fn free_search(req: &QueryRequest) -> Plan {
    let vectors = vec![VectorRequest {
        collection: "cortex-code".into(),
        query: req.query.clone(),
        k: req.k,
        scope: req.scope.clone(),
    }];
    let keywords = vec![KeywordRequest {
        index: "cortex-code".into(),
        query: req.query.clone(),
        limit: req.limit,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors,
        keywords,
        graphs: Vec::new(),
        overlays: overlays_from_include(&req.include, &[]),
        split_pct: BudgetSplit {
            vector: 50,
            keyword: 50,
            graph: 0,
        },
    }
}

fn overlays_from_include(include: &[IncludeField], allowed: &[Overlay]) -> Vec<Overlay> {
    let mut out = Vec::new();
    for ov in allowed {
        let included = match ov {
            Overlay::Decisions => include.contains(&IncludeField::Decisions),
            Overlay::LawsAndViolations => include.contains(&IncludeField::Violations),
            Overlay::GraphNeighbors => include.contains(&IncludeField::GraphNeighbors),
            Overlay::SimilarTurns => include.contains(&IncludeField::SimilarTurns),
        };
        if included {
            out.push(*ov);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Intent, QueryRequest, Scope};

    fn req(intent: Intent) -> QueryRequest {
        QueryRequest {
            intent,
            scope: Scope::default(),
            query: "test".to_string(),
            limit: 20,
            k: 50,
            include: vec![
                IncludeField::Snippets,
                IncludeField::Decisions,
                IncludeField::Violations,
                IncludeField::GraphNeighbors,
                IncludeField::SimilarTurns,
            ],
            budget_ms: 500,
        }
    }

    #[test]
    fn pre_change_context_uses_three_lanes() {
        let plan = build_plan(&req(Intent::PreChangeContext));
        assert!(!plan.vectors.is_empty());
        assert!(!plan.keywords.is_empty());
        assert!(!plan.graphs.is_empty());
        assert_eq!(plan.split_pct.vector, 40);
    }

    #[test]
    fn law_check_skips_vector_lane() {
        let plan = build_plan(&req(Intent::LawCheck));
        assert!(plan.vectors.is_empty(), "law_check must not run vector");
        assert!(!plan.keywords.is_empty());
        assert!(!plan.graphs.is_empty());
    }

    #[test]
    fn similar_problems_skips_keyword_lane() {
        let plan = build_plan(&req(Intent::SimilarProblems));
        assert!(plan.keywords.is_empty(), "similar_problems must not run keyword");
        assert!(!plan.vectors.is_empty());
    }

    #[test]
    fn free_search_carries_no_overlays() {
        let plan = build_plan(&req(Intent::FreeSearch));
        assert!(plan.overlays.is_empty());
        assert!(plan.graphs.is_empty());
    }

    #[test]
    fn overlays_drop_when_caller_does_not_include_field() {
        let mut r = req(Intent::PreChangeContext);
        r.include = vec![IncludeField::Snippets];
        let plan = build_plan(&r);
        assert!(plan.overlays.is_empty());
    }
}
