//! Reciprocal Rank Fusion. Spec 11 §Fan-out + fusion:
//! `score(d) = Σ 1 / (60 + rank_lane(d))`. Ties break on recency
//! (higher `ts` first), then severity (`critical > notable > info`).

use std::collections::BTreeMap;

use crate::lanes::LaneHit;

/// Standard RRF k-constant from Cormack et al. 2009.
pub const RRF_K: f64 = 60.0;

/// Fuse per-lane hit lists into a single ranked output. Each input
/// list must already be sorted in lane-native rank order (best
/// first). Returns hits ordered by fused score with deterministic
/// tie-breaks.
pub fn rrf_fuse(lanes: Vec<Vec<LaneHit>>) -> Vec<LaneHit> {
    let mut scores: BTreeMap<String, f64> = BTreeMap::new();
    let mut representative: BTreeMap<String, LaneHit> = BTreeMap::new();
    for lane in &lanes {
        for (idx, hit) in lane.iter().enumerate() {
            let rank = idx + 1;
            let score = 1.0 / (RRF_K + rank as f64);
            *scores.entry(hit.doc_id.clone()).or_insert(0.0) += score;
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

    #[test]
    fn rrf_sums_reciprocal_ranks_across_lanes() {
        let lane_a = vec![hit("X", 0, None), hit("Y", 0, None)];
        let lane_b = vec![hit("Y", 0, None), hit("X", 0, None)];
        let fused = rrf_fuse(vec![lane_a, lane_b]);
        assert_eq!(fused.len(), 2);
        // X: 1/(60+1) + 1/(60+2) = 1/61 + 1/62; Y: same. Tie broken
        // by doc_id ⇒ X first.
        assert_eq!(fused[0].doc_id, "X");
        let expected = (1.0_f64 / 61.0) + (1.0_f64 / 62.0);
        assert!((fused[0].score - expected).abs() < 1e-9);
    }

    #[test]
    fn ties_break_on_recency() {
        let lane_a = vec![hit("OLD", 100, None), hit("NEW", 200, None)];
        let lane_b = vec![hit("OLD", 100, None), hit("NEW", 200, None)];
        let fused = rrf_fuse(vec![lane_a, lane_b]);
        assert_eq!(fused[0].doc_id, "OLD", "lane-rank wins over recency");
        // Same lane-rank ⇒ recency would be the tie-break, but here
        // the first lane both has OLD ahead of NEW, so OLD wins.
        // Test the recency tie-break with two equally-ranked hits.
        let lane_c = vec![hit("OLD", 100, None)];
        let lane_d = vec![hit("NEW", 200, None)];
        let fused2 = rrf_fuse(vec![lane_c, lane_d]);
        assert_eq!(fused2[0].doc_id, "NEW");
    }

    #[test]
    fn ties_break_on_severity_after_recency() {
        let lane_a = vec![hit("INFO", 100, Some("info"))];
        let lane_b = vec![hit("CRIT", 100, Some("critical"))];
        let fused = rrf_fuse(vec![lane_a, lane_b]);
        assert_eq!(fused[0].doc_id, "CRIT");
    }

    #[test]
    fn empty_lanes_produce_empty_output() {
        assert!(rrf_fuse(vec![vec![], vec![]]).is_empty());
    }
}
