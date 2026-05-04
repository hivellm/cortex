//! Intent → execution-plan mapper.
//!
//! Mirrors `docs/specs/11-query-api.md` §Intent → strategy table.
//! Every intent owns a pure `fn` that builds a [`Plan`] from the
//! request — adding a new intent is a compile error until a builder
//! exists, so the dispatcher's exhaustive match catches drift at
//! build time rather than at runtime.

use serde_json::json;

use cortex_storage::names::{
    COLLECTION_CONSOLIDATION_FP32, COLLECTION_DECISION_FP32, COLLECTION_TOPIC_CARD_FP32,
    COLLECTION_TURN_FP32, COLLECTION_TURN_PQ, INDEX_CONSOLIDATIONS, INDEX_DECISIONS, INDEX_LAWS,
    INDEX_TOPIC_CARDS, INDEX_TURNS,
};

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
    // Phase11j §4.1 — fan out to the consolidations lane alongside
    // the code/docs corpus. Hot tier collection + global Meili index
    // keep the read path simple (no per-repo consolidation index
    // exists yet — consolidations live in the global lane the
    // dashboard reads, scope.repo is honoured via the document
    // `repo` filter).
    let collections = vec![
        repo_scoped(req, "code"),
        repo_scoped(req, "docs"),
        COLLECTION_CONSOLIDATION_FP32.to_string(),
        // Phase11r §5.2 — topic cards lead the renderer's context
        // band; fan out to the hot-tier collection so the section
        // surfaces when a relevant card exists. Warm / PQ tier
        // intentionally not consulted on this lane — staleness is
        // handled by the §5.4 advisory + downgrade.
        COLLECTION_TOPIC_CARD_FP32.to_string(),
    ];
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
        INDEX_CONSOLIDATIONS.to_string(),
        // Phase11r §5.2 — global topic-cards index handles the
        // literal-phrase recall path (a topic_slug, a quoted ADR
        // id); per-repo scoping rides on the document `repos`
        // field that the §3.3 settings.v1 pass made filterable.
        INDEX_TOPIC_CARDS.to_string(),
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

