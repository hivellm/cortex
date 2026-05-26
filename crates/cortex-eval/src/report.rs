//! Phase14c — suite report shapes + JSON / markdown rendering.
//!
//! Every suite emits a [`SuiteReport`] with per-row diagnostics +
//! aggregate metrics. The CI gate compares a fresh report against a
//! baseline (a JSON-serialised prior report from main) using the
//! [`crate::CI_REGRESSION_THRESHOLD`] envelope.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One aggregate metric carried on a [`SuiteReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricRow {
    /// Metric name (`mrr_at_10`, `recall_at_5`, `entity_recall`,
    /// `macro_f1`, etc).
    pub name: String,
    /// Observed value in `[0.0, 1.0]`.
    pub value: f64,
    /// Acceptance floor pinned by the suite spec — `Some(_)` for
    /// hard floors, `None` for advisory metrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<f64>,
    /// `true` when `floor` exists and `value >= floor`.
    pub passed: bool,
}

/// One suite's aggregate report. Carries enough context for the CI
/// gate to compute regression vs a prior baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    /// Suite name (`retrieval`, `consolidation`, `classification`).
    pub suite: String,
    /// RFC-3339 timestamp the run finished at.
    pub finished_at: String,
    /// Total rows evaluated.
    pub rows_total: u32,
    /// Rows that errored out before metrics could be computed
    /// (network failures, missing fixtures, etc.).
    pub rows_errored: u32,
    /// Aggregate metrics in stable insertion order. Use
    /// [`SuiteReport::metric`] for read access.
    pub metrics: Vec<MetricRow>,
    /// Free-form per-row diagnostics — keyed by row id, useful for
    /// debug printouts. CI typically drops this before persisting
    /// the baseline.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_row: BTreeMap<String, serde_json::Value>,
}

impl SuiteReport {
    /// Lookup helper — returns the first metric matching `name`.
    pub fn metric(&self, name: &str) -> Option<&MetricRow> {
        self.metrics.iter().find(|m| m.name == name)
    }

    /// Render as markdown — used by the CLI's default text output.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# cortex-eval — {} suite\n\nFinished: {}\nRows: {} (errored: {})\n\n",
            self.suite, self.finished_at, self.rows_total, self.rows_errored
        ));
        out.push_str("| metric | value | floor | passed |\n");
        out.push_str("|---|---|---|---|\n");
        for m in &self.metrics {
            let floor = m
                .floor
                .map(|f| format!("{f:.3}"))
                .unwrap_or_else(|| "—".into());
            let mark = if m.passed { "✅" } else { "❌" };
            out.push_str(&format!(
                "| {} | {:.4} | {} | {} |\n",
                m.name, m.value, floor, mark
            ));
        }
        out
    }
}

/// Compute the absolute drop between two metric values. Positive
/// means `current` regressed below `baseline`; negative means
/// improvement. NaN-safe.
pub fn metric_delta(baseline: f64, current: f64) -> f64 {
    baseline - current
}

/// `true` when `current` regressed by more than `threshold` versus
/// `baseline`. NaN inputs default to "no regression" (false) so a
/// missing baseline does not block the gate.
pub fn is_regression(baseline: f64, current: f64, threshold: f64) -> bool {
    if baseline.is_nan() || current.is_nan() {
        return false;
    }
    metric_delta(baseline, current) > threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SuiteReport {
        SuiteReport {
            suite: "retrieval".into(),
            finished_at: "2026-05-25T12:00:00Z".into(),
            rows_total: 100,
            rows_errored: 0,
            metrics: vec![
                MetricRow {
                    name: "mrr_at_10".into(),
                    value: 0.62,
                    floor: Some(0.60),
                    passed: true,
                },
                MetricRow {
                    name: "recall_at_5".into(),
                    value: 0.55,
                    floor: Some(0.50),
                    passed: true,
                },
            ],
            per_row: Default::default(),
        }
    }

    #[test]
    fn round_trips_via_serde() {
        let r = sample();
        let json = serde_json::to_string(&r).unwrap();
        let back: SuiteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn metric_lookup_finds_named_row() {
        let r = sample();
        assert_eq!(r.metric("mrr_at_10").unwrap().value, 0.62);
        assert!(r.metric("nonexistent").is_none());
    }

    #[test]
    fn is_regression_flags_drop_above_threshold() {
        assert!(is_regression(0.60, 0.50, 0.05)); // 0.10 drop > 0.05
        assert!(!is_regression(0.60, 0.58, 0.05)); // 0.02 drop ≤ 0.05
        assert!(!is_regression(0.60, 0.62, 0.05)); // improvement
    }

    #[test]
    fn is_regression_tolerates_nan() {
        assert!(!is_regression(f64::NAN, 0.50, 0.05));
        assert!(!is_regression(0.60, f64::NAN, 0.05));
    }

    #[test]
    fn markdown_table_carries_pass_markers() {
        let md = sample().to_markdown();
        assert!(md.contains("mrr_at_10"));
        assert!(md.contains("✅"));
    }
}
