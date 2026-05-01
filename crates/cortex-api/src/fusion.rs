//! Reciprocal Rank Fusion. Spec 11 §Fan-out + fusion + Fusion algorithm.
//!
//! Phase6c — the original positional-only blend (`Σ 1/(60+rank)`)
//! discarded the lane-native scores already captured into
//! `LaneHit.score`. A single weak graph hit at rank 1 ended up
//! tied with a top-3 vector hit, which let sparse-lane noise
//! whipsaw fused order. The score-aware blend below mixes the
//! positional reciprocal with the normalised native score:
//!
//! ```text
//! fused(d) = Σ_lanes [ alpha * (1 / (k + rank_lane(d)))
//!                    + (1 - alpha) * lane.normalized_score(d) ]
//! ```
//!
//! `alpha = 1.0` reproduces today's positional-only RRF (regression
//! escape hatch); `alpha = 0.0` ranks by native score alone. The
//! orchestrator carries a [`FusionConfig`] sourced from the
//! `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` env vars at boot.
//!
//! Ties break on recency (higher `ts` first), then severity
//! (`critical > notable > info`), then `doc_id` for determinism.

use std::collections::BTreeMap;

use crate::lanes::LaneHit;

/// Standard RRF k-constant from Cormack et al. 2009.
pub const RRF_K: f64 = 60.0;

/// Default blend weight: 70% positional / 30% native score. Picked
/// to bias toward RRF's stability while still letting strong
/// lane-native scores break weak-positional ties. Operators can
/// override at boot via `CORTEX_RRF_ALPHA`.
pub const DEFAULT_RRF_ALPHA: f32 = 0.7;

/// Phase11i §3.1 — default recency-decay λ (per day) for the
/// `pre_change_context` intent. `score *= exp(-λ · days_old)`
/// where `days_old` is the age of the hit in days. λ=0 disables
/// decay entirely (the legacy behaviour); λ=0.02 gives a ~50 %
/// half-life around 35 days, which balances "bias toward recent
/// work" against "yes we still want to surface a six-month-old
/// ADR when it's the right answer".
pub const DEFAULT_RECENCY_LAMBDA_PRE_CHANGE: f32 = 0.02;
/// Recency λ for `decision_lookup`. Decisions are sticky — once
/// an ADR is canonical it stays canonical. λ=0.005 (~140-day
/// half-life) so the recency bonus is mild.
pub const DEFAULT_RECENCY_LAMBDA_DECISION: f32 = 0.005;
/// Recency λ for `law_check`. Laws are evergreen — the body of
/// a LAW-CORTEX-* declaration does not get more or less true
/// with time. λ=0 disables the bonus.
pub const DEFAULT_RECENCY_LAMBDA_LAW: f32 = 0.0;

/// Number of milliseconds in one calendar day. Used by the
/// recency-decay path to convert `now_ms - hit.ts` into days.
pub const MS_PER_DAY: f64 = 86_400_000.0;

/// Phase11i §3.2 — default cross-repo boost. `0.0` means
/// "out-of-scope hits do not contribute" (the legacy hard-filter
/// behaviour); `1.0` means "weight cross-repo hits the same as
/// in-scope". The unit-clamp keeps the multiplier well-defined.
pub const DEFAULT_CROSS_REPO_BOOST: f32 = 0.0;