/// phase10a — `decision_lookup` fans out across the global decision
/// stores. The 2026-04-29 audit caught the previous per-repo
/// `cortex-{slug}-decisions` plan returning empty bodies because the
/// rationale text lives in the global `cortex_decisions` Meili index
/// + `cortex.decision.fp32` Vectorizer collection (per
/// `cortex-storage::names`). Fusing `Vectorizer + Meili + Nexus`
/// against those globals — same RRF blend `pre_change_context` uses
/// — closes the 50% recall floor on `decision_lookup` queries whose
/// answer lives in the ADR body, not the title.
///
/// phase11e §5.3 — also fan out to the canonical per-repo
/// `cortex-{slug}-decisions` collection / index that the
/// writer-side spec 06 / 08 actually populates today. The legacy
/// globals stay so historical bootstraps keep their content
/// reachable, but the per-repo names are what every NEW envelope
/// from the embedder + fulltext-worker lands on. Both lanes
/// gracefully fall back to empty hits when one of the names is
/// absent (404 on per-repo or legacy alike).
fn decision_lookup(req: &QueryRequest) -> Plan {
    let vectors = vec![
        VectorRequest {
            collection: COLLECTION_DECISION_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        VectorRequest {
            collection: repo_scoped(req, "decisions"),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        // Phase11r §5.2 — topic cards drill into evidence (§4.2),
        // making them a natural cross-lookup surface for
        // `decision_lookup`: a card synthesised over an ADR is
        // the agent's shortest path from "what was decided" to
        // "why and what's the tension".
        VectorRequest {
            collection: COLLECTION_TOPIC_CARD_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
    ];
    let keywords = vec![
        KeywordRequest {
            index: INDEX_DECISIONS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
        KeywordRequest {
            index: repo_scoped(req, "decisions"),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
        // Phase11r §5.2 — paired with the vector lane above.
        KeywordRequest {
            index: INDEX_TOPIC_CARDS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
    ];
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

/// phase10a — `similar_problems` queries the global turn corpus.
/// The audit caught the previous plan only fanning out to the
/// vector lane (and against a per-repo collection that's frequently
/// empty); both the FP32 hot tier and the PQ warm tier are now
/// queried, and the keyword lane gets `cortex_turns` so callers
/// looking for past turns by literal phrase (a tool name, an error
/// string) match. `scope.repo` round-trips through the lane so a
/// repo-scoped query does not bleed across other sessions stored in
/// the same global index.
fn similar_problems(req: &QueryRequest) -> Plan {
    let vectors = vec![
        VectorRequest {
            collection: COLLECTION_TURN_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        VectorRequest {
            collection: COLLECTION_TURN_PQ.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        // phase11e §5.3 — per-repo `cortex-{slug}-turns` is the
        // canonical write target of the embedder. Fan out so a
        // fresh stack with no legacy global tier still answers
        // similar_problems queries.
        VectorRequest {
            collection: repo_scoped(req, "turns"),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        // Phase11j §4.1 — consolidations beat raw turns at recalling
        // similar past problems because they distill the lesson
        // learned. Hot tier only: warm tier holds older Shallow
        // consolidations whose recall tradeoff is wrong for this
        // strategy.
        VectorRequest {
            collection: COLLECTION_CONSOLIDATION_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
        // Phase11r §5.2 — topic cards beat raw consolidations on
        // similar-problems recall because they fold many
        // consolidations into a single living synthesis. Hot tier
        // only — same reasoning as the consolidation lane.
        VectorRequest {
            collection: COLLECTION_TOPIC_CARD_FP32.to_string(),
            query: req.query.clone(),
            k: req.k,
            scope: req.scope.clone(),
        },
    ];
    let keywords = vec![
        KeywordRequest {
            index: INDEX_TURNS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
        KeywordRequest {
            index: repo_scoped(req, "turns"),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
        // Phase11j §4.1 — global consolidations index supplies the
        // literal-phrase recall path (a tool name, an error string)
        // alongside the vector lane.
        KeywordRequest {
            index: INDEX_CONSOLIDATIONS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
        // Phase11r §5.2 — paired with the topic-card vector lane
        // above; covers the literal-phrase recall path.
        KeywordRequest {
            index: INDEX_TOPIC_CARDS.to_string(),
            query: req.query.clone(),
            limit: req.limit,
            scope: req.scope.clone(),
        },
    ];
    let graphs = vec![GraphRequest {
        template: "turn_analysis_decision_chain".into(),
        params: json!({ "query": req.query }),
        max_hops: 2,
        scope: req.scope.clone(),
    }];
    Plan {
        vectors,
        keywords,
        graphs,
        overlays: overlays_from_include(&req.include, &[Overlay::SimilarTurns]),
        // Vector-heavy: literal phrase + KNN both contribute, but
        // semantic similarity is the load-bearing signal for
        // "similar past problems".
        split_pct: BudgetSplit {
            vector: 50,
            keyword: 25,
            graph: 25,
        },
    }
}

/// phase10a — `law_check` queries the global laws index plus the
/// `:Law`/`:LawViolation` graph. The audit caught the previous plan
/// targeting `cortex-{slug}-governance` (the law-violation per-repo
/// index) which carries violation rows but not the law definitions
/// themselves. The dashboard reports 37 active laws + 121 recent
/// violations; switching the keyword lane to the global `cortex_laws`
/// index surfaces both the law title AND the body excerpt the agent
/// needs to quote back, while the existing
/// `law_violations_last_30d` graph template remains the source for
/// recent-violation overlays.
fn law_check(req: &QueryRequest) -> Plan {
    // Laws are cross-repo by design and the `cortex_laws` Meili
    // index does NOT carry `repo` as a filterable attribute (see
    // `crates/cortex-storage/schemas/meili/cortex_laws.settings.v1.json`),
    // so strip `scope.repo` before forwarding to the keyword lane —
    // otherwise Meili rejects the search with a 4xx and the lane
    // returns zero hits.
    let mut law_scope = req.scope.clone();
    law_scope.repo = None;
    let keywords = vec![KeywordRequest {
        index: INDEX_LAWS.to_string(),
        query: req.query.clone(),
        limit: req.limit,
        scope: law_scope,
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
            budget_bytes: None,
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

    // ---- Phase10a — lane composition for the three audit-detected gaps ----

    #[test]
    fn decision_lookup_targets_global_decision_stores() {
        // phase10a §1.1 — the audit caught `decision_lookup` returning
        // empty because the per-repo `cortex-{slug}-decisions` plan
        // only matched on title. The fix routes to the global
        // `cortex.decision.fp32` collection + `cortex_decisions`
        // index (per `cortex-storage::names`) where the rationale
        // body is searchable.
        // phase11e §5.3 — plan now fans out to both the legacy
        // global tiers AND the per-repo `cortex-{slug}-decisions`
        // names that the writer side actually populates today.
        let plan = build_plan(&req(Intent::DecisionLookup));
        let collections: Vec<_> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_DECISION_FP32),
            "legacy fp32 tier must still be queried: {collections:?}"
        );
        assert!(
            collections.iter().any(|c| c.ends_with("-decisions")),
            "per-repo `cortex-<slug>-decisions` must also be queried: {collections:?}"
        );
        let indexes: Vec<_> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(indexes.contains(&INDEX_DECISIONS));
        assert!(indexes.iter().any(|i| i.ends_with("-decisions")));
        assert_eq!(plan.graphs.len(), 1);
        assert_eq!(plan.graphs[0].template, "decision_supersedes_chain");
        // §1.3 — same RRF blend `pre_change_context` uses.
        assert_eq!(plan.split_pct.vector, 40);
        assert_eq!(plan.split_pct.keyword, 40);
        assert_eq!(plan.split_pct.graph, 20);
    }

    #[test]
    fn law_check_targets_global_laws_index() {
        // phase10a §2.1 — the audit caught `law_check` returning
        // empty because the per-repo `cortex-{slug}-governance`
        // index only carries violation rows, not law definitions.
        let plan = build_plan(&req(Intent::LawCheck));
        assert_eq!(plan.keywords.len(), 1);
        assert_eq!(plan.keywords[0].index, INDEX_LAWS);
        assert_eq!(plan.graphs.len(), 1);
        assert_eq!(plan.graphs[0].template, "law_violations_last_30d");
    }

    #[test]
    fn similar_problems_fans_out_across_turn_tiers_and_keyword() {
        // phase10a §3.1 — the audit caught `similar_problems`
        // empty because it only queried the per-repo turn
        // collection. The fix fans out across both global tiers
        // (`cortex.turn.fp32` hot + `cortex.turn.pq` warm) and adds
        // the keyword lane against `cortex_turns` so literal-phrase
        // matches surface alongside semantic neighbours.
        let plan = build_plan(&req(Intent::SimilarProblems));
        let collections: Vec<_> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_TURN_FP32),
            "fp32 hot tier must be queried: {collections:?}"
        );
        assert!(
            collections.contains(&COLLECTION_TURN_PQ),
            "pq warm tier must be queried: {collections:?}"
        );
        // phase11e §5.3 — per-repo `cortex-{slug}-turns` is the
        // canonical writer destination today and must also fan out.
        assert!(
            collections.iter().any(|c| c.ends_with("-turns")),
            "per-repo `cortex-<slug>-turns` must be queried: {collections:?}"
        );
        let indexes: Vec<_> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(indexes.contains(&INDEX_TURNS));
        assert!(indexes.iter().any(|i| i.ends_with("-turns")));
    }

    #[test]
    fn similar_problems_round_trips_scope_repo_to_keyword_lane() {
        // §3.3 — a repo-scoped query must NOT bleed across other
        // sessions stored under different repos in the same global
        // turn index. The strategies layer round-trips `scope.repo`
        // verbatim onto every lane request; the live keyword lane
        // translates that into a Meili filter.
        let mut r = req(Intent::SimilarProblems);
        r.scope = crate::types::Scope {
            repo: Some("Cortex".into()),
            ..Default::default()
        };
        let plan = build_plan(&r);
        assert!(
            plan.vectors
                .iter()
                .all(|v| v.scope.repo.as_deref() == Some("Cortex")),
            "every vector request must carry the request's scope.repo"
        );
        assert!(
            plan.keywords
                .iter()
                .all(|k| k.scope.repo.as_deref() == Some("Cortex")),
            "every keyword request must carry the request's scope.repo"
        );
    }

    #[test]
    fn free_search_carries_no_overlays() {
        let plan = build_plan(&req(Intent::FreeSearch));
        assert!(plan.overlays.is_empty());
        assert!(plan.graphs.is_empty());
    }

    #[test]
    fn pre_change_context_includes_consolidations_lane() {
        // Phase11j §4.1 — `pre_change_context` must fan out to the
        // consolidations vector + keyword stores so the renderer's
        // `Consolidated context` section has rows to read. Drift
        // here means consolidations land in storage but never reach
        // the agent's pre-thinking bundle.
        let plan = build_plan(&req(Intent::PreChangeContext));
        let collections: Vec<&str> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_CONSOLIDATION_FP32),
            "pre_change_context must query cortex.consolidation.fp32: {collections:?}"
        );
        let indexes: Vec<&str> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(
            indexes.contains(&INDEX_CONSOLIDATIONS),
            "pre_change_context must query cortex_consolidations: {indexes:?}"
        );
    }

    #[test]
    fn similar_problems_includes_consolidations_lane() {
        // Phase11j §4.1 — `similar_problems` benefits even more
        // from the consolidations lane than `pre_change_context`
        // because consolidations distill the lesson learned from
        // past sessions into a single recall-tuned row.
        let plan = build_plan(&req(Intent::SimilarProblems));
        let collections: Vec<&str> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_CONSOLIDATION_FP32),
            "similar_problems must query cortex.consolidation.fp32: {collections:?}"
        );
        let indexes: Vec<&str> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(
            indexes.contains(&INDEX_CONSOLIDATIONS),
            "similar_problems must query cortex_consolidations: {indexes:?}"
        );
    }

    #[test]
    fn pre_change_context_includes_topic_cards_lane() {
        // Phase11r §5.2 — `pre_change_context` must fan out to the
        // hot-tier topic-card vector + keyword stores so the
        // renderer's top-priority section reads from the lane.
        // Drift here means topic cards land in storage but never
        // reach the pre-thinking bundle.
        let plan = build_plan(&req(Intent::PreChangeContext));
        let collections: Vec<&str> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_TOPIC_CARD_FP32),
            "pre_change_context must query cortex.topic_card.fp32: {collections:?}"
        );
        let indexes: Vec<&str> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(
            indexes.contains(&INDEX_TOPIC_CARDS),
            "pre_change_context must query cortex_topic_cards: {indexes:?}"
        );
    }

    #[test]
    fn decision_lookup_includes_topic_cards_lane() {
        // Phase11r §5.2 — topic cards drill into evidence (§4.2),
        // so `decision_lookup` must fan out to the lane to surface
        // the synthesis attached to a queried ADR.
        let plan = build_plan(&req(Intent::DecisionLookup));
        let collections: Vec<&str> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_TOPIC_CARD_FP32),
            "decision_lookup must query cortex.topic_card.fp32: {collections:?}"
        );
        let indexes: Vec<&str> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(
            indexes.contains(&INDEX_TOPIC_CARDS),
            "decision_lookup must query cortex_topic_cards: {indexes:?}"
        );
    }

    #[test]
    fn similar_problems_includes_topic_cards_lane() {
        // Phase11r §5.2 — topic cards beat raw consolidations on
        // similar-problems recall (a topic card folds many
        // consolidations into a single living synthesis).
        let plan = build_plan(&req(Intent::SimilarProblems));
        let collections: Vec<&str> = plan.vectors.iter().map(|v| v.collection.as_str()).collect();
        assert!(
            collections.contains(&COLLECTION_TOPIC_CARD_FP32),
            "similar_problems must query cortex.topic_card.fp32: {collections:?}"
        );
        let indexes: Vec<&str> = plan.keywords.iter().map(|k| k.index.as_str()).collect();
        assert!(
            indexes.contains(&INDEX_TOPIC_CARDS),
            "similar_problems must query cortex_topic_cards: {indexes:?}"
        );
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
