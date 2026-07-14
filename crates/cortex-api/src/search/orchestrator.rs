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
use crate::lanes::{
    is_collection_missing_marker, GraphLane, GraphRequest, KeywordLane, LaneHit, VectorLane,
};
use crate::query_rewrite::{PassthroughRewriter, QueryRewriter, RewrittenQuery};
use crate::strategies::{build_plan, Overlay, Plan};
use crate::types::{
    empty_response, BudgetReport, DebugNote, DecisionRef, GraphNeighbor, IncludeField, Intent,
    LawRef, QueryRequest, QueryResponse, SimilarTurn, Snippet, ViolationRef,
};
use cortex_config::{
    AccessControlConfig, CrossProjectConfig, RerankerConfig, TemporalConfig, VerifyConfig,
};
use cortex_workers::rerank::{Candidate as RerankCandidate, Reranker};
use cortex_workers::temporal::classifier::{
    classify, Action as TemporalAction, Candidate as TemporalCandidate, IncludeFlags,
    TemporalConfig as ClassifierConfig,
};
use cortex_workers::verify::{verify_symbol, SymbolVerdict};

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
    /// Phase18 §3.3 — bitemporal classifier config. Mirrors the
    /// fusion handle (Arc<RwLock<_>>) so SIGHUP-driven reloads swap
    /// the snapshot in place. Default leaves the classifier
    /// enabled with the design.md §2.2 tuning; the live binary
    /// overrides via `with_temporal` at boot.
    pub temporal: Arc<RwLock<TemporalConfig>>,
    /// Phase18 §5.2 — cross-project propagation config. Mirrors the
    /// temporal handle (Arc<RwLock<_>>) so SIGHUP-driven reloads swap
    /// the snapshot in place. Default OFF per ADR-020; the live binary
    /// overrides via `with_cross_project` at boot.
    pub cross_project: Arc<RwLock<CrossProjectConfig>>,
    /// Phase21 §7.2 — Bell-LaPadula access-control posture snapshot.
    /// Mirrors temporal/cross_project (Arc<RwLock<_>>) so SIGHUP reloads
    /// can hot-swap the config. Default OFF (ADR-1.3); when disabled every
    /// enforcement point in this orchestrator is a no-op pass-through.
    pub access_control: Arc<RwLock<AccessControlConfig>>,
    /// Phase6f — pre-fan-out query rewriter. Defaults to
    /// [`PassthroughRewriter`] so existing constructors and tests
    /// reproduce today's behaviour. The live binary swaps in
    /// [`crate::query_rewrite::NounPhraseRewriter`] (default) or
    /// [`crate::query_rewrite::SonnetRewriter`] based on
    /// `CORTEX_QUERY_REWRITER`.
    pub rewriter: Arc<dyn QueryRewriter>,
    /// Phase18 §7.2 — optional Prometheus metrics handle. When `Some`,
    /// the temporal / branch / cross-project wedges record counters
    /// into this registry. `None` (the default) is a no-op so all
    /// existing `Orchestrator::new` call sites compile unchanged.
    pub temporal_metrics: Option<Arc<crate::TemporalMetrics>>,
    /// Phase21 §8.2 — optional ACL decision metrics handle. When `Some`,
    /// `apply_acl_wedge` pushes every decision into the aggregating store
    /// so the dashboard can surface denial rate + top denied principals.
    pub acl_metrics: Option<Arc<crate::AclMetrics>>,
    /// Phase17 §2.3 — optional cross-encoder reranker. When `Some` and
    /// `reranker_cfg.enabled`, the top-`top_k_input` fused candidates are
    /// sent to the TEI service before the dedupe/truncate step.
    /// `None` (the default) bypasses reranking entirely so existing
    /// constructors compile unchanged.
    pub reranker: Option<Arc<dyn Reranker>>,
    /// Phase17 §2.4 — reranker knob snapshot (cloned from config at boot).
    pub reranker_cfg: RerankerConfig,
    /// Phase17 §3.5 — phantom-link verifier config.
    pub verify_cfg: VerifyConfig,
    /// Phase17 §3.5 — absolute workspace root for resolving repo-relative
    /// snippet paths. `None` disables filesystem verification even when
    /// `verify_cfg.symbols_enabled = true` (safe default for tests that
    /// don't have a real workspace on disk).
    pub workspace_root: Option<std::path::PathBuf>,
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
            temporal: Arc::new(RwLock::new(TemporalConfig::default())),
            cross_project: Arc::new(RwLock::new(CrossProjectConfig::default())),
            access_control: Arc::new(RwLock::new(AccessControlConfig::default())),
            rewriter: Arc::new(PassthroughRewriter),
            temporal_metrics: None,
            acl_metrics: None,
            reranker: None,
            reranker_cfg: RerankerConfig::default(),
            verify_cfg: VerifyConfig::default(),
            workspace_root: None,
        }
    }

    /// Phase18 §3.3 — install a non-default temporal config at boot.
    /// Mirrors [`Orchestrator::with_fusion`] so the binary can
    /// thread `relevance.toml`'s `[temporal]` block into the live
    /// handle.
    pub fn with_temporal(self, temporal: TemporalConfig) -> Self {
        *self.temporal.write().expect("temporal lock poisoned") = temporal;
        self
    }

    /// Phase18 §3.3 — hot-replace the live temporal config (SIGHUP
    /// reload path; matches `replace_fusion`).
    pub fn replace_temporal(&self, temporal: TemporalConfig) {
        *self.temporal.write().expect("temporal lock poisoned") = temporal;
    }

    /// Phase18 §3.3 — clone the current temporal snapshot. Cheap
    /// (single read lock) so the wedge can call it once per request.
    pub fn current_temporal(&self) -> TemporalConfig {
        self.temporal
            .read()
            .expect("temporal lock poisoned")
            .clone()
    }

    /// Phase18 §5.2 — install a non-default cross-project config at boot.
    /// Mirrors [`Orchestrator::with_temporal`] so the binary can
    /// thread `relevance.toml`'s `[cross_project]` block into the live
    /// handle.
    pub fn with_cross_project(self, cfg: CrossProjectConfig) -> Self {
        *self
            .cross_project
            .write()
            .expect("cross_project lock poisoned") = cfg;
        self
    }

    /// Phase18 §5.2 — hot-replace the live cross-project config (SIGHUP
    /// reload path; matches `replace_temporal`).
    pub fn replace_cross_project(&self, cfg: CrossProjectConfig) {
        *self
            .cross_project
            .write()
            .expect("cross_project lock poisoned") = cfg;
    }

    /// Phase18 §5.2 — clone the current cross-project snapshot. Cheap
    /// (single read lock) so the propagation helper can call it once
    /// per request.
    pub fn current_cross_project(&self) -> CrossProjectConfig {
        self.cross_project
            .read()
            .expect("cross_project lock poisoned")
            .clone()
    }

    /// Phase21 §7.2 — install a non-default access-control config at boot.
    /// Mirrors [`Orchestrator::with_temporal`] / `with_cross_project`.
    pub fn with_access_control(self, cfg: AccessControlConfig) -> Self {
        *self
            .access_control
            .write()
            .expect("access_control lock poisoned") = cfg;
        self
    }

    /// Phase21 §7.2 — hot-replace the live access-control config (SIGHUP
    /// reload path; matches `replace_temporal` / `replace_cross_project`).
    pub fn replace_access_control(&self, cfg: AccessControlConfig) {
        *self
            .access_control
            .write()
            .expect("access_control lock poisoned") = cfg;
    }

    /// Phase21 §7.2 — clone the current access-control snapshot. Cheap
    /// (single read lock); called once per request at every enforcement point.
    pub fn current_access_control(&self) -> AccessControlConfig {
        self.access_control
            .read()
            .expect("access_control lock poisoned")
            .clone()
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

    /// Phase18 §7.2 — attach a `TemporalMetrics` handle. When set,
    /// every temporal/branch/cross-project wedge records counters.
    /// Omitting this builder leaves `temporal_metrics = None` and all
    /// record calls become no-ops (existing tests are unaffected).
    pub fn with_temporal_metrics(mut self, m: Arc<crate::TemporalMetrics>) -> Self {
        self.temporal_metrics = Some(m);
        self
    }

    /// Phase21 §8.2 — attach an `AclMetrics` handle. When set, every
    /// ACL decision in `apply_acl_wedge` is recorded into the aggregating
    /// store. Omitting this builder leaves `acl_metrics = None` and all
    /// existing constructors compile unchanged (no recording = no-op).
    pub fn with_acl_metrics(mut self, m: Arc<crate::AclMetrics>) -> Self {
        self.acl_metrics = Some(m);
        self
    }

    /// Phase17 §2.3 — attach a cross-encoder reranker. The live binary
    /// wires in [`cortex_workers::rerank::bge_v2m3::BgeRerankerV2M3`]
    /// when `CORTEX_RERANKER_ENDPOINT` is set. Omitting this builder
    /// leaves `reranker = None` and all existing constructors compile
    /// unchanged (no reranking = pass-through).
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>, cfg: RerankerConfig) -> Self {
        self.reranker = Some(reranker);
        self.reranker_cfg = cfg;
        self
    }

    /// Phase17 §3.5 — configure the phantom-link verifier. The live binary
    /// passes the workspace root and `VerifyConfig` from `Config::verify` at
    /// boot. Omitting this builder leaves `symbols_enabled = true` (default)
    /// but `workspace_root = None`, which disables filesystem verification so
    /// tests that don't mount a real workspace are unaffected.
    pub fn with_verify(mut self, cfg: VerifyConfig, root: std::path::PathBuf) -> Self {
        self.verify_cfg = cfg;
        self.workspace_root = Some(root);
        self
    }

    /// Run the request end-to-end and return the response paired
    /// with the [`RewrittenQuery`] the rewriter produced (so the
    /// caller can stamp the audit envelope). The caller decides
    /// whether to short-circuit on a cache hit before invoking this.
    ///
    /// ACL-unaware wrapper — all lane requests use scope-only filters.
    /// Use [`Self::run_with_principal`] when Bell-LaPadula filtering is
    /// required.
    pub async fn run(&self, req: &QueryRequest) -> (QueryResponse, RewrittenQuery) {
        self.run_with_acl(req, None, None).await
    }

    /// Phase21 §5.2 — ACL-aware entry point. Resolves the principal's
    /// clearance + compartments, stamps [`crate::lanes::AclContext`] onto
    /// every keyword-lane request, and delegates to [`Self::run_with_acl`].
    ///
    /// Super-admin / acl_admin principals bypass the backend filter (they
    /// are unrestricted at the application layer; only their level gate
    /// applies).  For all other principals the Bell-LaPadula filter is
    /// injected before each lane search so the Meili server enforces it
    /// server-side (performance primary; the post-fusion wedge §5.5 is
    /// the authoritative gate).
    pub async fn run_with_principal(
        &self,
        req: &QueryRequest,
        principal: &crate::security::principal::Principal,
    ) -> (QueryResponse, RewrittenQuery) {
        // Phase21 §7.2 — when AC is disabled every enforcement point is a
        // no-op; delegate straight to the unconstrained runner so classified
        // rows are visible to every principal (backward-compat, ADR-1.3).
        if !self.current_access_control().enabled {
            return self.run(req).await;
        }
        if principal.is_acl_admin() {
            return self.run(req).await;
        }
        let acl = crate::lanes::AclContext {
            clearance_level: principal.clearance_level,
            compartment_grants: principal.compartment_grants.clone(),
        };
        self.run_with_acl(req, Some(acl), Some(principal.id.as_str()))
            .await
    }

    /// Private inner runner. `acl_override` is stamped onto every keyword
    /// lane request so the Meili backend can apply the Bell-LaPadula filter
    /// server-side. `None` = passthrough (scope-only filters, backward compat).
    /// `principal_id` threads through to the post-fusion wedge for audit logging.
    async fn run_with_acl(
        &self,
        req: &QueryRequest,
        acl_override: Option<crate::lanes::AclContext>,
        principal_id: Option<&str>,
    ) -> (QueryResponse, RewrittenQuery) {
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
        // Phase21 §5.2 / §5.3 / §5.4 — stamp ACL context onto keyword,
        // vector, and graph lane requests. Keyword requests get server-side
        // filtering via `meili_acl_filter`; vector requests get client-side
        // filtering via `vectorizer_acl_matches`; graph requests get inline
        // Cypher WHERE injection via `nexus_acl_where_for_alias`.
        if let Some(acl) = &acl_override {
            for k in plan.keywords.iter_mut() {
                k.acl = Some(acl.clone());
            }
            for v in plan.vectors.iter_mut() {
                v.acl = Some(acl.clone());
            }
            for g in plan.graphs.iter_mut() {
                g.acl = Some(acl.clone());
            }
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

        // Phase22 P1 — the FastEmbed dense lane (all-MiniLM-L6-v2 embed
        // on the Vectorizer server + KNN over `cortex-{slug}-code/docs`)
        // measures 250–595ms on the live stack (the embed call is the
        // dominant, high-variance term). The spec-11 default split
        // (40% to vector) grants only ~200ms on the canonical
        // `budget_ms = 500`, so the lane erodes to `budget exceeded` on
        // most queries and the bundle silently degrades to keyword-only
        // — see the §P1 dense-validation harness + `query_explain`
        // dumps. Give the vector lane a 600ms floor so the p99 dense
        // call lands within budget on a healthy stack. Vector is the
        // dominant lane (keyword fans out in ~50ms in parallel), so the
        // floor is capped at the FULL `total_budget`, not `/2` like the
        // graph lane: spending the whole budget on the lane the caller
        // most wants is the honest semantics and never delays the
        // response beyond what the caller already authorised.
        // Operator-tuned splits that already grant ≥600ms keep the
        // computed value.
        let vector_budget = Duration::from_millis(req.budget_ms * split.vector as u64 / 100)
            .max(Duration::from_millis(600))
            .min(total_budget);
        let keyword_budget = Duration::from_millis(req.budget_ms * split.keyword as u64 / 100)
            .max(Duration::from_millis(1));
        // Phase20 §12.2 — the spec-11 default split (20% to graph)
        // gives the graph lane only ~100ms on the canonical
        // `budget_ms = 500`. Nexus 2.3.4's CONTAINS scan on the
        // Artifact label (16k+ nodes, no full-text index) takes
        // ~1600ms — a regression vs 2.2.0's property-keyed seek
        // (150–250ms on 137k nodes). Bump the floor to 2000ms so
        // the lane lands within budget on a healthy stack; match
        // the vector lane cap at `total_budget` (not `/ 2`) since
        // all three lanes fan out in parallel — the cap is an
        // early per-lane kill, not a serialisation barrier. When
        // the operator-tuned split already grants ≥2000ms the
        // computed value wins.
        let graph_budget = Duration::from_millis(req.budget_ms * split.graph as u64 / 100)
            .max(Duration::from_millis(2000))
            .min(total_budget);

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
        // `overlay.source = LaneSource::Keyword`. ADR-011 retired
        // the stringly-typed `extras["source"] = "keyword"` check
        // here; the typed enum makes a future regression in any
        // new lane implementation a compile error rather than a
        // runtime assert.
        debug_assert!(
            keyword_result
                .hits
                .iter()
                .all(|h| h.overlay.source == crate::lanes::LaneSource::Keyword),
            "keyword lane returned a hit without overlay.source = Keyword"
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

        // Phase18 §3.3 — temporal classifier wedge. Runs after RRF
        // fusion and before the anchor-dedupe + truncate so dropped
        // hits never reach the snippet section, and demoted hits
        // are reordered by the multiplier before truncation. The
        // classifier reads the bitemporal axes
        // (`valid_from_unix` / `valid_to_unix` / `superseded_at_unix`
        // / `lifecycle`) off each `LaneHit::extras` map — projected
        // there by the Meili / Vectorizer lane builders per
        // phase18 §2.6. Hits that lack the axes (pre-migration
        // rows) flow through as VALID via the
        // `temporal::classifier::derive_state` defaults.
        let temporal_snapshot = self.current_temporal();
        if temporal_snapshot.enabled {
            let as_of_override = parse_as_of_to_unix(req.as_of.as_deref());
            let flag_overrides = IncludeFlags {
                include_history: req.include_history.unwrap_or(false),
                include_future: req.include_future.unwrap_or(false),
                include_branches: req.include_branches.unwrap_or(false),
            };
            let metrics = self.temporal_metrics.as_deref();
            apply_temporal_classifier(
                &mut fused,
                &temporal_snapshot,
                &response.query_id,
                as_of_override,
                flag_overrides,
                metrics,
            );
            // Phase18 §7.2 — per-request counters: one query + an
            // as-of bump when the caller pinned a historical instant.
            if let Some(m) = metrics {
                m.record_query();
                if as_of_override.is_some() {
                    m.record_as_of();
                }
            }
            // Phase18 §3.9 — branch_resolution envelope. Derives
            // `project` from the canonical scope.repo (every live
            // request resolves to a single repo at this point); the
            // ancestry chain defaults to `<project>:main` when the
            // request surface adds a caller-supplied branch (§4 P3).
            let project = req.scope.repo.as_deref().unwrap_or("cortex");
            emit_branch_resolution_envelope(
                &response.query_id,
                project,
                req.branch.as_deref(),
                metrics,
            );
        }

        // Phase18 §5.2 — cross-project propagation. Gated by the
        // §5.3 `CrossProjectConfig` (default OFF per ADR-020). When
        // enabled and the request names sibling `projects`, walk the
        // active project's `CROSS_PROJECT_REF` edges, classify each
        // discovered reference by the edge's temporal window, and
        // fuse the survivors in with `source_project` provenance.
        if self.current_cross_project().enabled {
            let as_of_override = parse_as_of_to_unix(req.as_of.as_deref());
            self.propagate_cross_project(&mut fused, req, as_of_override, &response.query_id)
                .await;
        }

        // Phase21 §5.5 — post-fusion ACL wedge. Drop any hit whose
        // `class_level`/`class_compartments` extras fail the
        // Bell-LaPadula predicate for the calling principal. This gate
        // runs AFTER cross-project propagation so propagated hits are
        // also filtered, and BEFORE the reranker so we don't score
        // hits that will be discarded.
        if let Some(acl) = &acl_override {
            apply_acl_wedge(
                &mut fused,
                acl,
                &response.query_id,
                principal_id,
                self.acl_metrics.as_deref(),
            );
        }

        // Phase17 §2.3 — cross-encoder reranker step (fail-open, §2.5).
        // Sends the top `top_k_input` fused candidates to the TEI
        // service and overwrites each hit's `score` with the returned
        // cross-encoder logit, then re-sorts descending. On timeout or
        // any HTTP error the pre-rerank order is kept intact and an
        // audit event is emitted with `reranker.fallback = true`.
        if let Some(reranker) = &self.reranker {
            if self.reranker_cfg.enabled {
                let top_k = self.reranker_cfg.top_k_input;
                let candidates: Vec<RerankCandidate> = fused
                    .iter()
                    .take(top_k)
                    .map(|h| RerankCandidate {
                        doc_id: h.doc_id.clone(),
                        text: h.text.clone(),
                    })
                    .collect();

                match reranker.score(&rewritten.vector_query, &candidates).await {
                    Ok(scores) => {
                        for (hit, score) in fused.iter_mut().zip(scores) {
                            hit.score = score as f64;
                        }
                        // Re-sort descending by the new scores from the
                        // cross-encoder (fusion order is no longer valid).
                        fused.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "reranker call failed; falling back to pre-rerank fusion order"
                        );
                        tracing::info!(
                            target: "cortex_audit",
                            event = "reranker.fallback",
                            reason = %err,
                            query_id = %response.query_id,
                        );
                    }
                }
            }
        }

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

            // Phase17 §3.5 — phantom-link verification pass.
            // Run only when `symbols_enabled` and a workspace root is
            // available; otherwise snippets flow through with
            // `verified = None` (not-run, not flagged).
            if self.verify_cfg.symbols_enabled {
                if let Some(root) = &self.workspace_root {
                    apply_phantom_link_verification(
                        &mut response.results.snippets,
                        root,
                        &self.verify_cfg.action,
                        &response.query_id,
                    );
                }
            }
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

    /// Phase18 §5.2 — walk `CROSS_PROJECT_REF` edges from the active
    /// project into the requested sibling projects, classify each
    /// discovered reference via the same temporal wedge, and fuse
    /// surviving hits (with `source_project` provenance) into `fused`.
    ///
    /// Returns immediately when `cross_project.enabled` is `false` or
    /// when `req.projects` is empty — no graph I/O in those cases.
    async fn propagate_cross_project(
        &self,
        fused: &mut Vec<LaneHit>,
        req: &QueryRequest,
        as_of_unix: Option<i64>,
        query_id: &str,
    ) {
        let cfg = self.current_cross_project();
        if !cfg.enabled {
            return;
        }
        let projects = req.projects.clone().unwrap_or_default();
        if projects.is_empty() {
            return;
        }
        let active = req.scope.repo.as_deref().unwrap_or("cortex").to_lowercase();
        let gr = GraphRequest {
            template: "cross_project_ref".into(),
            params: serde_json::json!({ "query": format!("{active}:main") }),
            max_hops: u8::try_from(cfg.max_hops).unwrap_or(1).max(1),
            scope: req.scope.clone(),
            acl: None,
        };
        let edges = match self.graph.query(&gr).await {
            Ok(e) => e,
            Err(err) => {
                tracing::event!(
                    target: "cortex_audit",
                    tracing::Level::INFO,
                    kind = "cross_project_propagation",
                    query_id = %query_id,
                    active_project = %active,
                    error = %err,
                );
                return;
            }
        };
        let discovered = edges.len();
        let projects_lower: Vec<String> = projects.iter().map(|p| p.to_ascii_lowercase()).collect();

        let now = as_of_unix.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
                .unwrap_or(0)
        });
        let flags = IncludeFlags {
            include_history: req.include_history.unwrap_or(false),
            include_future: req.include_future.unwrap_or(false),
            include_branches: req.include_branches.unwrap_or(false),
        };
        let classifier_cfg = classifier_config_from(&self.current_temporal());

        let mut kept_count = 0usize;
        let mut propagated: Vec<LaneHit> = Vec::new();

        for mut hit in edges {
            let source_project = hit
                .extras
                .get("source_project")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if !projects_lower.contains(&source_project) {
                continue;
            }
            kept_count += 1;

            // Convert RFC-3339 (or YYYY-MM-DD) valid_from / valid_to
            // strings to epoch seconds and stamp them as JSON numbers so
            // `candidate_from_hit` picks them up via `extras_as_unix`.
            if let Some(vf_str) = hit
                .extras
                .get("valid_from")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            {
                if let Some(unix) = rfc3339_to_unix(&vf_str) {
                    hit.extras.insert(
                        "valid_from_unix".to_string(),
                        serde_json::Value::Number(unix.into()),
                    );
                }
            }
            if let Some(vt_str) = hit
                .extras
                .get("valid_to")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            {
                if let Some(unix) = rfc3339_to_unix(&vt_str) {
                    hit.extras.insert(
                        "valid_to_unix".to_string(),
                        serde_json::Value::Number(unix.into()),
                    );
                }
            }

            let candidate = candidate_from_hit(&hit);
            let (_state, action) = classify(&candidate, now, flags, &classifier_cfg);
            match action {
                TemporalAction::Drop => continue,
                TemporalAction::Pass { multiplier } => {
                    hit.score *= f64::from(multiplier);
                }
                TemporalAction::Demote { factor } => {
                    hit.score *= f64::from(factor);
                }
            }
            propagated.push(hit);
        }

        let dropped = kept_count.saturating_sub(propagated.len());
        tracing::event!(
            target: "cortex_audit",
            tracing::Level::INFO,
            kind = "cross_project_propagation",
            query_id = %query_id,
            active_project = %active,
            requested = projects.len(),
            discovered = discovered,
            kept = kept_count,
            propagated = propagated.len(),
            dropped = dropped,
        );
        // Phase18 §7.2 — cross-project propagation counters.
        if let Some(m) = self.temporal_metrics.as_deref() {
            m.record_cross_project(discovered as u64, propagated.len() as u64);
        }

        fused.append(&mut propagated);
        // Stable sort by adjusted score so propagated hits interleave
        // by score — mirrors the sort in `apply_temporal_classifier`.
        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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

/// Phase18 §3.3 — translate the cortex-config snapshot into the
/// classifier's pure config struct (the workers crate intentionally
/// stays decoupled from `cortex-config` so the classifier remains
/// callable from non-API binaries).
fn classifier_config_from(snapshot: &TemporalConfig) -> ClassifierConfig {
    ClassifierConfig {
        temporal_boost: snapshot.temporal_boost,
        temporal_window_seconds: i64::from(snapshot.temporal_window_days) * 24 * 60 * 60,
        demote_factor: snapshot.demote_factor,
    }
}

/// Phase18 §3.3 — read a single `i64` epoch-second column from a
/// `LaneHit::extras` map. The Meili / Vectorizer projection stamps
/// the value as either an integer (`i64`) or a string (legacy
/// rows); both shapes round-trip through `as_i64` first, then a
/// numeric-string fallback. Absent / unparseable columns return
/// `None` so the classifier treats the row as a pre-migration
/// VALID hit instead of mis-dating it.
fn extras_as_unix(hit: &LaneHit, key: &str) -> Option<i64> {
    let v = hit.extras.get(key)?;
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        if let Ok(n) = s.parse::<i64>() {
            return Some(n);
        }
    }
    None
}

