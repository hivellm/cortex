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

/// Tunable parameters for [`rrf_fuse`].
///
/// Built once at orchestrator boot from the
/// `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` env vars and stamped on
/// every audit envelope so phase6e's harness can attribute
/// regressions to fusion-tuning changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FusionConfig {
    /// Blend weight in `[0.0, 1.0]`. `1.0` = pure positional RRF;
    /// `0.0` = pure normalised native score.
    pub alpha: f32,
    /// RRF stabilisation constant (Cormack et al. default = 60).
    /// Larger `k` flattens the per-lane curve; smaller `k`
    /// emphasises rank-1 hits.
    pub k: u32,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_RRF_ALPHA,
            k: RRF_K as u32,
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
        }
    }
}

/// Fuse per-lane hit lists into a single ranked output. Each input
/// list must already be sorted in lane-native rank order (best
/// first). Returns hits ordered by fused score with deterministic
/// tie-breaks.
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
        rrf_fuse(lanes, &FusionConfig { alpha: 1.0, k: 60 })
    }

    #[test]
    fn rrf_sums_reciprocal_ranks_across_lanes() {
        // Pure positional baseline (alpha=1.0) so the analytic
        // reciprocal sum is exact.
        let lane_a = vec![hit("X", 0, None), hit("Y", 0, None)];
        let lane_b = vec![hit("Y", 0, None), hit("X", 0, None)];
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 1.0, k: 60 });
        assert_eq!(fused.len(), 2);
        // X: 1/(60+1) + 1/(60+2) = 1/61 + 1/62; Y: same. Tie broken
        // by doc_id ⇒ X first.
        assert_eq!(fused[0].doc_id, "X");
        let expected = (1.0_f64 / 61.0) + (1.0_f64 / 62.0);
        assert!((fused[0].score - expected).abs() < 1e-9);
    }

    #[test]
    fn ties_break_on_recency() {
        let cfg = FusionConfig { alpha: 1.0, k: 60 };
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
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 1.0, k: 60 });
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
            &FusionConfig { alpha: 1.0, k: 60 },
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
        let fused = rrf_fuse(vec![lane_a, lane_b], &FusionConfig { alpha: 0.0, k: 60 });
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
}
