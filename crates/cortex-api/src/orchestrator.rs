//! Query orchestrator — parallel fan-out + RRF fusion + overlays.
//!
//! Spec 11 §Fan-out + fusion: every plan executes its lanes in
//! parallel under tokio, each carrying its own sub-budget. The
//! orchestrator then fuses with RRF, attaches the requested
//! overlays, and packages the response. Total budget enforcement
//! flips `debug.truncated = true` when stragglers are dropped.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::fusion::{rrf_fuse, FusionConfig};
use crate::lanes::{is_collection_missing_marker, GraphLane, KeywordLane, LaneHit, VectorLane};
use crate::query_rewrite::{PassthroughRewriter, QueryRewriter, RewrittenQuery};
use crate::strategies::{build_plan, Overlay, Plan};
use crate::types::{
    empty_response, BudgetReport, DebugNote, DecisionRef, GraphNeighbor, IncludeField, Intent,
    LawRef, QueryRequest, QueryResponse, SimilarTurn, Snippet, ViolationRef,
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
    /// Phase6c — score-aware RRF tuning. `Default::default` keeps
    /// the spec-11 documented defaults; the live binary overrides
    /// via `with_fusion(FusionConfig::new(env_alpha, env_k))` at
    /// boot so tests, in-tree fixtures, and the dashboard helper
    /// don't have to thread the config through their own
    /// constructors. Phase11i §3.6 wraps the value in
    /// `Arc<RwLock<…>>` so the SIGHUP-driven reload path can swap
    /// the active config in place; in-flight requests pick up the
    /// new snapshot on their next `current_fusion()` read.
    pub fusion: Arc<RwLock<FusionConfig>>,
    /// Phase6f — pre-fan-out query rewriter. Defaults to
    /// [`PassthroughRewriter`] so existing constructors and tests
    /// reproduce today's behaviour. The live binary swaps in
    /// [`crate::query_rewrite::NounPhraseRewriter`] (default) or
    /// [`crate::query_rewrite::SonnetRewriter`] based on
    /// `CORTEX_QUERY_REWRITER`.
    pub rewriter: Arc<dyn QueryRewriter>,
}

impl Orchestrator {
    /// Build a new orchestrator wrapping the three lane clients.
    /// Phase6c default fusion config is `alpha = 0.7`, `k = 60`;
    /// override with [`Orchestrator::with_fusion`].
    pub fn new(
        vector: Arc<dyn VectorLane>,
        keyword: Arc<dyn KeywordLane>,
        graph: Arc<dyn GraphLane>,
    ) -> Self {
        Self {
            vector,
            keyword,
            graph,
            fusion: Arc::new(RwLock::new(FusionConfig::default())),
            rewriter: Arc::new(PassthroughRewriter),
        }
    }

    /// Phase6c — replace the default fusion config with one parsed
    /// from `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` (or any test
    /// override). Builder method so existing
    /// [`Orchestrator::new`] callers stay unchanged. Phase11i §3.6
    /// stamps the new value into the live `RwLock` so any clones
    /// created downstream observe the same snapshot.
    pub fn with_fusion(self, fusion: FusionConfig) -> Self {
        *self.fusion.write().expect("fusion lock poisoned") = fusion;
        self
    }

    /// Phase11i §3.6 — hot-replace the live fusion config (SIGHUP
    /// reload path). Existing clones share the same `Arc<RwLock>`
    /// so the new value is immediately visible to every in-flight
    /// orchestrator handle.
    pub fn replace_fusion(&self, fusion: FusionConfig) {
        *self.fusion.write().expect("fusion lock poisoned") = fusion;
    }

    /// Phase11i §3.6 — clone the current fusion snapshot. Cheap
    /// (single read lock) and used by every per-request fuse call
    /// + the audit envelope builder so the stamped values reflect
    ///   whatever was loaded most recently.
    pub fn current_fusion(&self) -> FusionConfig {
        self.fusion.read().expect("fusion lock poisoned").clone()
    }

    /// Phase6f — install a query rewriter that fires once before
    /// per-lane fan-out. Builder method so existing
    /// [`Orchestrator::new`] callers stay unchanged; default is
    /// [`PassthroughRewriter`] (today's behaviour).
    pub fn with_rewriter(mut self, rewriter: Arc<dyn QueryRewriter>) -> Self {
        self.rewriter = rewriter;
        self
    }