/// Phase18 §3.3 — Build a `Candidate` view of a fused hit. The
/// classifier is pure, so this helper does no I/O — it just maps
/// the `extras` map to the typed candidate the state machine
/// expects.
fn candidate_from_hit(hit: &LaneHit) -> TemporalCandidate {
    let lifecycle = hit
        .extras
        .get("lifecycle")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "active" => Some("active"),
            "deprecated" => Some("deprecated"),
            "abandoned" => Some("abandoned"),
            "archived" => Some("archived"),
            _ => None,
        });
    TemporalCandidate {
        valid_from_unix: extras_as_unix(hit, "valid_from_unix"),
        valid_to_unix: extras_as_unix(hit, "valid_to_unix"),
        superseded_at_unix: extras_as_unix(hit, "superseded_at_unix"),
        lifecycle,
    }
}

/// Phase18 §3.3 — wedge the temporal classifier between RRF fusion
/// and the anchor-dedupe. Iterates the fused candidate list, runs
/// `classify()` per hit, and applies the resulting action:
///
/// - `Drop`           → remove the hit from the candidate set.
/// - `Pass {mult}`    → `hit.score *= mult` (`mult = 1.0` for VALID).
/// - `Demote {factor}`→ `hit.score *= factor`.
///
/// After scoring the list is re-sorted descending by score so the
/// downstream truncate keeps the top-K post-classifier. The
/// `include_*` flags currently default to false (current-scope-only
/// retrieval) until the request surface adds the
/// `include_history` / `include_future` / `include_branches`
/// switches (phase18 §4 P3 on the request type).
fn apply_temporal_classifier(
    fused: &mut Vec<LaneHit>,
    snapshot: &TemporalConfig,
    query_id: &str,
    as_of_override: Option<i64>,
    flag_overrides: IncludeFlags,
    metrics: Option<&crate::TemporalMetrics>,
) {
    let classifier_cfg = classifier_config_from(snapshot);
    let flags = IncludeFlags {
        include_history: flag_overrides.include_history || snapshot.include_history_default,
        include_future: flag_overrides.include_future,
        include_branches: flag_overrides.include_branches,
    };
    // `as_of` defaults to the wall clock; the request surface
    // (phase18 §4.4 / §4.5) lets the caller pin a historical
    // instant via the body's `as_of` field. The orchestrator
    // parses the RFC-3339 / day-precision string at the call
    // site and threads the resulting epoch second through here.
    let now = as_of_override.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    });
    let mut kept: Vec<LaneHit> = Vec::with_capacity(fused.len());
    let mut state_counts: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    let mut action_counts: std::collections::BTreeMap<&'static str, u32> =
        std::collections::BTreeMap::new();
    for mut hit in std::mem::take(fused) {
        let candidate = candidate_from_hit(&hit);
        let (state, action) = classify(&candidate, now, flags, &classifier_cfg);
        *state_counts.entry(state.as_str()).or_insert(0) += 1;
        let action_label = match action {
            TemporalAction::Drop => "drop",
            TemporalAction::Pass { multiplier } => {
                if (multiplier - 1.0).abs() < f32::EPSILON {
                    "pass"
                } else {
                    "boost"
                }
            }
            TemporalAction::Demote { .. } => "demote",
        };
        *action_counts.entry(action_label).or_insert(0) += 1;
        // Phase18 §3.9 — per-hit `temporal_classification` envelope
        // on the `cortex_audit` tracing target. Operators tail this
        // channel to debug "why did Cortex drop this fact" without
        // having to run the query through the explain path.
        tracing::event!(
            target: "cortex_audit",
            tracing::Level::INFO,
            kind = "temporal_classification",
            query_id = %query_id,
            doc_id = %hit.doc_id,
            state = %state.as_str(),
            action = %action_label,
            as_of_unix = now,
        );
        match action {
            TemporalAction::Drop => continue,
            TemporalAction::Pass { multiplier } => {
                hit.score *= f64::from(multiplier);
                kept.push(hit);
            }
            TemporalAction::Demote { factor } => {
                hit.score *= f64::from(factor);
                kept.push(hit);
            }
        }
    }
    // Stable sort by adjusted score, ties broken by original order.
    kept.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Phase18 §3.9 — one rollup envelope per request so the
    // dashboard can chart "evaluated / dropped / boosted / demoted"
    // ratios without joining the per-hit stream.
    tracing::event!(
        target: "cortex_audit",
        tracing::Level::INFO,
        kind = "temporal_classification_summary",
        query_id = %query_id,
        evaluated = state_counts.values().sum::<u32>(),
        valid = state_counts.get("valid").copied().unwrap_or(0),
        temporal = state_counts.get("temporal").copied().unwrap_or(0),
        superseded = state_counts.get("superseded").copied().unwrap_or(0),
        expired = state_counts.get("expired").copied().unwrap_or(0),
        not_yet_valid = state_counts.get("not_yet_valid").copied().unwrap_or(0),
        abandoned = state_counts.get("abandoned").copied().unwrap_or(0),
        drops = action_counts.get("drop").copied().unwrap_or(0),
        boosts = action_counts.get("boost").copied().unwrap_or(0),
        demotes = action_counts.get("demote").copied().unwrap_or(0),
    );
    // Phase18 §7.2 — push per-state/per-action counts into the
    // Prometheus registry when a handle is wired.
    if let Some(m) = metrics {
        for (state, n) in &state_counts {
            m.record_state(state, u64::from(*n));
        }
        for (action, n) in &action_counts {
            m.record_action(action, u64::from(*n));
        }
    }
    *fused = kept;
}