/// Tunable parameters for [`rrf_fuse`].
///
/// Built once at orchestrator boot from the
/// `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` env vars and stamped on
/// every audit envelope so phase6e's harness can attribute
/// regressions to fusion-tuning changes.
#[derive(Debug, Clone, PartialEq)]
pub struct FusionConfig {
    /// Blend weight in `[0.0, 1.0]`. `1.0` = pure positional RRF;
    /// `0.0` = pure normalised native score.
    pub alpha: f32,
    /// RRF stabilisation constant (Cormack et al. default = 60).
    /// Larger `k` flattens the per-lane curve; smaller `k`
    /// emphasises rank-1 hits.
    pub k: u32,
    /// Phase11i §3.1 — recency decay λ in units of `1 / day`.
    /// Each hit's fused score is multiplied by
    /// `exp(-recency_decay_lambda · days_old)` where `days_old`
    /// is `(now_ms - hit.ts) / 86_400_000`. `0.0` disables the
    /// decay entirely (the legacy behaviour); positive values
    /// bias toward recent hits.
    pub recency_decay_lambda: f32,
    /// Phase11i §3.1 — current wall-clock anchor in epoch ms.
    /// Lets tests pin a deterministic `now`; callers that want
    /// real time pass `chrono::Utc::now().timestamp_millis()`.
    /// `0` is a sentinel meaning "use the system clock at fuse
    /// time" — the path computes `now_ms` lazily so tests can
    /// still control it via the explicit field.
    pub now_ms: i64,
    /// Phase11i §3.2 — cross-repo boost in `[0.0, 1.0]`. When
    /// `Some(active_repo)` is supplied alongside a positive
    /// boost, hits whose `repo` field differs from `active_repo`
    /// have their fused score multiplied by `cross_repo_boost`.
    /// `0.0` (default) cleanly excludes cross-repo hits from
    /// the bundle — the same outcome the per-repo lane filter
    /// already produces today. `1.0` lets cross-repo hits
    /// compete on equal footing.
    pub cross_repo_boost: f32,
    /// Phase11i §3.2 — active repo slug. When `None`, the
    /// cross-repo boost has no effect (every hit is treated as
    /// "in scope"). When `Some(s)`, hits whose `repo` does not
    /// equal `s` get the boost multiplier applied.
    pub active_repo: Option<String>,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_RRF_ALPHA,
            k: RRF_K as u32,
            recency_decay_lambda: 0.0,
            now_ms: 0,
            cross_repo_boost: DEFAULT_CROSS_REPO_BOOST,
            active_repo: None,
        }
    }
}

impl FusionConfig {
    /// Construct a config with operator-supplied values, clamping
    /// `alpha` to `[0.0, 1.0]` and `k` to `>= 1` so the fusion
    /// formula stays numerically well-defined.
    pub fn new(alpha: f32, k: u32) -> Self {
        Self {
            alpha: alpha.clamp(0.0, 1.0),
            k: k.max(1),
            recency_decay_lambda: 0.0,
            now_ms: 0,
            cross_repo_boost: DEFAULT_CROSS_REPO_BOOST,
            active_repo: None,
        }
    }

    /// Phase11i §3.1 — chain a recency-decay λ onto an existing
    /// config. λ < 0 is clamped to 0 (no decay) so a malformed
    /// override never inverts the multiplier into "older first".
    pub fn with_recency_decay(mut self, lambda: f32) -> Self {
        self.recency_decay_lambda = lambda.max(0.0);
        self
    }

    /// Phase11i §3.2 — chain the cross-repo boost + active repo
    /// onto an existing config. The boost is clamped to
    /// `[0.0, 1.0]`; an `active_repo` of `Some("")` is treated as
    /// `None` so canonicalisation upstream that produces an
    /// empty string never accidentally suppresses every hit.
    pub fn with_cross_repo_boost(mut self, boost: f32, active_repo: Option<String>) -> Self {
        self.cross_repo_boost = boost.clamp(0.0, 1.0);
        self.active_repo = active_repo.filter(|s| !s.is_empty());
        self
    }

    /// Pin a deterministic `now_ms` anchor for tests + replay.
    /// Defaults to `0` which the fuse path treats as "use the
    /// system clock now".
    pub fn with_now_ms(mut self, now_ms: i64) -> Self {
        self.now_ms = now_ms;
        self
    }

    /// Return the per-intent recency-λ default. Centralised here
    /// so callers (the orchestrator + tests + future config-file
    /// reload) all read from the same source.
    pub fn default_recency_lambda_for_intent(intent: &str) -> f32 {
        match intent {
            "pre_change_context" | "free_search" | "explain" => DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            "similar_problems" => DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            "decision_lookup" => DEFAULT_RECENCY_LAMBDA_DECISION,
            "law_check" => DEFAULT_RECENCY_LAMBDA_LAW,
            _ => 0.0,
        }
    }
}

