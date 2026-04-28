//! Query orchestrator — parallel fan-out + RRF fusion + overlays.
//!
//! Spec 11 §Fan-out + fusion: every plan executes its lanes in
//! parallel under tokio, each carrying its own sub-budget. The
//! orchestrator then fuses with RRF, attaches the requested
//! overlays, and packages the response. Total budget enforcement
//! flips `debug.truncated = true` when stragglers are dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::fusion::rrf_fuse;
use crate::lanes::{GraphLane, KeywordLane, LaneHit, VectorLane};
use crate::strategies::{build_plan, Overlay, Plan};
use crate::types::{
    empty_response, BudgetReport, DecisionRef, GraphNeighbor, IncludeField, Intent, LawRef,
    QueryRequest, QueryResponse, SimilarTurn, Snippet, ViolationRef,
};

/// Orchestrator handles — fan-out clients passed in via `Arc` so the
/// daemon owns lifetimes.
#[derive(Clone)]
pub struct Orchestrator {
    /// Vector-lane client.
    pub vector: Arc<dyn VectorLane>,
    /// Keyword-lane client.
    pub keyword: Arc<dyn KeywordLane>,
    /// Graph-lane client.
    pub graph: Arc<dyn GraphLane>,
}

impl Orchestrator {
    /// Build a new orchestrator wrapping the three lane clients.
    pub fn new(
        vector: Arc<dyn VectorLane>,
        keyword: Arc<dyn KeywordLane>,
        graph: Arc<dyn GraphLane>,
    ) -> Self {
        Self {
            vector,
            keyword,
            graph,
        }
    }

