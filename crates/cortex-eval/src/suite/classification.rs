//! Phase14c classification suite — per-kind F1 + macro-F1 against
//! the classifier worker. Golden CSV shape:
//!
//! ```csv
//! id,envelope_json,expected_kind
//! cl-001,"{\"tool\":\"claude-code\",\"payload\":{}}","Turn"
//! ```
//!
//! `envelope_json` is the wire shape the classifier receives; the
//! harness POSTs it to the classifier endpoint and compares the
//! returned kind string against `expected_kind`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::{f1, macro_f1};
use crate::report::{MetricRow, SuiteReport};
use crate::suite::AcceptanceVerdict;

/// One row in `tests/golden/classification.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ClassificationRow {
    /// Stable row id.
    pub id: String,
    /// JSON envelope the classifier reads. Stored as a raw string
    /// so the CSV stays single-line.
    pub envelope_json: String,
    /// Canonical Kind label the classifier MUST return (e.g.
    /// `Turn`, `ToolCall`, `Decision`).
    pub expected_kind: String,
}

/// Acceptance floor for macro-F1 per the phase14c proposal.
pub const MACRO_F1_FLOOR: f64 = 0.90;

/// Phase14c §3.4 acceptance gate for the classification suite.
pub fn classification_acceptance(report: &SuiteReport) -> AcceptanceVerdict {
    let mut failed = Vec::new();
    if report
        .metric("macro_f1")
        .map(|m| m.value < MACRO_F1_FLOOR)
        .unwrap_or(true)
    {
        failed.push("macro_f1".into());
    }
    AcceptanceVerdict {
        passed: failed.is_empty(),
        failed_metrics: failed,
    }
}

/// Read every [`ClassificationRow`] out of the golden CSV at `path`.
pub fn load_csv(path: &std::path::Path) -> Result<Vec<ClassificationRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("open classification golden csv {}", path.display()))?;
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        let r: ClassificationRow = row.context("parse classification csv row")?;
        out.push(r);
    }
    Ok(out)
}

/// Build a [`SuiteReport`] from per-row observations. `predicted`
/// supplies the kind string the classifier actually returned per
/// row, in the same order as `rows`.
pub fn build_report(rows: &[ClassificationRow], predicted: &[String]) -> SuiteReport {
    assert_eq!(
        rows.len(),
        predicted.len(),
        "row/prediction length mismatch"
    );
    // Collect the union of labels so we compute F1 over every kind
    // that appears in expected OR predicted.
    let mut labels: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in rows {
        labels.insert(r.expected_kind.clone());
    }
    for p in predicted {
        labels.insert(p.clone());
    }
    // Confusion-matrix counters per label.
    let mut counters: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for label in &labels {
        counters.insert(label.clone(), (0, 0, 0));
    }
    for (row, pred) in rows.iter().zip(predicted.iter()) {
        if &row.expected_kind == pred {
            counters.get_mut(pred).expect("seeded").0 += 1; // tp
        } else {
            // pred → false positive on pred label
            counters.get_mut(pred).expect("seeded").1 += 1;
            // expected → false negative on expected label
            counters.get_mut(&row.expected_kind).expect("seeded").2 += 1;
        }
    }
    let per_class: Vec<(String, (usize, usize, usize))> =
        counters.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let macro_score = macro_f1(&per_class);

    // Per-label F1 emitted as advisory metrics so the dashboard can
    // surface per-kind weakness without redefining the floor.
    let mut metrics = Vec::with_capacity(per_class.len() + 1);
    metrics.push(MetricRow {
        name: "macro_f1".into(),
        value: macro_score,
        floor: Some(MACRO_F1_FLOOR),
        passed: macro_score >= MACRO_F1_FLOOR,
    });
    for (label, (tp, fp, fn_)) in &per_class {
        let v = f1(*tp, *fp, *fn_);
        metrics.push(MetricRow {
            name: format!("f1.{label}"),
            value: v,
            floor: None,
            passed: true,
        });
    }
    let mut per_row = BTreeMap::new();
    for (row, pred) in rows.iter().zip(predicted.iter()) {
        per_row.insert(
            row.id.clone(),
            serde_json::json!({
                "expected": row.expected_kind,
                "predicted": pred,
                "ok": &row.expected_kind == pred,
            }),
        );
    }
    SuiteReport {
        suite: "classification".into(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        rows_total: rows.len() as u32,
        rows_errored: 0,
        metrics,
        per_row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: &str) -> ClassificationRow {
        ClassificationRow {
            id: id.into(),
            envelope_json: format!("{{\"kind\":\"{kind}\"}}"),
            expected_kind: kind.into(),
        }
    }

    #[test]
    fn build_report_perfect_predictions_pass() {
        let rows = vec![row("a", "Turn"), row("b", "ToolCall"), row("c", "Turn")];
        let preds = vec!["Turn".into(), "ToolCall".into(), "Turn".into()];
        let r = build_report(&rows, &preds);
        assert_eq!(r.metric("macro_f1").unwrap().value, 1.0);
        let v = classification_acceptance(&r);
        assert!(v.passed);
    }

    #[test]
    fn build_report_one_misclass_drops_macro_f1() {
        // 3 rows; 1 misclass → some labels at <1.0 → macro_f1 < 1.0.
        let rows = vec![row("a", "Turn"), row("b", "ToolCall"), row("c", "Decision")];
        let preds = vec!["Turn".into(), "Turn".into(), "Decision".into()];
        let r = build_report(&rows, &preds);
        let m = r.metric("macro_f1").unwrap().value;
        assert!(m < 1.0 && m > 0.0);
    }

    #[test]
    fn empty_input_yields_zero_macro_f1() {
        let r = build_report(&[], &[]);
        assert_eq!(r.metric("macro_f1").unwrap().value, 0.0);
        let v = classification_acceptance(&r);
        assert!(!v.passed);
    }
}