/// Phase21 §5.5 — post-fusion ACL drop-wedge.
///
/// Iterates over all fused hits and drops those whose `class_level` /
/// `class_compartments` extras fail the `can_read` Bell-LaPadula
/// predicate for the calling principal's `acl`. Hits that carry no
/// classification metadata are treated as `class_level = 0` (public),
/// so they always pass. This function is the authoritative enforcement
/// gate; the per-lane filters (§5.2 – §5.4) are load-bearing
/// pre-filters that reduce result set size before fusion.
///
/// Emits per-hit + per-request audit events on `cortex_audit` so
/// operators can diagnose "why did Cortex hide this fact" without
/// needing the explain path.
/// Phase21 §8.1 — categorise the reason for an ACL decision so the
/// `access_decision` envelope carries a human-readable `reason` field.
fn acl_decision_reason(
    acl: &crate::lanes::AclContext,
    fact_level: Option<u8>,
    permit: bool,
) -> &'static str {
    if !permit {
        let level = fact_level.unwrap_or(0);
        if acl.clearance_level < level {
            return "above_clearance_level";
        }
        return "missing_compartment";
    }
    "granted"
}

fn apply_acl_wedge(
    fused: &mut Vec<LaneHit>,
    acl: &crate::lanes::AclContext,
    query_id: &str,
    principal_id: Option<&str>,
    acl_metrics: Option<&crate::AclMetrics>,
) {
    let pid = principal_id.unwrap_or("anonymous");
    let mut kept: Vec<LaneHit> = Vec::with_capacity(fused.len());
    let mut evaluated: u32 = 0;
    let mut granted: u32 = 0;
    let mut denied: u32 = 0;

    for hit in std::mem::take(fused) {
        evaluated += 1;

        let fact_level = hit
            .extras
            .get("class_level")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8);
        let fact_compartments: Vec<String> = hit
            .extras
            .get("class_compartments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        let permit = cortex_workers::acl::filter::vectorizer_acl_matches(
            acl.clearance_level,
            &acl.compartment_grants,
            fact_level,
            &fact_compartments,
        );

        let verdict: &'static str = if permit { "grant" } else { "deny" };
        let reason = acl_decision_reason(acl, fact_level, permit);
        let comps = fact_compartments.join(",");
        tracing::event!(
            target: "cortex_audit",
            tracing::Level::INFO,
            kind = "access_decision",
            query_id = %query_id,
            principal_id = %pid,
            doc_id = %hit.doc_id,
            fact_level = ?fact_level,
            fact_compartments = %comps,
            clearance_level = acl.clearance_level,
            verdict = %verdict,
            reason = %reason,
        );

        // Phase21 §8.2 — push this decision into the metrics store when wired.
        if let Some(m) = acl_metrics {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            m.record(
                now_secs,
                verdict,
                pid,
                fact_level.unwrap_or(0),
                reason,
                &hit.doc_id,
            );
        }

        if permit {
            granted += 1;
            kept.push(hit);
        } else {
            denied += 1;
        }
    }

    tracing::event!(
        target: "cortex_audit",
        tracing::Level::INFO,
        kind = "access_decision_summary",
        query_id = %query_id,
        principal_id = %pid,
        evaluated = evaluated,
        granted = granted,
        denied = denied,
    );

    *fused = kept;
}