    /// Run the request end-to-end and return the response paired
    /// with the [`RewrittenQuery`] the rewriter produced (so the
    /// caller can stamp the audit envelope). The caller decides
    /// whether to short-circuit on a cache hit before invoking this.
    pub async fn run(&self, req: &QueryRequest) -> (QueryResponse, RewrittenQuery) {
        let started = Instant::now();

        // Phase6f §4.1 — rewrite the prompt once, then patch each
        // per-lane request's `query` field with the appropriate
        // rewritten string. The rewriter's `Result` collapses to a
        // passthrough on failure so a flaky upstream never fails the
        // user-facing call here.
        let rewritten = match self.rewriter.rewrite(&req.query, req.intent).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "query rewriter returned an error; falling back to passthrough"
                );
                RewrittenQuery::passthrough(&req.query)
            }
        };

        let mut plan = build_plan(req);
        for v in plan.vectors.iter_mut() {
            v.query = rewritten.vector_query.clone();
        }
        for k in plan.keywords.iter_mut() {
            k.query = rewritten.keyword_query.clone();
        }
        // The graph lane today binds the original prompt as the
        // `query` Cypher param (most templates ignore it; the few
        // that match against text use the noun-phrase form). We
        // stamp the vector_query for consistency — same shape as
        // the embedding lane sees.
        for g in plan.graphs.iter_mut() {
            if let Some(obj) = g.params.as_object_mut() {
                obj.insert(
                    "query".to_string(),
                    serde_json::Value::String(rewritten.vector_query.clone()),
                );
            }
        }
        let plan = plan;
        let total_budget = Duration::from_millis(req.budget_ms.max(1));
        let split = plan.split_pct;

        let vector_budget = Duration::from_millis(req.budget_ms * split.vector as u64 / 100)
            .max(Duration::from_millis(1));
        let keyword_budget = Duration::from_millis(req.budget_ms * split.keyword as u64 / 100)
            .max(Duration::from_millis(1));
        let graph_budget = Duration::from_millis(req.budget_ms * split.graph as u64 / 100)
            .max(Duration::from_millis(1));

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
        let (mut vector_result, mut keyword_result, mut graph_result, truncated) =
            match total_fut.await {
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

        // Phase11e §3 — split the synthetic missing-collection
        // markers out of each lane's hit set BEFORE fusion (so RRF
        // never sees them) and translate them into structured
        // `DebugInfo.notes` entries. The lanes only emit these
        // when `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1`; when
        // unset, every hit list passes through unchanged.
        partition_collection_missing(&mut vector_result.hits, "vector", &mut response.debug.notes);
        partition_collection_missing(
            &mut keyword_result.hits,
            "keyword",
            &mut response.debug.notes,
        );
        partition_collection_missing(&mut graph_result.hits, "graph", &mut response.debug.notes);

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
        let fusion_snapshot = self.current_fusion();
        let mut fused = rrf_fuse(
            vec![
                vector_result.hits.clone(),
                keyword_result.hits.clone(),
                graph_result.hits.clone(),
            ],
            &fusion_snapshot,
        );
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
            response.results.snippets = fused
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
                    let (active, violations) =
                        derive_laws(&keyword_result.hits, &graph_result.hits, req.limit);
                    response.laws_active = active;
                    if req.include.contains(&IncludeField::Violations) {
                        response.results.violations = violations;
                    }
                }
                Overlay::GraphNeighbors => {
                    response.results.graph_neighbors =
                        derive_graph_neighbors(&graph_result.hits, req.limit);
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
        (response, rewritten)
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

/// Phase11e §3 — strip every synthetic missing-collection marker
/// out of `hits` and append a `DebugNote` per marker to `notes`.
/// Must run BEFORE fusion so the synthetic doc_ids never reach
/// the renderer / overlays. Idempotent and safe on lanes that
/// emit no markers (the env switch is off — the common path).
fn partition_collection_missing(
    hits: &mut Vec<LaneHit>,
    lane: &'static str,
    notes: &mut Vec<DebugNote>,
) {
    let mut kept: Vec<LaneHit> = Vec::with_capacity(hits.len());
    for hit in std::mem::take(hits) {
        if is_collection_missing_marker(&hit) {
            let collection = hit
                .extras
                .get("__collection_missing")
                .and_then(|v| v.as_str())
                .map(String::from);
            let display = collection.as_deref().unwrap_or("(unknown)");
            notes.push(DebugNote {
                lane: lane.to_string(),
                kind: "collection_missing".to_string(),
                collection: collection.clone(),
                message: format!(
                    "{lane} lane: collection / index {display} is not present on the live backend"
                ),
            });
        } else {
            kept.push(hit);
        }
    }
    *hits = kept;
}

async fn run_vector_lane(lane: Arc<dyn VectorLane>, plan: &Plan, budget: Duration) -> LaneOutcome {
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

async fn run_graph_lane(lane: Arc<dyn GraphLane>, plan: &Plan, budget: Duration) -> LaneOutcome {
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

/// Kind labels the meili / vectorizer lanes stamp into
/// `LaneHit.symbol` for back-compat with overlay derivations. They
/// must NOT reach the wire `Snippet.symbol` field — spec 11 says
/// that field carries the Tree-sitter symbol (for code) or H1 (for
/// docs), not the event kind. The pre-thinking renderer formats
/// `path:symbol`, and surfacing `path:artifact` / `path:turn`
/// produced the audit-flagged `Cortex/.../types.rs:artifact`
/// rendering. Filter the kind labels here so callers see the real
/// symbol or `None`.
const SYMBOL_KIND_LABELS: &[&str] = &[
    "artifact",
    "turn",
    "tool_call",
    "agent_call",
    "decision",
    "analysis",
    "memory",
    "law_violation",
];

fn snippet_from_hit(rank: usize, hit: &LaneHit) -> Snippet {
    // phase10b §2.1 — strip kind labels from the wire `symbol`
    // field. Decisions get the curated `decision_title` extras
    // (phase10a) when available; everything else degrades to
    // `None` and the bundle renderer falls back to `repo/path`.
    let symbol = hit
        .extras
        .get("decision_title")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            hit.symbol
                .as_deref()
                .filter(|s| !SYMBOL_KIND_LABELS.contains(s))
                .map(String::from)
        });
    let body_truncated = hit
        .extras
        .get("body_truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Snippet {
        rank,
        source: lane_label(hit),
        collection: hit
            .extras
            .get("collection")
            .and_then(|v| v.as_str())
            .map(String::from),
        repo: hit.repo.clone(),
        path: hit.path.clone(),
        symbol,
        content_hash: hit.content_hash.clone(),
        text: hit.text.clone(),
        body_truncated,
        score: hit.score,
        why: hit
            .extras
            .get("why")
            .and_then(|v| v.as_str())
            .map(String::from),
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
            // phase10a §1.2 — prefer the lane's stamped
            // `decision_title` over the kind label that
            // `LaneHit.symbol` carries today (the meili projection
            // sets `symbol = doc.kind`, so without this fallback
            // every decision overlay rendered as the literal string
            // `"decision"`).
            let title = h
                .extras
                .get("decision_title")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| h.symbol.clone().filter(|s| s != "decision"))
                .unwrap_or_else(|| h.text.clone());
            let rationale_excerpt = h
                .extras
                .get("rationale_excerpt")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            Some(DecisionRef {
                rank: 0,
                id: id.to_string(),
                title,
                rationale_excerpt,
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
            let hops = h.extras.get("hops").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
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
                outcome: h
                    .extras
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .map(String::from),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::LaneHit;

    fn hit_with(symbol: Option<&str>, text: &str) -> LaneHit {
        LaneHit {
            doc_id: "h1".into(),
            text: text.into(),
            repo: Some("Cortex".into()),
            path: Some("crates/cortex-api/src/types.rs".into()),
            symbol: symbol.map(String::from),
            content_hash: None,
            score: 0.5,
            ts: 0,
            severity: None,
            extras: crate::types::Props::new(),
        }
    }

    // Phase11e §3 — synthetic missing-collection markers must
    // never reach RRF / the renderer. The orchestrator splits
    // them into `debug.notes` BEFORE fusion. These tests pin
    // the partitioning to the lane label and the structured
    // shape every consumer (dashboard, MCP) reads.
    #[test]
    fn partition_collection_missing_strips_marker_and_emits_note() {
        let real = hit_with(None, "real chunk body");
        let marker = crate::lanes::collection_missing_marker("cortex-cortex-decisions");
        let mut hits = vec![real.clone(), marker];
        let mut notes: Vec<DebugNote> = Vec::new();
        super::partition_collection_missing(&mut hits, "vector", &mut notes);
        assert_eq!(hits.len(), 1, "synthetic marker must be stripped");
        assert_eq!(hits[0].doc_id, real.doc_id);
        assert_eq!(
            notes.len(),
            1,
            "exactly one DebugNote per missing collection"
        );
        let note = &notes[0];
        assert_eq!(note.lane, "vector");
        assert_eq!(note.kind, "collection_missing");
        assert_eq!(note.collection.as_deref(), Some("cortex-cortex-decisions"));
        assert!(note.message.contains("cortex-cortex-decisions"));
    }

    #[test]
    fn partition_collection_missing_is_idempotent_when_no_marker_present() {
        let real_a = hit_with(None, "alpha");
        let real_b = hit_with(None, "beta");
        let mut hits = vec![real_a.clone(), real_b.clone()];
        let mut notes: Vec<DebugNote> = Vec::new();
        super::partition_collection_missing(&mut hits, "keyword", &mut notes);
        assert_eq!(hits.len(), 2, "real hits round-trip unchanged");
        assert!(notes.is_empty(), "no markers ⇒ no notes appended");
    }

    #[test]
    fn partition_collection_missing_handles_multiple_markers_per_lane() {
        let real = hit_with(None, "real");
        let marker_a = crate::lanes::collection_missing_marker("cortex-cortex-turns");
        let marker_b = crate::lanes::collection_missing_marker("cortex-rulebook-decisions");
        let mut hits = vec![marker_a, real.clone(), marker_b];
        let mut notes: Vec<DebugNote> = Vec::new();
        super::partition_collection_missing(&mut hits, "vector", &mut notes);
        assert_eq!(hits.len(), 1);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].lane, "vector");
        assert_eq!(notes[1].lane, "vector");
        let collections: Vec<_> = notes.iter().filter_map(|n| n.collection.clone()).collect();
        assert!(collections.contains(&"cortex-cortex-turns".to_string()));
        assert!(collections.contains(&"cortex-rulebook-decisions".to_string()));
    }

    #[test]
    fn snippet_strips_kind_labels_from_symbol_field() {
        // phase10b §2.1 — every kind label the meili / vectorizer
        // lane stamps into `LaneHit.symbol` for back-compat MUST
        // be filtered before reaching `Snippet.symbol`. The audit
        // logged `Cortex/.../types.rs:artifact` headers because
        // the formatter rendered `path:symbol` literally — this
        // test pins the kind-label filter so a future regression
        // surfaces immediately.
        for kind in SYMBOL_KIND_LABELS {
            let hit = hit_with(Some(kind), "real body content");
            let snippet = snippet_from_hit(1, &hit);
            assert_eq!(
                snippet.symbol, None,
                "kind label `{kind}` must not reach Snippet.symbol"
            );
        }
    }

    #[test]
    fn snippet_keeps_real_symbols_distinct_from_kind_labels() {
        let hit = hit_with(Some("hnsw_search"), "pub fn hnsw_search() { ... }");
        let snippet = snippet_from_hit(1, &hit);
        assert_eq!(snippet.symbol.as_deref(), Some("hnsw_search"));
        assert_eq!(snippet.text, "pub fn hnsw_search() { ... }");
        assert!(!snippet.body_truncated);
    }

    #[test]
    fn snippet_propagates_body_truncated_extras_flag() {
        // phase10b §2.2 — when the lane stamped
        // `extras.body_truncated = true`, the wire `Snippet`
        // carries the same flag so the bundle renderer can
        // collapse to a header-only line.
        let mut hit = hit_with(None, "");
        hit.extras.insert("body_truncated".to_string(), json!(true));
        let snippet = snippet_from_hit(1, &hit);
        assert!(snippet.body_truncated);
        assert!(snippet.text.is_empty());
    }

    #[test]
    fn snippet_pulls_decision_title_extras_when_present() {
        // phase10a stamped `decision_title` for decisions; the
        // snippet projector prefers it over the kind-stripped
        // symbol so the bundle header carries the real ADR title.
        let mut hit = hit_with(Some("decision"), "Adopt RRF fusion (rationale)");
        hit.extras.insert(
            "decision_title".to_string(),
            json!("DEC-0042 Adopt RRF fusion"),
        );
        let snippet = snippet_from_hit(1, &hit);
        assert_eq!(snippet.symbol.as_deref(), Some("DEC-0042 Adopt RRF fusion"));
    }
}
