//! Phase14c metric primitives. Pure-Rust, no IO, fully unit-tested.
//!
//! Every function returns `f64 ∈ [0.0, 1.0]`. Inputs that violate
//! the contract (negative k, empty expected set) return `0.0`
//! rather than panicking — eval harnesses run unattended in CI and
//! a panic would block the gate without surfacing the data shape
//! issue.

/// Mean Reciprocal Rank at k. For a single query, the reciprocal
/// rank is `1 / (rank + 1)` of the first relevant hit within the
/// top-k results, or `0` when no relevant hit appears. The
/// returned value here is the per-query reciprocal rank — callers
/// average across queries to compute the suite-level MRR@k.
pub fn mrr_at_k(ranked_results: &[String], relevant: &[String], k: usize) -> f64 {
    if k == 0 || ranked_results.is_empty() || relevant.is_empty() {
        return 0.0;
    }
    for (idx, hit) in ranked_results.iter().take(k).enumerate() {
        if relevant.iter().any(|r| r == hit) {
            // `+ 1` so the top hit (idx 0) scores 1.0.
            return 1.0 / ((idx + 1) as f64);
        }
    }
    0.0
}

/// Recall at k — fraction of `relevant` items that show up in the
/// top-k ranked results. Returns `0.0` when `relevant` is empty
/// (no expected hits means recall is undefined; reporting 0 keeps
/// the metric aggregable).
pub fn recall_at_k(ranked_results: &[String], relevant: &[String], k: usize) -> f64 {
    if k == 0 || relevant.is_empty() {
        return 0.0;
    }
    let top: std::collections::HashSet<&String> = ranked_results.iter().take(k).collect();
    let hits = relevant.iter().filter(|r| top.contains(r)).count();
    hits as f64 / relevant.len() as f64
}

/// Per-class F1 = `2*P*R / (P+R)`. Inputs are confusion-matrix
/// counts. Returns 0.0 when both precision and recall are 0.
pub fn f1(true_positive: usize, false_positive: usize, false_negative: usize) -> f64 {
    if true_positive == 0 {
        return 0.0;
    }
    let precision = true_positive as f64 / (true_positive + false_positive) as f64;
    let recall = true_positive as f64 / (true_positive + false_negative) as f64;
    if (precision + recall).abs() < f64::EPSILON {
        return 0.0;
    }
    2.0 * precision * recall / (precision + recall)
}

/// Macro-F1 = unweighted mean of per-class F1. `per_class` carries
/// `(label, (tp, fp, fn))` triplets. Empty input → 0.0.
pub fn macro_f1(per_class: &[(String, (usize, usize, usize))]) -> f64 {
    if per_class.is_empty() {
        return 0.0;
    }
    let sum: f64 = per_class
        .iter()
        .map(|(_, (tp, fp, fn_))| f1(*tp, *fp, *fn_))
        .sum();
    sum / per_class.len() as f64
}

/// Set-based recall — fraction of `expected` items present in
/// `observed`. Used by the consolidation suite (entity-recall +
/// fact-recall). `expected.is_empty()` returns 1.0 (vacuously
/// satisfied — no entities expected means the row passes).
pub fn set_recall(observed: &[String], expected: &[String]) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let observed_set: std::collections::HashSet<&String> = observed.iter().collect();
    let hits = expected.iter().filter(|e| observed_set.contains(e)).count();
    hits as f64 / expected.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn mrr_top_hit_is_one() {
        assert!((mrr_at_k(&s(&["a", "b"]), &s(&["a"]), 10) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mrr_third_hit_is_one_third() {
        assert!((mrr_at_k(&s(&["x", "y", "a"]), &s(&["a"]), 10) - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn mrr_no_match_is_zero() {
        assert_eq!(mrr_at_k(&s(&["x", "y"]), &s(&["a"]), 10), 0.0);
    }

    #[test]
    fn mrr_empty_inputs_return_zero() {
        assert_eq!(mrr_at_k(&[], &s(&["a"]), 10), 0.0);
        assert_eq!(mrr_at_k(&s(&["a"]), &[], 10), 0.0);
        assert_eq!(mrr_at_k(&s(&["a"]), &s(&["a"]), 0), 0.0);
    }

    #[test]
    fn recall_top_k_picks_two_of_three() {
        let observed = s(&["a", "b", "c", "d"]);
        let expected = s(&["a", "b", "z"]);
        assert!((recall_at_k(&observed, &expected, 4) - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn recall_caps_at_one() {
        let observed = s(&["a", "b"]);
        let expected = s(&["a", "b"]);
        assert!((recall_at_k(&observed, &expected, 5) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn f1_balanced_case() {
        // Precision = 4/5, Recall = 4/6 → F1 ≈ 0.7272
        let v = f1(4, 1, 2);
        assert!((v - 0.7272727272727273).abs() < 1e-9);
    }

    #[test]
    fn f1_zero_when_no_true_positives() {
        assert_eq!(f1(0, 5, 5), 0.0);
    }

    #[test]
    fn macro_f1_averages_per_class() {
        let per = vec![("a".to_string(), (4, 1, 2)), ("b".to_string(), (3, 0, 1))];
        let m = macro_f1(&per);
        // F1_a = 8/11 = 0.7272…, F1_b = 6/7 = 0.8571…, mean = 61/77 ≈ 0.7922077922
        assert!((m - 61.0 / 77.0).abs() < 1e-9);
    }

    #[test]
    fn set_recall_empty_expected_is_vacuous_one() {
        assert!((set_recall(&s(&["x"]), &[]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn set_recall_partial_match() {
        let r = set_recall(&s(&["a", "b", "z"]), &s(&["a", "b", "c"]));
        assert!((r - 2.0 / 3.0).abs() < 1e-9);
    }
}
