//! Phase14c retrieval suite — MRR@10 + recall@5 against
//! `cortex-api /v1/query`. Golden CSV shape:
//!
//! ```csv
//! id,query,repo,expected_paths
//! r-001,"how does tier sweep work","cortex","crates/.../sweep.rs;docs/specs/19-retention.md"
//! ```
//!
//! `expected_paths` is a `;`-delimited list (CSV-safe). phase0 2026-06-22
//! — the live `/v1/query` returns `results.snippets[]` keyed by `path`
//! (+ `content_hash`), NOT `event_id`; the golden set + driver were
//! re-keyed to `path`. The metrics (`mrr_at_k`, `recall_at_k`) are
//! identity-agnostic string matchers, so only the column name + the
//! driver's hit extraction changed.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::{mrr_at_k, recall_at_k};
use crate::report::{MetricRow, SuiteReport};
use crate::suite::AcceptanceVerdict;

/// One row in `tests/golden/retrieval.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RetrievalRow {
    /// Stable row id — feeds [`SuiteReport::per_row`].
    pub id: String,
    /// Free-text query the harness sends to `/v1/query`.
    pub query: String,
    /// Optional repo scope. Empty means cross-repo.
    #[serde(default)]
    pub repo: String,
    /// `;`-delimited list of repo-relative snippet paths the row
    /// expects to surface in the top-10 results. Parsed via
    /// [`RetrievalRow::expected_paths`].
    pub expected_paths: String,
}

impl RetrievalRow {
    /// Parse the `;`-delimited expected-path column into a vec,
    /// trimming whitespace and dropping empty entries.
    pub fn expected_paths(&self) -> Vec<String> {
        self.expected_paths
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Acceptance floors per the phase14c proposal.
pub const MRR_AT_10_FLOOR: f64 = 0.60;
/// Acceptance floor for recall@5.
pub const RECALL_AT_5_FLOOR: f64 = 0.50;

/// Phase14c k for MRR.
pub const MRR_K: usize = 10;
/// Phase14c k for recall.
pub const RECALL_K: usize = 5;

/// Walk a [`SuiteReport`] and return a verdict against the
/// retrieval-suite floors.
pub fn retrieval_acceptance(report: &SuiteReport) -> AcceptanceVerdict {
    let mut failed = Vec::new();
    if report
        .metric("mrr_at_10")
        .map(|m| m.value < MRR_AT_10_FLOOR)
        .unwrap_or(true)
    {
        failed.push("mrr_at_10".into());
    }
    if report
        .metric("recall_at_5")
        .map(|m| m.value < RECALL_AT_5_FLOOR)
        .unwrap_or(true)
    {
        failed.push("recall_at_5".into());
    }
    AcceptanceVerdict {
        passed: failed.is_empty(),
        failed_metrics: failed,
    }
}

/// Read every [`RetrievalRow`] out of the golden CSV at `path`.
pub fn load_csv(path: &std::path::Path) -> Result<Vec<RetrievalRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("open retrieval golden csv {}", path.display()))?;
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        let r: RetrievalRow = row.context("parse retrieval csv row")?;
        out.push(r);
    }
    Ok(out)
}

/// Build a [`SuiteReport`] from per-row observations. Each row
/// supplies the ranked list it actually got back from `/v1/query`.
pub fn build_report(rows: &[RetrievalRow], observed: &[Vec<String>]) -> SuiteReport {
    assert_eq!(
        rows.len(),
        observed.len(),
        "row/observation length mismatch"
    );
    let mut sum_mrr = 0.0;
    let mut sum_recall = 0.0;
    let mut per_row = std::collections::BTreeMap::new();
    for (row, obs) in rows.iter().zip(observed.iter()) {
        let expected = row.expected_paths();
        let row_mrr = mrr_at_k(obs, &expected, MRR_K);
        let row_recall = recall_at_k(obs, &expected, RECALL_K);
        sum_mrr += row_mrr;
        sum_recall += row_recall;
        per_row.insert(
            row.id.clone(),
            serde_json::json!({
                "mrr_at_10": row_mrr,
                "recall_at_5": row_recall,
                "expected": expected,
                "observed_top_k": obs.iter().take(MRR_K).collect::<Vec<_>>(),
            }),
        );
    }
    let n = rows.len().max(1) as f64;
    let mrr = sum_mrr / n;
    let recall = sum_recall / n;
    SuiteReport {
        suite: "retrieval".into(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        rows_total: rows.len() as u32,
        rows_errored: 0,
        metrics: vec![
            MetricRow {
                name: "mrr_at_10".into(),
                value: mrr,
                floor: Some(MRR_AT_10_FLOOR),
                passed: mrr >= MRR_AT_10_FLOOR,
            },
            MetricRow {
                name: "recall_at_5".into(),
                value: recall,
                floor: Some(RECALL_AT_5_FLOOR),
                passed: recall >= RECALL_AT_5_FLOOR,
            },
        ],
        per_row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_paths_splits_and_trims() {
        let row = RetrievalRow {
            id: "r1".into(),
            query: "q".into(),
            repo: "".into(),
            expected_paths:"a; b ; ; c".into(),
        };
        assert_eq!(row.expected_paths(), vec!["a", "b", "c"]);
    }

    #[test]
    fn build_report_perfect_match_passes_floor() {
        let rows = vec![RetrievalRow {
            id: "r1".into(),
            query: "q".into(),
            repo: "".into(),
            expected_paths:"a".into(),
        }];
        let observed = vec![vec!["a".to_string(), "b".to_string()]];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("mrr_at_10").unwrap().value, 1.0);
        assert_eq!(r.metric("recall_at_5").unwrap().value, 1.0);
        let verdict = retrieval_acceptance(&r);
        assert!(verdict.passed);
        assert!(verdict.failed_metrics.is_empty());
    }

    #[test]
    fn build_report_no_match_fails_acceptance() {
        let rows = vec![RetrievalRow {
            id: "r1".into(),
            query: "q".into(),
            repo: "".into(),
            expected_paths:"a".into(),
        }];
        let observed = vec![vec!["x".to_string(), "y".to_string()]];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("mrr_at_10").unwrap().value, 0.0);
        let verdict = retrieval_acceptance(&r);
        assert!(!verdict.passed);
        assert_eq!(verdict.failed_metrics.len(), 2);
    }

    #[test]
    fn load_csv_round_trips_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retrieval.csv");
        std::fs::write(
            &path,
            "id,query,repo,expected_paths\n\
             r1,how does tier sweep work,cortex,crates/a/sweep.rs;docs/specs/19.md\n\
             r2,what is ADR-013,cortex,docs/specs/02.md\n",
        )
        .unwrap();
        let rows = load_csv(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "r1");
        assert_eq!(
            rows[0].expected_paths(),
            vec!["crates/a/sweep.rs", "docs/specs/19.md"]
        );
    }
}