/// Fuse per-lane hit lists into a single ranked output. Each input
/// list must already be sorted in lane-native rank order (best
/// first). Returns hits ordered by fused score with deterministic
/// tie-breaks.
///
/// Phase11i §3.1 — when `cfg.recency_decay_lambda > 0`, every
/// hit's accumulated fused score is multiplied by
/// `exp(-λ · days_old)` before sorting. `days_old` is computed
/// from `cfg.now_ms` (or system clock when `now_ms == 0`) minus
/// `hit.ts`. Hits with `ts == 0` (no timestamp) skip the decay
/// so a missing-timestamp regression cannot reorder the bundle.
pub fn rrf_fuse(lanes: Vec<Vec<LaneHit>>, cfg: &FusionConfig) -> Vec<LaneHit> {
    let mut scores: BTreeMap<String, f64> = BTreeMap::new();
    let mut representative: BTreeMap<String, LaneHit> = BTreeMap::new();
    let alpha = cfg.alpha as f64;
    let k = cfg.k as f64;
    for lane in &lanes {
        for (idx, hit) in lane.iter().enumerate() {
            let rank = (idx + 1) as f64;
            let positional = 1.0 / (k + rank);
            let native = hit.normalized_score();
            let blended = alpha * positional + (1.0 - alpha) * native;
            *scores.entry(hit.doc_id.clone()).or_insert(0.0) += blended;
            representative
                .entry(hit.doc_id.clone())
                .or_insert_with(|| hit.clone());
        }
    }

    // Phase11i §3.1 — apply the recency-decay multiplier post-
    // accumulation so it operates on the full per-doc score
    // rather than on each lane's contribution. This keeps the
    // sort stable and the decay analytic.
    if cfg.recency_decay_lambda > 0.0 {
        let now_ms = if cfg.now_ms != 0 {
            cfg.now_ms
        } else {
            chrono::Utc::now().timestamp_millis()
        };
        let lambda = cfg.recency_decay_lambda as f64;
        for (doc_id, score) in scores.iter_mut() {
            if let Some(hit) = representative.get(doc_id) {
                let ts = hit.ts;
                if ts == 0 {
                    continue;
                }
                let days_old = (now_ms - ts).max(0) as f64 / MS_PER_DAY;
                let multiplier = (-lambda * days_old).exp();
                *score *= multiplier;
            }
        }
    }

    // Phase11i §3.2 — cross-repo boost. When the orchestrator
    // forks a parallel cross-repo lane scan, the foreign hits
    // come back with `repo != active_repo`. We multiply their
    // accumulated score by `cross_repo_boost` so an in-repo hit
    // at the same lane position always wins on tie. boost=0
    // collapses cross-repo hits to score=0, which the sort
    // naturally pushes to the tail; boost=1 lets foreign hits
    // compete on equal footing.
    if let Some(active) = cfg.active_repo.as_deref() {
        if cfg.cross_repo_boost < 1.0 && !active.is_empty() {
            let boost = cfg.cross_repo_boost as f64;
            for (doc_id, score) in scores.iter_mut() {
                if let Some(hit) = representative.get(doc_id) {
                    if let Some(repo) = hit.repo.as_deref() {
                        if !repo.is_empty() && repo != active {
                            *score *= boost;
                        }
                    }
                }
            }
        }
    }

    let mut out: Vec<(LaneHit, f64)> = representative
        .into_iter()
        .map(|(id, hit)| {
            let s = *scores.get(&id).unwrap_or(&0.0);
            (hit, s)
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.0.ts.cmp(&a.0.ts))
            .then(severity_rank(&b.0.severity).cmp(&severity_rank(&a.0.severity)))
            .then(a.0.doc_id.cmp(&b.0.doc_id))
    });
    out.into_iter()
        .map(|(mut hit, score)| {
            hit.score = score;
            hit
        })
        .collect()
}