    /// Run the request end-to-end. The caller decides whether to
    /// short-circuit on a cache hit before invoking this.
    pub async fn run(&self, req: &QueryRequest) -> QueryResponse {
        let started = Instant::now();
        let plan = build_plan(req);
        let total_budget = Duration::from_millis(req.budget_ms.max(1));
        let split = plan.split_pct;

        let vector_budget =
            Duration::from_millis(req.budget_ms * split.vector as u64 / 100).max(Duration::from_millis(1));
        let keyword_budget =
            Duration::from_millis(req.budget_ms * split.keyword as u64 / 100).max(Duration::from_millis(1));
        let graph_budget =
            Duration::from_millis(req.budget_ms * split.graph as u64 / 100).max(Duration::from_millis(1));

        let mut response = empty_response(req);
        response.budget.cap_ms = req.budget_ms;
        response.budget.cache = "miss".to_string();

        // ---- Fan out ----
        let vector_fut = run_vector_lane(self.vector.clone(), &plan, vector_budget);
        let keyword_fut = run_keyword_lane(self.keyword.clone(), &plan, keyword_budget);
        let graph_fut = run_graph_lane(self.graph.clone(), &plan, graph_budget);

        let total_fut = tokio::time::timeout(
            total_budget,
            futures_join_three(vector_fut, keyword_fut, graph_fut),
        );
        let (vector_result, keyword_result, graph_result, truncated) = match total_fut.await {
            Ok((v, k, g)) => (v, k, g, false),
            Err(_) => {
                // Total budget elapsed — record what we have and
                // flip the truncated flag. With no per-lane partial
                // accumulator we treat the truncated path as
                // empty-everything; production will hold partials
                // via a `select!` that captures lane completions
                // mid-flight (Phase-2 retrieval-quality follow-up).
                (
                    LaneOutcome::default(),
                    LaneOutcome::default(),
                    LaneOutcome::default(),
                    true,
                )
            }
        };
        response.debug.truncated = truncated;

        // ---- Record lane timings + errors ----
        if let Some(ms) = vector_result.elapsed_ms {
            response.debug.lanes.vector_ms = Some(ms);
        }
        if let Some(ms) = keyword_result.elapsed_ms {
            response.debug.lanes.keyword_ms = Some(ms);
        }
        if let Some(ms) = graph_result.elapsed_ms {
            response.debug.lanes.graph_ms = Some(ms);
        }
        if let Some(err) = &vector_result.error {
            response.debug.errors.insert("vector".into(), err.clone());
        }
        if let Some(err) = &keyword_result.error {
            response.debug.errors.insert("keyword".into(), err.clone());
        }
        if let Some(err) = &graph_result.error {
            response.debug.errors.insert("graph".into(), err.clone());
        }

        // Flip the truncated flag when any lane bailed because of
        // budget pressure — spec 11 §Failure modes: partial results
        // carry `debug.truncated = true` so callers can decide
        // whether to retry with a wider budget.
        let lane_budget_hit = [
            &vector_result.error,
            &keyword_result.error,
            &graph_result.error,
        ]
        .iter()
        .any(|e| {
            e.as_deref()
                .map(|s| s.contains("budget exceeded"))
                .unwrap_or(false)
        });
        if lane_budget_hit {
            response.debug.truncated = true;
        }

        // Debug-only invariant — every keyword-lane hit must carry
        // `extras["source"] = "keyword"`. The 2026-04-27 audit caught
        // hits surfacing as `source = "vector"` because the previous
        // double left the field empty and `lane_label()` falls back
        // to `"vector"`. The live `MeiliKeywordLane` stamps the field;
        // this assert catches a future regression in any new lane
        // implementation.
        debug_assert!(
            keyword_result
                .hits
                .iter()
                .all(|h| h.extras.get("source").and_then(|v| v.as_str()) == Some("keyword")),
            "keyword lane returned a hit without extras[\"source\"] = \"keyword\""
        );

        // ---- Fuse ----
        let mut fused = rrf_fuse(vec![
            vector_result.hits.clone(),
            keyword_result.hits.clone(),
            graph_result.hits.clone(),
        ]);
        // Post-fusion dedupe + degenerate-hit filter. `rrf_fuse` only
        // dedupes by `doc_id`, so the same artifact lands twice when
        // the keyword lane (`meili|...`) and vector lane (`vec|...`)
        // both surface it. Collapse on `(repo, path, symbol)` and
        // keep the first (highest-fused) occurrence so the snippet
        // section doesn't repeat the same file under different lane
        // labels. Hits that carry no identifying tuple AND no text
        // — observed when the vector lane returns empty metadata
        // — are dropped entirely; otherwise they render as blank
        // numbered lines in the spec-12 bundle.
        let mut seen_anchor: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();
        fused.retain(|h| {
            let anchor = (
                h.repo.clone().unwrap_or_default(),
                h.path.clone().unwrap_or_default(),
                h.symbol.clone().unwrap_or_default(),
            );
            let has_anchor = !(anchor.0.is_empty() && anchor.1.is_empty() && anchor.2.is_empty());
            if has_anchor {
                return seen_anchor.insert(anchor);
            }
            // Anchor-less hit: only keep when it carries useful text
            // or a `why` blurb. Filters out vector-lane hits whose
            // upstream metadata was empty.
            let has_text = !h.text.trim().is_empty();
            let has_why = h
                .extras
                .get("why")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            has_text || has_why
        });
        fused.truncate(req.limit);

        if req.include.contains(&IncludeField::Snippets) {
            response.results.snippets =
                fused
                    .iter()
                    .enumerate()
                    .map(|(idx, hit)| snippet_from_hit(idx + 1, hit))
                    .collect();
        }

        // ---- Overlays ----
        for overlay in &plan.overlays {
            match overlay {
                Overlay::Decisions => {
                    response.results.decisions = derive_decisions(&fused, req.limit);
                }
                Overlay::LawsAndViolations => {
                    let (active, violations) = derive_laws(&keyword_result.hits, &graph_result.hits, req.limit);
                    response.laws_active = active;
                    if req.include.contains(&IncludeField::Violations) {
                        response.results.violations = violations;
                    }
                }
                Overlay::GraphNeighbors => {
                    response.results.graph_neighbors = derive_graph_neighbors(&graph_result.hits, req.limit);
                }
                Overlay::SimilarTurns => {
                    response.results.similar_turns = derive_similar_turns(&fused, req.limit);
                }
            }
        }

        let used_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        response.budget = BudgetReport {
            used_ms,
            cap_ms: req.budget_ms,
            cache: "miss".to_string(),
        };
        // For law_check the spec is explicit: only violations surface
        // by default — strip snippets before returning.
        if matches!(req.intent, Intent::LawCheck) {
            response.results.snippets.clear();
            response.results.decisions.clear();
            response.results.graph_neighbors.clear();
            response.results.similar_turns.clear();
        }
        response
    }
}