/// Phase18 §3.9 — emit a `branch_resolution` envelope on the
/// `cortex_audit` channel. Today's orchestrator does not yet
/// receive a caller-supplied branch (phase18 §4 P3 extends the
/// request surface with `branch`), so the default chain is the
/// canonical root `<project>:main`. Once the request carries the
/// branch + an in-process parent map, the wedge call site swaps
/// the singleton chain for the full ancestry walk.
fn emit_branch_resolution_envelope(
    query_id: &str,
    project: &str,
    branch_override: Option<&str>,
    metrics: Option<&crate::TemporalMetrics>,
) {
    use cortex_workers::temporal::branch_filter::{compose_id, DEFAULT_BRANCH};
    // Resolve the requested branch composite id: an explicit `req.branch`
    // either ships pre-composed (`<project>:<branch>`) or as the bare
    // branch name. The walker normalises both shapes before stamping.
    let resolved = match branch_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if s.contains(':') => s.to_string(),
        Some(s) => compose_id(project, s),
        None => compose_id(project, DEFAULT_BRANCH),
    };
    // Phase18 §7.2 — branch-usage counter, bucketed main vs non-main
    // to keep the metric cardinality bounded.
    if let Some(m) = metrics {
        m.record_branch(resolved.ends_with(":main"));
    }
    // Today's call site does not hydrate a parent_map (phase18 §3.5 ships
    // the helper; the per-request cache lands with the §4 surface). Stamp
    // the singleton chain — the audit consumer sees the requested branch
    // even when the parent walk is a no-op.
    let chain = vec![resolved.clone()];
    tracing::event!(
        target: "cortex_audit",
        tracing::Level::INFO,
        kind = "branch_resolution",
        query_id = %query_id,
        branch = %resolved,
        ancestry_chain = ?chain,
    );
}

