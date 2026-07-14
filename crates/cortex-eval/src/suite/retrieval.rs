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

/// One row in `crates/cortex-eval/tests/golden/retrieval.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RetrievalRow {
    /// Stable row id — feeds [`SuiteReport::per_row`].
    pub id: String,
    /// Free-text query the harness sends to `/v1/query`.
    pub query: String,
    /// Optional repo scope. Empty means cross-repo.
    #[serde(default)]
    pub repo: String,
    /// Query intent the row exercises — one of the five `/v1/query`
    /// intents (`pre_change_context`, `decision_lookup`,
    /// `similar_problems`, `law_check`, `free_search`). Empty defaults
    /// to `free_search` so pre-phase28 four-column fixtures keep
    /// loading (phase28 retrieval-eval-gate-live §2.2).
    #[serde(default)]
    pub intent: String,
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

    /// The `/v1/query` intent this row exercises; defaults to
    /// `free_search` when the column is empty.
    pub fn intent(&self) -> &str {
        let trimmed = self.intent.trim();
        if trimmed.is_empty() {
            "free_search"
        } else {
            trimmed
        }
    }
}

/// Acceptance floor for MRR@10. Phase28 (retrieval-eval-gate-live
/// §4.1/§6) — re-derived from the 2026-07-14 baseline of the 27-row
/// multi-intent golden set, then widened for LIVE-CORPUS DRIFT: the
/// corpus this suite measures grows while the maintainer works (a
/// session's own tool_call events route into the code family and
/// displace results — the same golden set measured 0.5864 → 0.5123
/// within one working day). Floor = drift-band low minus tolerance.
/// The original 0.60 was calibrated for the 18-row free_search-only
/// set (0.6315) and would have permanently failed the harder
/// intent-diverse set.
pub const MRR_AT_10_FLOOR: f64 = 0.45;
/// Acceptance floor for recall@5 — drift band 0.5185–0.5926 measured
/// on 2026-07-14, floor = band low minus tolerance (was 0.50,
/// calibrated pre-phase28).
pub const RECALL_AT_5_FLOOR: f64 = 0.45;

/// Phase14c k for MRR.
pub const MRR_K: usize = 10;
/// Phase14c k for recall.
pub const RECALL_K: usize = 5;

/// Phase28 (retrieval-eval-gate-live §6.2) — ADR-026 / spec 28 §3.10
/// gate: at most 1% of the verification-eligible snippets a retrieval
/// run returns may be phantom links (`verified == false`).
pub const PHANTOM_LINK_RATE_GATE: f64 = 0.01;

/// Phase28 §6.2 — build the `phantom_link_rate` metric row from the
/// verifier stamps observed across a retrieval run. `verified_true` /
/// `verified_false` count snippets whose `verified` field was
/// `Some(..)`; snippets with `verified == None` (verifier not run for
/// that hit) are excluded from the denominator. Returns `None` when
/// the verifier stamped nothing at all (verification disabled in the
/// target deployment) so the metric never reports a fake 0%.
pub fn phantom_metric(verified_true: u64, verified_false: u64) -> Option<MetricRow> {
    let total = verified_true + verified_false;
    if total == 0 {
        return None;
    }
    let rate = verified_false as f64 / total as f64;
    Some(MetricRow {
        name: "phantom_link_rate".into(),
        value: rate,
        // `floor` semantics are "value must be >= floor", which is
        // inverted for a rate that must stay LOW — leave floor unset
        // and carry the verdict in `passed` (retrieval_acceptance
        // honours it).
        floor: None,
        passed: rate <= PHANTOM_LINK_RATE_GATE,
    })
}

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
    // Phase28 §6.2 — the phantom-link gate only participates when the
    // metric is present (the driver omits it when the target
    // deployment has verification disabled).
    if report
        .metric("phantom_link_rate")
        .map(|m| !m.passed)
        .unwrap_or(false)
    {
        failed.push("phantom_link_rate".into());
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
            intent: "".into(),
            expected_paths: "a; b ; ; c".into(),
        };
        assert_eq!(row.expected_paths(), vec!["a", "b", "c"]);
    }

    #[test]
    fn build_report_perfect_match_passes_floor() {
        let rows = vec![RetrievalRow {
            id: "r1".into(),
            query: "q".into(),
            repo: "".into(),
            intent: "".into(),
            expected_paths: "a".into(),
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
            intent: "".into(),
            expected_paths: "a".into(),
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
        // Pre-phase28 four-column fixture (no `intent` header) —
        // defaults to free_search.
        assert_eq!(rows[0].intent(), "free_search");
    }

    // ── Phase28 §6.2 — phantom_link_rate metric ──────────────────────

    #[test]
    fn phantom_metric_none_when_verifier_stamped_nothing() {
        assert!(phantom_metric(0, 0).is_none(), "no stamps → no metric");
    }

    #[test]
    fn phantom_metric_passes_at_or_below_one_percent() {
        let m = phantom_metric(99, 1).unwrap();
        assert!((m.value - 0.01).abs() < 1e-9);
        assert!(m.passed, "exactly 1% is within the gate");
        let clean = phantom_metric(50, 0).unwrap();
        assert_eq!(clean.value, 0.0);
        assert!(clean.passed);
    }

    #[test]
    fn phantom_metric_fails_above_one_percent_and_gates_acceptance() {
        let m = phantom_metric(90, 10).unwrap();
        assert!((m.value - 0.1).abs() < 1e-9);
        assert!(!m.passed);
        // A report that clears the mrr/recall floors but carries a
        // failing phantom metric must fail acceptance.
        let rows = vec![RetrievalRow {
            id: "r1".into(),
            query: "q".into(),
            repo: "".into(),
            intent: "".into(),
            expected_paths: "a".into(),
        }];
        let observed = vec![vec!["a".to_string()]];
        let mut report = build_report(&rows, &observed);
        report.metrics.push(m);
        let verdict = retrieval_acceptance(&report);
        assert!(!verdict.passed);
        assert!(verdict
            .failed_metrics
            .contains(&"phantom_link_rate".to_string()));
    }

    #[test]
    fn load_csv_reads_intent_column_and_defaults_blank_to_free_search() {
        // Phase28 §2.2 — five-column shape with a per-row intent.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retrieval.csv");
        std::fs::write(
            &path,
            "id,query,repo,intent,expected_paths\n\
             r1,what laws govern git,cortex,law_check,docs/specs/13-laws-dsl.md\n\
             r2,what is ADR-013,cortex,,docs/specs/02.md\n",
        )
        .unwrap();
        let rows = load_csv(&path).unwrap();
        assert_eq!(rows[0].intent(), "law_check");
        assert_eq!(rows[1].intent(), "free_search", "blank intent defaults");
    }
}