/// Outcome returned by one lane runner. Captures the elapsed time +
/// hit list + classified error string so the orchestrator can fold
/// it into the response without re-throwing.
#[derive(Debug, Default, Clone)]
struct LaneOutcome {
    hits: Vec<LaneHit>,
    elapsed_ms: Option<u64>,
    error: Option<String>,
}

async fn run_vector_lane(
    lane: Arc<dyn VectorLane>,
    plan: &Plan,
    budget: Duration,
) -> LaneOutcome {
    if plan.vectors.is_empty() {
        return LaneOutcome::default();
    }
    let started = Instant::now();
    let mut hits: Vec<LaneHit> = Vec::new();
    let mut error: Option<String> = None;
    for req in &plan.vectors {
        match tokio::time::timeout(budget, lane.search(req)).await {
            Ok(Ok(mut h)) => hits.append(&mut h),
            Ok(Err(e)) => {
                error = Some(format!("{e}"));
                break;
            }
            Err(_) => {
                error = Some("budget exceeded".into());
                break;
            }
        }
    }
    LaneOutcome {
        hits,
        elapsed_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        error,
    }
}

async fn run_keyword_lane(
    lane: Arc<dyn KeywordLane>,
    plan: &Plan,
    budget: Duration,
) -> LaneOutcome {
    if plan.keywords.is_empty() {
        return LaneOutcome::default();
    }
    let started = Instant::now();
    let mut hits: Vec<LaneHit> = Vec::new();
    let mut error: Option<String> = None;
    for req in &plan.keywords {
        match tokio::time::timeout(budget, lane.search(req)).await {
            Ok(Ok(mut h)) => hits.append(&mut h),
            Ok(Err(e)) => {
                error = Some(format!("{e}"));
                break;
            }
            Err(_) => {
                error = Some("budget exceeded".into());
                break;
            }
        }
    }
    LaneOutcome {
        hits,
        elapsed_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        error,
    }
}

async fn run_graph_lane(
    lane: Arc<dyn GraphLane>,
    plan: &Plan,
    budget: Duration,
) -> LaneOutcome {
    if plan.graphs.is_empty() {
        return LaneOutcome::default();
    }
    let started = Instant::now();
    let mut hits: Vec<LaneHit> = Vec::new();
    let mut error: Option<String> = None;
    for req in &plan.graphs {
        match tokio::time::timeout(budget, lane.query(req)).await {
            Ok(Ok(mut h)) => hits.append(&mut h),
            Ok(Err(e)) => {
                error = Some(format!("{e}"));
                break;
            }
            Err(_) => {
                error = Some("budget exceeded".into());
                break;
            }
        }
    }
    LaneOutcome {
        hits,
        elapsed_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        error,
    }
}

async fn futures_join_three<A, B, C>(a: A, b: B, c: C) -> (LaneOutcome, LaneOutcome, LaneOutcome)
where
    A: std::future::Future<Output = LaneOutcome>,
    B: std::future::Future<Output = LaneOutcome>,
    C: std::future::Future<Output = LaneOutcome>,
{
    tokio::join!(a, b, c)
}

fn snippet_from_hit(rank: usize, hit: &LaneHit) -> Snippet {
    Snippet {
        rank,
        source: lane_label(hit),
        collection: hit.extras.get("collection").and_then(|v| v.as_str()).map(String::from),
        repo: hit.repo.clone(),
        path: hit.path.clone(),
        symbol: hit.symbol.clone(),
        content_hash: hit.content_hash.clone(),
        text: hit.text.clone(),
        score: hit.score,
        why: hit.extras.get("why").and_then(|v| v.as_str()).map(String::from),
    }
}