/// Phase18 §4.4 — parse the request's `as_of` field into epoch
/// seconds. Accepts RFC-3339 or `YYYY-MM-DD` per ADR-018; empty
/// / unparseable input falls through to `None` so the wedge uses
/// the wall clock.
fn parse_as_of_to_unix(raw: Option<&str>) -> Option<i64> {
    let s = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    chrono::DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Phase18 §5.2 — parse an RFC-3339 / `YYYY-MM-DD` timestamp (as stored
/// on a `CROSS_PROJECT_REF` edge's `valid_from` / `valid_to` props) to
/// epoch seconds. Thin wrapper over [`parse_as_of_to_unix`] so the
/// cross-project propagation path shares the exact same date grammar
/// the temporal wedge uses.
fn rfc3339_to_unix(s: &str) -> Option<i64> {
    parse_as_of_to_unix(Some(s))
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
    // ADR-011 — typed overlay carries the truncation flag. Fall
    // back to extras during the staged migration so a lane that
    // has not yet been ported keeps working.
    let body_truncated = hit.overlay.body_truncated
        || hit
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
        doc_id: (!hit.doc_id.trim().is_empty()).then(|| hit.doc_id.clone()),
        text: hit.text.clone(),
        body_truncated,
        score: hit.score,
        why: hit
            .extras
            .get("why")
            .and_then(|v| v.as_str())
            .map(String::from),
        // Phase17 §3.5 — phantom-link verification is applied as a
        // post-assembly pass in `Orchestrator::run` (see
        // `apply_phantom_link_verification`). These fields start as
        // `None` and are filled in by that pass.
        verified: None,
        verdict: None,
        // Phase21 §5.6 — project classification metadata from extras so
        // the pre-thinking bundle filter can enforce `can_read` on the
        // assembled prompt context without re-accessing the lane level.
        class_level: hit
            .extras
            .get("class_level")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8),
        class_compartments: hit
            .extras
            .get("class_compartments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Phase17 §3.5 + §3.9 — phantom-link verification post-assembly pass.
///
/// For each snippet that has both a `path` and a `symbol`, runs
/// [`verify_symbol`] against `workspace_root/path`. Attaches `verified`
/// and `verdict` metadata. When `action = "filter"`, removes snippets
/// whose symbol was `NotFound`. Emits a `phantom_link_dropped` audit event
/// with the per-turn drop count (§3.9).
fn apply_phantom_link_verification(
    snippets: &mut Vec<crate::types::Snippet>,
    workspace_root: &std::path::Path,
    action: &str,
    query_id: &str,
) {
    let mut dropped = 0u32;

    for snippet in snippets.iter_mut() {
        let (Some(path_str), Some(symbol)) = (snippet.path.as_deref(), snippet.symbol.as_deref())
        else {
            continue;
        };
        let abs_path = workspace_root.join(path_str);
        let v = verify_symbol(&abs_path, symbol);
        snippet.verified = Some(v == SymbolVerdict::Verified);
        snippet.verdict = Some(v);
    }

    if action == "filter" {
        snippets.retain(|s| {
            let keep = s.verified.unwrap_or(true);
            if !keep {
                dropped += 1;
            }
            keep
        });
    } else {
        // "flag" mode: count how many are unverified for the audit event.
        for s in snippets.iter() {
            if s.verified == Some(false) {
                dropped += 1;
            }
        }
    }

    if dropped > 0 {
        tracing::info!(
            target: "cortex_audit",
            event = "phantom_link_dropped",
            dropped = dropped,
            action = action,
            query_id = query_id,
        );
    }
}

fn lane_label(hit: &LaneHit) -> String {
    // ADR-011 — typed lane-of-origin read. Unknown falls back to
    // `"vector"` to preserve the pre-ADR default. Lane impls that
    // already stamped `overlay.source` (every live lane post-§3.1)
    // win over the extras lookup; the legacy extras key still
    // serves any pre-ADR-011 fixture that has not been updated.
    match hit.overlay.source {
        crate::lanes::LaneSource::Vector => "vector".to_string(),
        crate::lanes::LaneSource::Keyword => "keyword".to_string(),
        crate::lanes::LaneSource::Graph => "graph".to_string(),
        crate::lanes::LaneSource::Unknown => hit
            .extras
            .get("source")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "vector".to_string()),
    }
}

fn derive_decisions(fused: &[LaneHit], limit: usize) -> Vec<DecisionRef> {
    fused
        .iter()
        .filter_map(|h| {
            // ADR-011 — typed overlay read replaces
            // `extras.get("decision_id").and_then(|v| v.as_str())`.
            let id = h.overlay.decision_id.as_deref()?;
            // phase10a §1.2 — prefer the lane's stamped
            // `decision_title` over the kind label that
            // `LaneHit.symbol` carries today (the meili projection
            // sets `symbol = doc.kind`, so without this fallback
            // every decision overlay rendered as the literal string
            // `"decision"`). `decision_title` and `rationale_excerpt`
            // remain in `extras` for now — they were never part of
            // the typed Overlay rollup; phase11v will fold them into
            // a dedicated `decision_meta` Overlay field.
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
                // ADR-011 — typed `DecisionStatus`. The wire
                // contract still emits the snake_case string; map
                // the enum back via `as_str_repr` to preserve
                // byte-for-byte parity with the pre-ADR path. A
                // hit that lacks `decision_status` falls back to
                // "proposed" (the previous behaviour).
                status: match h.overlay.decision_status {
                    Some(crate::lanes::DecisionStatus::Proposed) => "proposed",
                    Some(crate::lanes::DecisionStatus::Accepted) => "accepted",
                    Some(crate::lanes::DecisionStatus::Superseded) => "superseded",
                    Some(crate::lanes::DecisionStatus::Deprecated) => "deprecated",
                    Some(crate::lanes::DecisionStatus::Rejected) => "rejected",
                    None => "proposed",
                }
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
        // ADR-011 — typed overlay read replaces `extras.get("law_id")`.
        if let Some(law_id) = h.overlay.law_id.as_deref() {
            active.push(LawRef {
                id: law_id.to_string(),
                severity: h.severity.clone().unwrap_or_else(|| "info".into()),
                title: h.symbol.clone().unwrap_or_else(|| h.text.clone()),
            });
        }
        if let Some(violation_id) = h.overlay.violation_id.as_deref() {
            violations.push(ViolationRef {
                id: violation_id.to_string(),
                law_id: h.overlay.law_id.as_deref().unwrap_or("LAW-???").to_string(),
                severity: h.severity.clone().unwrap_or_else(|| "info".into()),
                message: h.text.clone(),
                // `observed_in` stays on extras for now — it is a
                // lane-debug field, not part of the typed Overlay.
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
            // ADR-011 — typed overlay read replaces
            // `extras.get("edge_from") / edge_to / hops`.
            let from = h.overlay.edge_from.as_deref()?;
            let to = h.overlay.edge_to.as_deref()?;
            // `edge_type` is graph-lane-only debug metadata; stays
            // on extras until phase11v adds a typed
            // `Overlay::edge_relation` field. Falls back to
            // `"RELATED"` when missing (legacy behaviour).
            let relation = h
                .extras
                .get("edge_type")
                .and_then(|v| v.as_str())
                .unwrap_or("RELATED");
            let hops = h.overlay.hops.unwrap_or(1);
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
            // ADR-011 — typed overlay read replaces
            // `extras.get("turn_id") / model / summary`. `outcome`
            // stays on extras until phase11v adds a typed
            // `Overlay::turn_outcome` field — it is currently only
            // stamped by the consolidator's session producer and
            // not by any live lane.
            let turn_id = h.overlay.turn_id.as_deref()?;
            Some(SimilarTurn {
                turn_id: turn_id.to_string(),
                ts: h.ts,
                model: h
                    .overlay
                    .model
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                summary: h.overlay.summary.clone().unwrap_or_else(|| h.text.clone()),
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
            overlay: crate::lanes::Overlay::default(),
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

    // -----------------------------------------------------------
    // ADR-011 §3.3 — empty-overlay regression tests.
    // -----------------------------------------------------------

    fn lane_hit_default(doc_id: &str) -> LaneHit {
        LaneHit {
            doc_id: doc_id.to_string(),
            text: format!("text-{doc_id}"),
            ..Default::default()
        }
    }

    #[test]
    fn derive_decisions_skips_hit_with_empty_overlay() {
        // A `LaneHit` whose overlay carries no `decision_id` must
        // NOT surface as a DecisionRef. Pre-ADR-011 the equivalent
        // gate was `extras.get("decision_id")?` — drop the typed
        // overlay equivalent and the fallthrough path is gone.
        let hit = lane_hit_default("doc-1");
        assert!(hit.overlay.is_empty(), "fixture overlay must start empty");
        let decisions = super::derive_decisions(&[hit], 10);
        assert!(
            decisions.is_empty(),
            "empty-overlay hit must NOT produce a DecisionRef"
        );
    }

    #[test]
    fn derive_graph_neighbors_skips_hit_with_empty_overlay() {
        // The graph-neighbor projector needs `edge_from` AND
        // `edge_to`. An empty overlay yields neither so the hit
        // is silently dropped (no panic, no fall-through to a
        // legacy extras path).
        let hit = lane_hit_default("doc-2");
        let neighbors = super::derive_graph_neighbors(&[hit], 10);
        assert!(neighbors.is_empty());
    }

    #[test]
    fn derive_similar_turns_skips_hit_with_empty_overlay() {
        let hit = lane_hit_default("doc-3");
        let turns = super::derive_similar_turns(&[hit], 10);
        assert!(turns.is_empty());
    }

    #[test]
    fn derive_laws_skips_hit_with_empty_overlay() {
        let hit = lane_hit_default("doc-4");
        let hits = std::slice::from_ref(&hit);
        let (laws, violations) = super::derive_laws(hits, hits, 10);
        assert!(laws.is_empty());
        assert!(violations.is_empty());
    }

    #[test]
    fn three_lanes_emit_empty_overlays_without_panic() {
        // ADR-011 §3.3 — exercise every overlay deriver against
        // hits emitted by three distinct lanes (Vector / Keyword /
        // Graph), each carrying ONLY a lane-of-origin stamp and
        // nothing else. The pre-ADR-011 stringly-typed path would
        // have panicked on `as_str()` when extras lacked the
        // expected keys; the typed path must return cleanly.
        let mut vector_hit = lane_hit_default("vec-1");
        vector_hit.overlay.source = crate::lanes::LaneSource::Vector;
        let mut keyword_hit = lane_hit_default("kw-1");
        keyword_hit.overlay.source = crate::lanes::LaneSource::Keyword;
        let mut graph_hit = lane_hit_default("gr-1");
        graph_hit.overlay.source = crate::lanes::LaneSource::Graph;

        let fused = vec![vector_hit.clone(), keyword_hit.clone(), graph_hit.clone()];
        let decisions = super::derive_decisions(&fused, 10);
        let neighbors = super::derive_graph_neighbors(&[graph_hit.clone()], 10);
        let turns = super::derive_similar_turns(&fused, 10);
        let (laws, violations) = super::derive_laws(&[keyword_hit], &[graph_hit], 10);

        assert!(decisions.is_empty(), "no decision_id → no DecisionRef");
        assert!(neighbors.is_empty(), "no edge_from/to → no GraphNeighbor");
        assert!(turns.is_empty(), "no turn_id → no SimilarTurn");
        assert!(laws.is_empty(), "no law_id → no LawRef");
        assert!(violations.is_empty(), "no violation_id → no ViolationRef");

        // The vector-lane hit's `lane_label` MUST report the
        // typed source instead of the legacy extras fallback.
        assert_eq!(super::lane_label(&vector_hit), "vector");
    }

    // -----------------------------------------------------------
    // Phase18 §3.3 — temporal classifier wedge pins.
    // -----------------------------------------------------------

    fn now_unix() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    fn hit_with_extras(doc_id: &str, score: f64, extras: &[(&str, serde_json::Value)]) -> LaneHit {
        let mut props = crate::types::Props::new();
        for (k, v) in extras {
            props.insert((*k).to_string(), v.clone());
        }
        LaneHit {
            doc_id: doc_id.to_string(),
            text: format!("body-{doc_id}"),
            repo: Some("cortex".into()),
            path: Some(format!("path/{doc_id}.rs")),
            symbol: Some(format!("sym_{doc_id}")),
            content_hash: None,
            score,
            ts: 0,
            severity: None,
            extras: props,
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[test]
    fn temporal_wedge_drops_superseded_hits_under_default_flags() {
        // SUPERSEDED + include_history=false → drop.
        let past = now_unix() - 86_400;
        let mut hits = vec![
            hit_with_extras("alive", 1.0, &[]),
            hit_with_extras(
                "dead",
                2.0,
                &[("superseded_at_unix", serde_json::json!(past))],
            ),
        ];
        let cfg = TemporalConfig::default();
        apply_temporal_classifier(
            &mut hits,
            &cfg,
            "test-query-id",
            None,
            IncludeFlags::default(),
            None,
        );
        assert_eq!(hits.len(), 1, "superseded hit must drop by default");
        assert_eq!(hits[0].doc_id, "alive");
    }

    #[test]
    fn temporal_wedge_passes_valid_hits_unchanged() {
        // No bitemporal columns ⇒ VALID ⇒ score unchanged.
        let mut hits = vec![
            hit_with_extras("a", 1.0, &[]),
            hit_with_extras("b", 0.5, &[]),
        ];
        let cfg = TemporalConfig::default();
        apply_temporal_classifier(
            &mut hits,
            &cfg,
            "test-query-id",
            None,
            IncludeFlags::default(),
            None,
        );
        assert_eq!(hits.len(), 2);
        // Sort is descending by score; `a` (1.0) must lead `b` (0.5).
        assert_eq!(hits[0].doc_id, "a");
        assert_eq!(hits[1].doc_id, "b");
        assert!((hits[0].score - 1.0).abs() < 1e-9);
        assert!((hits[1].score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn temporal_wedge_boosts_temporal_state_by_configured_multiplier() {
        // `valid_to` lands inside the recency window ⇒ TEMPORAL ⇒
        // score *= 1.10 (design.md §2.2 default).
        let now = now_unix();
        let mut hits = vec![hit_with_extras(
            "soon",
            1.0,
            &[("valid_to_unix", serde_json::json!(now + 3600))],
        )];
        let cfg = TemporalConfig::default();
        apply_temporal_classifier(
            &mut hits,
            &cfg,
            "test-query-id",
            None,
            IncludeFlags::default(),
            None,
        );
        assert_eq!(hits.len(), 1);
        let expected = 1.0 * f64::from(cfg.temporal_boost);
        assert!(
            (hits[0].score - expected).abs() < 1e-6,
            "score must be boosted by temporal_boost ({} got {})",
            expected,
            hits[0].score
        );
    }

    #[test]
    fn temporal_wedge_demotes_superseded_when_include_history_default_set() {
        // `include_history_default = true` ⇒ SUPERSEDED demoted, not dropped.
        let past = now_unix() - 86_400;
        let mut hits = vec![hit_with_extras(
            "dead",
            2.0,
            &[("superseded_at_unix", serde_json::json!(past))],
        )];
        let cfg = TemporalConfig {
            include_history_default: true,
            ..TemporalConfig::default()
        };
        apply_temporal_classifier(
            &mut hits,
            &cfg,
            "test-query-id",
            None,
            IncludeFlags::default(),
            None,
        );
        assert_eq!(hits.len(), 1, "demoted hits are kept");
        let expected = 2.0 * f64::from(cfg.demote_factor);
        assert!((hits[0].score - expected).abs() < 1e-6);
    }

    // -----------------------------------------------------------
    // Phase21 §7.2 — AccessControlConfig handle + disabled pass-through.
    // -----------------------------------------------------------

    #[test]
    fn access_control_defaults_disabled_and_reader_returns_snapshot() {
        let orch = Orchestrator::new(
            Arc::new(crate::lanes::MemoryVectorLane::new()),
            Arc::new(crate::lanes::MemoryKeywordLane::new()),
            Arc::new(crate::lanes::MemoryGraphLane::new()),
        );
        let cfg = orch.current_access_control();
        assert!(!cfg.enabled, "default AC config must be OFF (ADR-1.3)");
        assert_eq!(cfg.default_level, 1, "default_level must be 1 (internal)");
        assert!(
            cfg.deny_on_missing_principal,
            "deny_on_missing_principal must default true"
        );
        assert!(cfg.default_compartments.is_empty());
    }

    #[test]
    fn with_access_control_installs_enabled_config() {
        let orch = Orchestrator::new(
            Arc::new(crate::lanes::MemoryVectorLane::new()),
            Arc::new(crate::lanes::MemoryKeywordLane::new()),
            Arc::new(crate::lanes::MemoryGraphLane::new()),
        )
        .with_access_control(AccessControlConfig {
            enabled: true,
            ..AccessControlConfig::default()
        });
        assert!(orch.current_access_control().enabled);
    }

    #[test]
    fn replace_access_control_hot_swaps_snapshot() {
        let orch = Orchestrator::new(
            Arc::new(crate::lanes::MemoryVectorLane::new()),
            Arc::new(crate::lanes::MemoryKeywordLane::new()),
            Arc::new(crate::lanes::MemoryGraphLane::new()),
        );
        assert!(!orch.current_access_control().enabled);
        orch.replace_access_control(AccessControlConfig {
            enabled: true,
            ..AccessControlConfig::default()
        });
        assert!(orch.current_access_control().enabled);
    }

    #[test]
    fn temporal_wedge_reorders_post_score_descending() {
        // Pre-wedge order: low-score VALID first, high-score TEMPORAL second.
        // Post-wedge: TEMPORAL (boosted) must lead.
        let now = now_unix();
        let mut hits = vec![
            hit_with_extras("low_valid", 1.0, &[]),
            hit_with_extras(
                "high_temporal",
                0.95,
                &[("valid_to_unix", serde_json::json!(now + 86_400))],
            ),
        ];
        let cfg = TemporalConfig::default();
        apply_temporal_classifier(
            &mut hits,
            &cfg,
            "test-query-id",
            None,
            IncludeFlags::default(),
            None,
        );
        assert_eq!(hits[0].doc_id, "high_temporal");
    }
}
