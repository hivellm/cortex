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

/// Resolve the per-repo collection / index family the strategies
/// should target. Reads `req.scope.repos[0]` and slugifies it; an
/// empty scope falls back to `unknown` so the produced name is always
/// well-formed (and the lane simply returns zero hits — surfacing the
/// missing scope to the caller).
fn repo_scoped(req: &QueryRequest, family: &str) -> String {
    use cortex_storage::names::{slug_for_repo, UNKNOWN_REPO_SLUG};
    let slug = req
        .scope
        .repo
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    format!("cortex-{slug}-{family}")
}

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
        Intent::Explain => explain(req),
    }
}

fn pre_change_context(req: &QueryRequest) -> Plan {
    let collections = vec![repo_scoped(req, "code"), repo_scoped(req, "docs")];
    let vectors = collections
        .into_iter()
        .map(|c| VectorRequest {
            collection: c,
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        })
        .collect();
    let keywords = vec![
        repo_scoped(req, "code"),
        repo_scoped(req, "docs"),
        repo_scoped(req, "decisions"),
    ]
    .into_iter()
    .map(|i| KeywordRequest {
        index: i,
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
        collection: repo_scoped(req, "decisions"),
        query: req.query.clone(),
        k: req.k,
        scope: req.scope.clone(),
    }];
    let keywords = vec![KeywordRequest {
        index: repo_scoped(req, "decisions"),
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
        collection: repo_scoped(req, "turns"),
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
        index: repo_scoped(req, "governance"),
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

/// Phase6d — `Intent::Explain` plan factory.
///
/// Navigational / explanatory prompts ("how does X work", "what is
/// X", "where is X defined") need the *retrieval* shape of
/// `pre_change_context` (vector + keyword fan-out on `code` /
/// `docs`) but NOT its overlay set: surfacing decisions / laws /
/// similar-turns on a code-reading question burns budget and
/// crowds the snippet ladder with policy noise.
///
/// Graph leg: today the `edge_artifact_definitions` strategy
/// hasn't shipped yet (tracked under phase4c). We omit the graph
/// leg entirely until that lands rather than reusing
/// `edge_artifact_touched_neighbours`, which surfaces files the
/// user *touched* — irrelevant to "explain X" prompts.
fn explain(req: &QueryRequest) -> Plan {
    let collections = vec![repo_scoped(req, "code"), repo_scoped(req, "docs")];
    let vectors = collections
        .into_iter()
        .map(|c| VectorRequest {
            collection: c,
            query: req.query.clone(),
            // Phase6d — modest fan-out (8 per lane). Explanatory
            // questions usually resolve from the top-3 — pulling
            // 50 vectors burns the trim ladder for no gain.
            k: req.k.min(8),
            scope: req.scope.clone(),
        })
        .collect();
    let keywords = vec![repo_scoped(req, "code"), repo_scoped(req, "docs")]
        .into_iter()
        .map(|i| KeywordRequest {
            index: i,
            query: req.query.clone(),
            limit: req.limit.min(8),
            scope: req.scope.clone(),
        })
        .collect();
    Plan {
        vectors,
        keywords,
        // No graph leg until phase4c lands `edge_artifact_definitions`.
        graphs: Vec::new(),
        // Snippets only — explicitly empty even when the caller
        // asked for `decisions` / `violations` / `similar_turns`,
        // because explanatory prompts shouldn't carry policy
        // noise. The orchestrator's overlay-derivation step
        // collapses to a no-op when the allow list is empty.
        overlays: overlays_from_include(&req.include, &[]),
        // Vector-heavy split for navigational prompts: cosine
        // similarity recovers semantic-near matches the keyword
        // tokenizer misses on free-form English questions.
        split_pct: BudgetSplit {
            vector: 60,
            keyword: 40,
            graph: 0,
        },
    }
}

fn free_search(req: &QueryRequest) -> Plan {
    let vectors = vec![VectorRequest {
        collection: repo_scoped(req, "code"),
        query: req.query.clone(),
        k: req.k,
        scope: req.scope.clone(),
    }];
    let keywords = vec![KeywordRequest {
        index: repo_scoped(req, "code"),
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

    // ---- Phase6d Explain plan ----

    #[test]
    fn explain_uses_vector_and_keyword_no_graph_no_overlays() {
        // Even when the caller asks for the full overlay set, the
        // Explain plan strips them — navigational prompts must not
        // burn budget on decisions / laws / similar-turns.
        let plan = build_plan(&req(Intent::Explain));
        assert_eq!(plan.vectors.len(), 2, "code + docs collections");
        assert_eq!(plan.keywords.len(), 2, "code + docs indexes");
        assert!(
            plan.graphs.is_empty(),
            "no graph leg until phase4c lands edge_artifact_definitions"
        );
        assert!(
            plan.overlays.is_empty(),
            "Explain must NOT carry decisions / violations / similar_turns / graph_neighbors overlays"
        );
        // Vector-heavy split — semantic similarity recovers
        // English-question hits the keyword tokenizer misses.
        assert_eq!(plan.split_pct.vector, 60);
        assert_eq!(plan.split_pct.keyword, 40);
        assert_eq!(plan.split_pct.graph, 0);
    }

    #[test]
    fn explain_caps_per_lane_fan_out_at_eight() {
        let mut r = req(Intent::Explain);
        // Caller asked for k=50 / limit=20; Explain caps at 8 to
        // avoid wasting the trim ladder on irrelevant tail hits.
        r.k = 50;
        r.limit = 20;
        let plan = build_plan(&r);
        assert!(
            plan.vectors.iter().all(|v| v.k == 8),
            "vector k capped at 8: got {:?}",
            plan.vectors.iter().map(|v| v.k).collect::<Vec<_>>()
        );
        assert!(
            plan.keywords.iter().all(|kw| kw.limit == 8),
            "keyword limit capped at 8: got {:?}",
            plan.keywords.iter().map(|kw| kw.limit).collect::<Vec<_>>()
        );
    }

    #[test]
    fn explain_preserves_smaller_caller_caps() {
        let mut r = req(Intent::Explain);
        // Caller asked for less than the cap — honour their value.
        r.k = 3;
        r.limit = 5;
        let plan = build_plan(&r);
        assert!(plan.vectors.iter().all(|v| v.k == 3));
        assert!(plan.keywords.iter().all(|kw| kw.limit == 5));
    }
}
