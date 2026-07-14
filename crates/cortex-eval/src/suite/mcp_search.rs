//! Phase19 §5.3 — `mcp_search` golden-set suite.
//!
//! Drives a 10-row fixture per granular tool exposed by phase19 and
//! asserts `recall@5 >= 0.5`. The harness CSV rows pin the `tool`
//! discriminator alongside the query so a future per-tool floor split
//! (different recall expectations for entity vs topic vs file-touched
//! lookups) is a one-line CSV change rather than a code change.
//!
//! Schema mirrors the retrieval suite plus a `tool` column:
//!
//! ```csv
//! id,tool,query,repo,expected_ids
//! m-001,cortex_consolidations_by_entity,"DEC-0042","cortex","01CONS01;01CONS02"
//! ```
//!
//! `expected_ids` is a `;`-delimited list of stable identifiers the
//! row expects to surface in the tool's top-K response (envelope
//! `event_id`, `consolidation_id`, or repo-rooted path depending on
//! the tool). The harness owner sends the actual MCP call and feeds
//! the observed top-K back through [`build_report`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::recall_at_k;
use crate::report::{MetricRow, SuiteReport};
use crate::suite::AcceptanceVerdict;

/// One row in `crates/cortex-eval/tests/golden/mcp_search.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct McpSearchRow {
    /// Stable row id — feeds [`SuiteReport::per_row`].
    pub id: String,
    /// MCP tool name the row targets (`cortex_consolidations_by_entity`,
    /// `cortex_topic_search`, `cortex_files_touched`, …).
    pub tool: String,
    /// Free-text query / entity value / topic prefix / file path that
    /// the row exercises. Driver code interprets per `tool`.
    pub query: String,
    /// Optional repo scope. Empty when the tool is cross-repo
    /// (or rejects a repo filter outright).
    #[serde(default)]
    pub repo: String,
    /// `;`-delimited list of identifiers the row expects to see in
    /// the tool's top-K response. Parsed via [`McpSearchRow::expected_ids`].
    pub expected_ids: String,
}

impl McpSearchRow {
    /// Parse the `;`-delimited expected-id column into a vec,
    /// trimming whitespace and dropping empty entries.
    pub fn expected_ids(&self) -> Vec<String> {
        self.expected_ids
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Acceptance floor — recall@5 averaged across the suite.
pub const RECALL_AT_5_FLOOR: f64 = 0.50;
/// Phase19 §5.3 k for recall.
pub const RECALL_K: usize = 5;

/// Walk a [`SuiteReport`] and return a verdict against the
/// mcp_search suite floor.
pub fn mcp_search_acceptance(report: &SuiteReport) -> AcceptanceVerdict {
    let mut failed = Vec::new();
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

/// Read every [`McpSearchRow`] out of the golden CSV at `path`.
pub fn load_csv(path: &std::path::Path) -> Result<Vec<McpSearchRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("open mcp_search golden csv {}", path.display()))?;
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        let r: McpSearchRow = row.context("parse mcp_search csv row")?;
        out.push(r);
    }
    Ok(out)
}

/// Build a [`SuiteReport`] from per-row observations. Each row
/// supplies the ranked id list its driver got back from the matching
/// MCP tool.
pub fn build_report(rows: &[McpSearchRow], observed: &[Vec<String>]) -> SuiteReport {
    assert_eq!(
        rows.len(),
        observed.len(),
        "row/observation length mismatch"
    );
    let mut sum_recall = 0.0;
    let mut per_row = std::collections::BTreeMap::new();
    let mut per_tool: std::collections::BTreeMap<String, (f64, u32)> =
        std::collections::BTreeMap::new();
    for (row, obs) in rows.iter().zip(observed.iter()) {
        let expected = row.expected_ids();
        let row_recall = recall_at_k(obs, &expected, RECALL_K);
        sum_recall += row_recall;
        let entry = per_tool.entry(row.tool.clone()).or_insert_with(|| (0.0, 0));
        entry.0 += row_recall;
        entry.1 += 1;
        per_row.insert(
            row.id.clone(),
            serde_json::json!({
                "tool": row.tool,
                "recall_at_5": row_recall,
                "expected": expected,
                "observed_top_k": obs.iter().take(RECALL_K).collect::<Vec<_>>(),
            }),
        );
    }
    let n = rows.len().max(1) as f64;
    let recall = sum_recall / n;
    let mut metrics = vec![MetricRow {
        name: "recall_at_5".into(),
        value: recall,
        floor: Some(RECALL_AT_5_FLOOR),
        passed: recall >= RECALL_AT_5_FLOOR,
    }];
    // Per-tool recall surfaces as informational metrics (no floor
    // today — the spec leaves per-tool floors for follow-up tuning).
    for (tool, (sum, count)) in &per_tool {
        let avg = sum / *count as f64;
        metrics.push(MetricRow {
            name: format!("recall_at_5__{tool}"),
            value: avg,
            floor: None,
            passed: true,
        });
    }
    SuiteReport {
        suite: "mcp_search".into(),
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

    fn row(id: &str, tool: &str, expected: &str) -> McpSearchRow {
        McpSearchRow {
            id: id.into(),
            tool: tool.into(),
            query: "q".into(),
            repo: "".into(),
            expected_ids: expected.into(),
        }
    }

    #[test]
    fn expected_ids_splits_and_trims() {
        let r = row("m1", "cortex_topic_search", "a; b ; ; c");
        assert_eq!(r.expected_ids(), vec!["a", "b", "c"]);
    }

    #[test]
    fn build_report_passes_floor_on_perfect_recall() {
        let rows = vec![row("m1", "cortex_topic_search", "a")];
        let observed = vec![vec!["a".to_string(), "b".to_string()]];
        let report = build_report(&rows, &observed);
        assert_eq!(report.metric("recall_at_5").unwrap().value, 1.0);
        let v = mcp_search_acceptance(&report);
        assert!(v.passed);
    }

    #[test]
    fn build_report_fails_floor_on_zero_recall() {
        let rows = vec![row("m1", "cortex_topic_search", "a")];
        let observed = vec![vec!["x".to_string()]];
        let report = build_report(&rows, &observed);
        let v = mcp_search_acceptance(&report);
        assert!(!v.passed);
        assert_eq!(v.failed_metrics, vec!["recall_at_5".to_string()]);
    }

    #[test]
    fn build_report_emits_per_tool_breakdown() {
        let rows = vec![
            row("m1", "cortex_topic_search", "a"),
            row("m2", "cortex_files_touched", "b"),
        ];
        let observed = vec![vec!["a".to_string()], vec!["b".to_string()]];
        let report = build_report(&rows, &observed);
        assert!(report.metric("recall_at_5__cortex_topic_search").is_some());
        assert!(report.metric("recall_at_5__cortex_files_touched").is_some());
    }

    #[test]
    fn load_csv_round_trips_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp_search.csv");
        std::fs::write(
            &path,
            "id,tool,query,repo,expected_ids\n\
             m-001,cortex_topic_search,tool:claude-code,cortex,01TC01;01TC02\n\
             m-002,cortex_files_touched,crates/cortex-api/src/lib.rs,cortex,crates/cortex-api/src/lib.rs\n",
        )
        .unwrap();
        let rows = load_csv(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool, "cortex_topic_search");
        assert_eq!(rows[1].expected_ids(), vec!["crates/cortex-api/src/lib.rs"]);
    }
}