fn severity_rank(s: &Option<String>) -> u8 {
    match s.as_deref() {
        Some("critical") => 3,
        Some("notable") => 2,
        Some("info") => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, ts: i64, severity: Option<&str>) -> LaneHit {
        LaneHit {
            doc_id: id.to_string(),
            text: format!("text-{id}"),
            repo: None,
            path: None,
            symbol: None,
            content_hash: None,
            score: 0.0,
            ts,
            severity: severity.map(String::from),
            extras: Default::default(),
        }
    }

    fn hit_with_score(id: &str, native: f64) -> LaneHit {
        LaneHit {
            score: native,
            ..hit(id, 0, None)
        }
    }

    /// Positional-only baseline used by the equivalence test below.
    /// Matches the pre-phase6c hard-coded behaviour byte-for-byte.
    fn positional_only(lanes: Vec<Vec<LaneHit>>) -> Vec<LaneHit> {
        rrf_fuse(lanes, &FusionConfig { alpha: 1.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None })
    }

    #[test]
    fn rrf_sums_reciprocal_ranks_across_lanes() {
        // Pure positional baseline (alpha=1.0) so the analytic
        // reciprocal sum is exact.
        let lane_a = vec![hit("X", 0, None), hit("Y", 0, None)];
        let lane_b = vec![hit("Y", 0, None), hit("X", 0, None)];
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 1.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None });
        assert_eq!(fused.len(), 2);
        // X: 1/(60+1) + 1/(60+2) = 1/61 + 1/62; Y: same. Tie broken
        // by doc_id ⇒ X first.
        assert_eq!(fused[0].doc_id, "X");
        let expected = (1.0_f64 / 61.0) + (1.0_f64 / 62.0);
        assert!((fused[0].score - expected).abs() < 1e-9);
    }

    #[test]
    fn ties_break_on_recency() {
        let cfg = FusionConfig { alpha: 1.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None };
        let lane_a = vec![hit("OLD", 100, None), hit("NEW", 200, None)];
        let lane_b = vec![hit("OLD", 100, None), hit("NEW", 200, None)];
        let fused = rrf_fuse(vec![lane_a, lane_b], &cfg);
        assert_eq!(fused[0].doc_id, "OLD", "lane-rank wins over recency");
        // Test the recency tie-break with two equally-ranked hits.
        let lane_c = vec![hit("OLD", 100, None)];
        let lane_d = vec![hit("NEW", 200, None)];
        let fused2 = rrf_fuse(vec![lane_c, lane_d], &cfg);
        assert_eq!(fused2[0].doc_id, "NEW");
    }

    #[test]
    fn ties_break_on_severity_after_recency() {
        let lane_a = vec![hit("INFO", 100, Some("info"))];
        let lane_b = vec![hit("CRIT", 100, Some("critical"))];
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 1.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None });
        assert_eq!(fused[0].doc_id, "CRIT");
    }

    #[test]
    fn empty_lanes_produce_empty_output() {
        assert!(rrf_fuse(vec![vec![], vec![]], &FusionConfig::default()).is_empty());
    }

    // ---------------- Phase6c regression suite ----------------

    #[test]
    fn weak_graph_hit_does_not_outrank_dense_vector_top3() {
        // Vector lane: dense top-3 with strong native scores.
        // Graph lane: a single weak hit (native 0.10) at rank 1.
        // Pre-phase6c, both rank-1 entries shared `1/(60+1) ≈ 0.0164`
        // and the graph hit could nudge ahead via tie-break. With
        // the score-aware blend, the strong vector top-3 retain
        // their position because their native scores dominate the
        // `(1 - alpha)` term.
        let vector = vec![
            hit_with_score("V1", 0.92),
            hit_with_score("V2", 0.88),
            hit_with_score("V3", 0.85),
        ];
        let graph = vec![hit_with_score("G1", 0.10)];
        let fused = rrf_fuse(vec![vector, vec![], graph], &FusionConfig::default());
        let position = fused
            .iter()
            .position(|h| h.doc_id == "G1")
            .expect("graph hit present in fused output");
        assert!(
            position >= 3,
            "weak graph hit should land at or below position 4; got position {} in {:?}",
            position + 1,
            fused.iter().map(|h| &h.doc_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn all_equal_native_scores_reduce_to_positional_rrf() {
        // When every hit has the same native score, the
        // `(1 - alpha) * native` term contributes the same constant
        // per lane-rank class — so the fused order MUST match the
        // pure-positional baseline.
        let make_lanes = || {
            let lane_a = vec![
                hit_with_score("A", 0.5),
                hit_with_score("B", 0.5),
                hit_with_score("C", 0.5),
            ];
            let lane_b = vec![
                hit_with_score("B", 0.5),
                hit_with_score("C", 0.5),
                hit_with_score("A", 0.5),
            ];
            vec![lane_a, lane_b]
        };
        let blended = rrf_fuse(make_lanes(), &FusionConfig::default());
        let positional = positional_only(make_lanes());
        let blended_ids: Vec<&str> = blended.iter().map(|h| h.doc_id.as_str()).collect();
        let positional_ids: Vec<&str> = positional.iter().map(|h| h.doc_id.as_str()).collect();
        assert_eq!(
            blended_ids, positional_ids,
            "uniform native scores must collapse onto the positional ranking"
        );
    }

    #[test]
    fn alpha_one_reproduces_positional_only() {
        // Regression escape hatch: operators reverting to the
        // pre-phase6c behaviour set `CORTEX_RRF_ALPHA=1.0`. The
        // fused score for each hit MUST match the positional-only
        // baseline within float epsilon.
        let lane_a = vec![hit_with_score("X", 0.91), hit_with_score("Y", 0.42)];
        let lane_b = vec![hit_with_score("Y", 0.50)];
        let blended = rrf_fuse(
            vec![lane_a.clone(), lane_b.clone()],
            &FusionConfig { alpha: 1.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None },
        );
        let baseline = positional_only(vec![lane_a, lane_b]);
        assert_eq!(blended.len(), baseline.len());
        for (b, p) in blended.iter().zip(baseline.iter()) {
            assert_eq!(b.doc_id, p.doc_id);
            assert!(
                (b.score - p.score).abs() < 1e-9,
                "alpha=1.0 must match positional-only; doc {} blended={} baseline={}",
                b.doc_id,
                b.score,
                p.score
            );
        }
    }

    #[test]
    fn alpha_zero_sorts_by_native_score() {
        // `alpha = 0.0` removes the positional contribution
        // entirely; ranking is by summed normalised native score.
        // The hit with the highest summed native score MUST land
        // first regardless of rank position.
        let lane_a = vec![
            hit_with_score("WEAK_RANK1", 0.10),
            hit_with_score("STRONG_RANK2", 0.95),
        ];
        let lane_b = vec![hit_with_score("STRONG_RANK2", 0.90)];
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 0.0, k: 60, recency_decay_lambda: 0.0, now_ms: 0, cross_repo_boost: 0.0, active_repo: None });
        assert_eq!(
            fused[0].doc_id, "STRONG_RANK2",
            "alpha=0.0 must rank by native score sum"
        );
        // Summed native = 0.95 + 0.90 = 1.85.
        assert!((fused[0].score - 1.85).abs() < 1e-9);
    }

    #[test]
    fn fusion_config_clamps_out_of_range_inputs() {
        let cfg = FusionConfig::new(2.5, 0);
        assert_eq!(cfg.alpha, 1.0);
        assert_eq!(cfg.k, 1);
        let cfg = FusionConfig::new(-0.4, 60);
        assert_eq!(cfg.alpha, 0.0);
        assert_eq!(cfg.k, 60);
    }

    // ---------------- Phase11i §3.1 — recency decay ----------------

    fn hit_with_ts(id: &str, ts: i64) -> LaneHit {
        LaneHit {
            score: 0.5,
            ..hit(id, ts, None)
        }
    }

    #[test]
    fn lambda_zero_reproduces_pre_decay_behaviour() {
        // The default config sets λ=0 so existing tests stay
        // green. With OLD at rank 1 and NEW at rank 2 in both
        // lanes, the legacy positional+native blend ranks OLD
        // first regardless of ts (recency tie-break only fires
        // on score ties; OLD's positional score is strictly
        // larger).
        let lane_a = vec![hit_with_ts("OLD", 1), hit_with_ts("NEW", 2)];
        let lane_b = vec![hit_with_ts("OLD", 1), hit_with_ts("NEW", 2)];
        let cfg = FusionConfig::default();
        assert_eq!(cfg.recency_decay_lambda, 0.0);
        let fused = rrf_fuse(vec![lane_a, lane_b], &cfg);
        assert_eq!(fused[0].doc_id, "OLD");
    }

    #[test]
    fn positive_lambda_demotes_older_hits_when_scores_match() {
        // Two hits at the same lane rank; one is 0 days old, the
        // other 30 days old. With λ=0.02, the 30-day-old hit's
        // multiplier is exp(-0.6) ≈ 0.549, so the recent hit must
        // outrank it.
        let day_ms = MS_PER_DAY as i64;
        let now_ms = 100 * day_ms;
        let recent = hit_with_ts("RECENT", now_ms);
        let old = hit_with_ts("OLD", now_ms - 30 * day_ms);
        let cfg = FusionConfig {
            alpha: 1.0, // pure positional so recency decay dominates
            k: 60,
            recency_decay_lambda: 0.02,
            now_ms,
            cross_repo_boost: 0.0,
            active_repo: None,
        };
        let fused = rrf_fuse(vec![vec![recent], vec![old]], &cfg);
        assert_eq!(fused[0].doc_id, "RECENT");
        assert!(fused[0].score > fused[1].score);
        let ratio = fused[1].score / fused[0].score;
        // λ stored as f32, cast to f64 inside rrf_fuse — the
        // f32→f64 widening loses the bottom bits of 0.02, so the
        // realised exponent is `-0.02_f32 * 30.0` rather than
        // `-0.02_f64 * 30.0`. Tolerance covers the ~7e-9 drift.
        let expected = (-(0.02_f32 as f64) * 30.0).exp();
        assert!(
            (ratio - expected).abs() < 1e-6,
            "expected ratio≈{expected}, got {ratio}"
        );
    }

    #[test]
    fn ts_zero_hits_skip_recency_decay() {
        // A hit with no timestamp must pass through with zero
        // decay so a metadata regression doesn't accidentally
        // bury it.
        let day_ms = MS_PER_DAY as i64;
        let now_ms = 50 * day_ms;
        let untimed = hit_with_ts("UNTIMED", 0); // ts=0 sentinel
        let dated = hit_with_ts("DATED", now_ms - 30 * day_ms);
        let cfg = FusionConfig {
            alpha: 1.0,
            k: 60,
            recency_decay_lambda: 0.02,
            now_ms,
            cross_repo_boost: 0.0,
            active_repo: None,
        };
        let fused = rrf_fuse(vec![vec![untimed], vec![dated]], &cfg);
        // UNTIMED keeps its full score; DATED gets exp(-0.6) ≈ 0.549.
        assert_eq!(fused[0].doc_id, "UNTIMED");
    }

    #[test]
    fn negative_lambda_clamps_to_zero() {
        // `with_recency_decay` clamps λ < 0 to 0 so a malformed
        // operator override does not invert the multiplier.
        let cfg = FusionConfig::default().with_recency_decay(-0.5);
        assert_eq!(cfg.recency_decay_lambda, 0.0);
    }

    #[test]
    fn intent_default_table_resolves_known_intents() {
        assert_eq!(
            FusionConfig::default_recency_lambda_for_intent("pre_change_context"),
            DEFAULT_RECENCY_LAMBDA_PRE_CHANGE
        );
        assert_eq!(
            FusionConfig::default_recency_lambda_for_intent("decision_lookup"),
            DEFAULT_RECENCY_LAMBDA_DECISION
        );
        assert_eq!(
            FusionConfig::default_recency_lambda_for_intent("law_check"),
            DEFAULT_RECENCY_LAMBDA_LAW
        );
        // Unknown intents fall back to 0 (no decay) — the safe
        // legacy behaviour.
        assert_eq!(
            FusionConfig::default_recency_lambda_for_intent("invented"),
            0.0
        );
    }

    fn hit_with_repo(id: &str, repo: &str, native: f64) -> LaneHit {
        let mut h = hit_with_score(id, native);
        h.repo = Some(repo.to_string());
        h
    }

    #[test]
    fn cross_repo_boost_zero_collapses_foreign_hits_to_zero() {
        // active_repo="cortex", boost=0.0 — every "vectorizer" hit
        // gets multiplied by 0.0 and lands at the tail.
        let lane = vec![
            hit_with_repo("V1", "vectorizer", 0.9),
            hit_with_repo("C1", "cortex", 0.5),
            hit_with_repo("V2", "vectorizer", 0.85),
        ];
        let cfg = FusionConfig::default()
            .with_cross_repo_boost(0.0, Some("cortex".to_string()));
        let fused = rrf_fuse(vec![lane], &cfg);
        // C1 must land first; the two vectorizer hits trail with score=0.
        assert_eq!(fused[0].doc_id, "C1");
        for h in &fused[1..] {
            assert_eq!(h.score, 0.0, "cross-repo hit at boost=0 must score 0");
        }
    }

    #[test]
    fn cross_repo_boost_half_demotes_but_keeps_foreign_hits() {
        // boost=0.5 — a strong foreign hit can still compete with
        // a weaker in-repo hit, but the multiplier shrinks its score.
        let lane = vec![
            hit_with_repo("V1", "vectorizer", 0.95), // rank 1, foreign
            hit_with_repo("C1", "cortex", 0.50),     // rank 2, in-repo
        ];
        let cfg = FusionConfig {
            alpha: 1.0, // pure positional so the analysis is exact
            k: 60,
            recency_decay_lambda: 0.0,
            now_ms: 0,
            cross_repo_boost: 0.5,
            active_repo: Some("cortex".to_string()),
        };
        let fused = rrf_fuse(vec![lane], &cfg);
        // V1 baseline = 1/61 ≈ 0.01639; multiplied by 0.5 → 0.00820.
        // C1 baseline = 1/62 ≈ 0.01613; no multiplier.
        // C1 must outrank V1.
        assert_eq!(fused[0].doc_id, "C1");
        assert_eq!(fused[1].doc_id, "V1");
        assert!(fused[1].score < fused[0].score);
    }

    #[test]
    fn cross_repo_boost_one_leaves_scores_unchanged() {
        // boost=1.0 ⇒ no change; foreign hits compete on equal
        // footing. Pin against the no-boost (default) ordering.
        let lane = vec![
            hit_with_repo("V1", "vectorizer", 0.95),
            hit_with_repo("C1", "cortex", 0.50),
        ];
        let baseline = rrf_fuse(
            vec![lane.clone()],
            &FusionConfig {
                alpha: 1.0,
                k: 60,
                recency_decay_lambda: 0.0,
                now_ms: 0,
                cross_repo_boost: 0.0,
                active_repo: None, // no active repo ⇒ boost no-op
            },
        );
        let with_full_boost = rrf_fuse(
            vec![lane],
            &FusionConfig {
                alpha: 1.0,
                k: 60,
                recency_decay_lambda: 0.0,
                now_ms: 0,
                cross_repo_boost: 1.0,
                active_repo: Some("cortex".to_string()),
            },
        );
        // Same ordering and same scores within float epsilon.
        assert_eq!(baseline.len(), with_full_boost.len());
        for (a, b) in baseline.iter().zip(with_full_boost.iter()) {
            assert_eq!(a.doc_id, b.doc_id);
            assert!((a.score - b.score).abs() < 1e-9);
        }
    }

    #[test]
    fn cross_repo_boost_with_empty_active_repo_is_a_noop() {
        // Empty-string active_repo (a canonicalisation accident
        // upstream) must not cause every hit to be classified as
        // foreign. Builder filters it to None; the multiplier
        // path observes None and skips entirely.
        let cfg = FusionConfig::default()
            .with_cross_repo_boost(0.0, Some(String::new()));
        assert!(cfg.active_repo.is_none());
        let lane = vec![
            hit_with_repo("V1", "vectorizer", 0.9),
            hit_with_repo("C1", "cortex", 0.5),
        ];
        let fused = rrf_fuse(vec![lane], &cfg);
        for h in &fused {
            assert!(h.score > 0.0, "no hit should be zeroed out");
        }
    }

    #[test]
    fn cross_repo_boost_clamps_out_of_range_inputs() {
        let cfg = FusionConfig::default()
            .with_cross_repo_boost(2.5, Some("cortex".to_string()));
        assert_eq!(cfg.cross_repo_boost, 1.0);
        let cfg = FusionConfig::default()
            .with_cross_repo_boost(-0.4, Some("cortex".to_string()));
        assert_eq!(cfg.cross_repo_boost, 0.0);
    }

    #[test]
    fn far_future_ts_does_not_panic_or_invert() {
        // A hit whose ts is in the future (clock skew, replay)
        // computes `days_old = max(0, …)` so the multiplier is 1.0;
        // we never multiply by exp(+positive) and inflate the score.
        let now_ms = 1_000_000_000;
        let future = hit_with_ts("FUTURE", now_ms + 99_000_000_000);
        let cfg = FusionConfig {
            alpha: 1.0,
            k: 60,
            recency_decay_lambda: 0.05,
            now_ms,
            cross_repo_boost: 0.0,
            active_repo: None,
        };
        let fused = rrf_fuse(vec![vec![future]], &cfg);
        // Score should equal positional (1/61) — no decay applied.
        let positional = 1.0_f64 / 61.0;
        assert!(
            (fused[0].score - positional).abs() < 1e-9,
            "future ts must not inflate score; got {}",
            fused[0].score
        );
    }
}