fn lane_label(hit: &LaneHit) -> String {
    hit.extras
        .get("source")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| "vector".to_string())
}

fn derive_decisions(fused: &[LaneHit], limit: usize) -> Vec<DecisionRef> {
    fused
        .iter()
        .filter_map(|h| {
            let id = h.extras.get("decision_id").and_then(|v| v.as_str())?;
            Some(DecisionRef {
                rank: 0,
                id: id.to_string(),
                title: h.symbol.clone().unwrap_or_else(|| h.text.clone()),
                status: h
                    .extras
                    .get("decision_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("proposed")
                    .to_string(),
                ts: h.ts,
                score: h.score,
                links: Vec::new(),
            })
        })
        .take(limit)
        .enumerate()
        .map(|(i, mut d)| {
            d.rank = i + 1;
            d
        })
        .collect()
}

fn derive_laws(
    keyword_hits: &[LaneHit],
    graph_hits: &[LaneHit],
    limit: usize,
) -> (Vec<LawRef>, Vec<ViolationRef>) {
    let mut active: Vec<LawRef> = Vec::new();
    let mut violations: Vec<ViolationRef> = Vec::new();
    for h in keyword_hits.iter().chain(graph_hits.iter()) {
        if let Some(law_id) = h.extras.get("law_id").and_then(|v| v.as_str()) {
            active.push(LawRef {
                id: law_id.to_string(),
                severity: h.severity.clone().unwrap_or_else(|| "info".into()),
                title: h.symbol.clone().unwrap_or_else(|| h.text.clone()),
            });
        }
        if let Some(violation_id) = h.extras.get("violation_id").and_then(|v| v.as_str()) {
            violations.push(ViolationRef {
                id: violation_id.to_string(),
                law_id: h
                    .extras
                    .get("law_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("LAW-???")
                    .to_string(),
                severity: h.severity.clone().unwrap_or_else(|| "info".into()),
                message: h.text.clone(),
                observed_in: h
                    .extras
                    .get("observed_in")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    active.sort_by(|a, b| a.id.cmp(&b.id));
    active.dedup_by(|a, b| a.id == b.id);
    active.truncate(limit);
    violations.truncate(limit);
    (active, violations)
}

fn derive_graph_neighbors(graph_hits: &[LaneHit], limit: usize) -> Vec<GraphNeighbor> {
    graph_hits
        .iter()
        .filter_map(|h| {
            let from = h.extras.get("edge_from").and_then(|v| v.as_str())?;
            let to = h.extras.get("edge_to").and_then(|v| v.as_str())?;
            let relation = h
                .extras
                .get("edge_type")
                .and_then(|v| v.as_str())
                .unwrap_or("RELATED");
            let hops = h
                .extras
                .get("hops")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u8;
            Some(GraphNeighbor {
                from: from.to_string(),
                relation: relation.to_string(),
                to: to.to_string(),
                hops,
            })
        })
        .take(limit)
        .collect()
}

fn derive_similar_turns(fused: &[LaneHit], limit: usize) -> Vec<SimilarTurn> {
    fused
        .iter()
        .filter_map(|h| {
            let turn_id = h.extras.get("turn_id").and_then(|v| v.as_str())?;
            Some(SimilarTurn {
                turn_id: turn_id.to_string(),
                ts: h.ts,
                model: h
                    .extras
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                summary: h
                    .extras
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| h.text.clone()),
                score: h.score,
            })
        })
        .take(limit)
        .collect()
}

/// Helper for tests that need an empty `extras` populated with a
/// known source label.
#[doc(hidden)]
pub fn extras_with_source(label: &str) -> crate::types::Props {
    let mut m = crate::types::Props::new();
    m.insert("source".to_string(), json!(label));
    m
}
