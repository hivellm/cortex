//! Relevance report — JSON shape persisted to disk + uploaded as the
//! CI gate's artifact. The shape is stable: the dashboard later reads
//! it from `.rulebook/learnings/relevance/<date>-<sha>.json`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::harness::ScoredQuery;

/// Per-bucket aggregate scores (per-intent or global).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IntentScores {
    /// Number of queries scored against this bucket.
    pub total: usize,
    /// Number of queries where any expected id appeared in top-10.
    pub matches: usize,
    /// `matches / total * 100.0` — `0.0` for empty buckets.
    pub recall_at_10_pct: f64,
    /// Mean `1/rank` of the first matching id. Missing matches
    /// contribute `0.0`. `0.0` for empty buckets.
    pub mrr_avg: f64,
}

impl IntentScores {
    /// Aggregate a slice of [`ScoredQuery`] outcomes into the bucket.
    pub fn from_scored(scored: &[ScoredQuery]) -> Self {
        let total = scored.len();
        if total == 0 {
            return Self::default();
        }
        let matches = scored.iter().filter(|s| s.recall_at_10).count();
        let recall_at_10_pct = (matches as f64 / total as f64) * 100.0;
        let mrr_sum: f64 = scored.iter().map(|s| s.mrr).sum();
        let mrr_avg = mrr_sum / total as f64;
        Self {
            total,
            matches,
            recall_at_10_pct,
            mrr_avg,
        }
    }
}

/// One per-query row carried in the report — kept in stable id order
/// so diffs across runs touch only the rows that actually changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    /// Stable fixture id (`rel-NNN`).
    pub id: String,
    /// Intent label.
    pub intent: String,
    /// The query text — kept inline so the report is self-contained.
    pub query: String,
    /// Did any expected id appear in the top-10?
    pub recall_at_10: bool,
    /// 1-based rank of the first match, or `None` when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rank: Option<usize>,
    /// `1.0 / matched_rank` or `0.0`.
    pub mrr: f64,
    /// First matched expected id (when any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_doc_id: Option<String>,
    /// Number of snippets returned for this query.
    pub returned: usize,
}

/// Top-level report shape persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevanceReport {
    /// ISO-8601 wall clock at run start.
    pub generated_at: String,
    /// Git commit the harness ran against (best-effort — `unknown`
    /// when no `git` is available at runtime).
    pub git_sha: String,
    /// `cortex-api` version surfaced via `/v1/status`.
    pub api_version: Option<String>,
    /// Intent buckets the run skipped because the underlying
    /// backend reported unhealthy at boot.
    pub omitted_intents: Vec<String>,
    /// Per-intent aggregate scores (sorted by intent label).
    pub per_intent: BTreeMap<String, IntentScores>,
    /// Global aggregate.
    pub global: IntentScores,
    /// Per-query rows (sorted by id).
    pub queries: Vec<QueryResult>,
}

impl RelevanceReport {
    /// Pretty-print to a `target/relevance/<git-sha>.json` file. The
    /// directory is created on demand.
    pub fn write_pretty(&self, dir: &Path, basename: &str) -> Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create report dir {}", dir.display()))?;
        let path = dir.join(format!("{basename}.json"));
        let body = serde_json::to_string_pretty(self).context("serialize report")?;
        std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    /// Load a previous report from disk for the regression gate.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read baseline {}", path.display()))?;
        let parsed: RelevanceReport =
            serde_json::from_str(&raw).context("parse baseline report")?;
        Ok(parsed)
    }
}

/// Outcome of comparing the current run against a baseline. The CLI
/// turns the verdict into the harness's exit code.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RegressionVerdict {
    /// Absolute pp delta on global recall (positive = better than baseline).
    pub recall_delta_pp: f64,
    /// Absolute delta on global MRR.
    pub mrr_delta: f64,
    /// Per-intent recall deltas.
    pub per_intent_recall_delta_pp: BTreeMap<String, f64>,
    /// Per-intent MRR deltas.
    pub per_intent_mrr_delta: BTreeMap<String, f64>,
    /// Hard-gate verdict — `true` when global metrics regressed
    /// beyond the configured threshold.
    pub hard_regression: bool,
    /// Soft-gate hits (per-intent regressions) for stdout warnings.
    pub soft_regressions: Vec<String>,
    /// Worst 5 regressed query ids (ranked by MRR drop).
    pub worst_queries: Vec<String>,
    /// Threshold used for the hard gate — echoed for CI logs.
    pub threshold_pp: f64,
}

impl RegressionVerdict {
    /// Compute deltas + verdict against a previous report.
    ///
    /// `threshold_pp` is the absolute pp band: a global drop of more
    /// than `threshold_pp` on either `recall_at_10_pct` or
    /// `mrr_avg * 100` flips `hard_regression`.
    pub fn evaluate(
        current: &RelevanceReport,
        baseline: &RelevanceReport,
        threshold_pp: f64,
    ) -> Self {
        let recall_delta_pp =
            current.global.recall_at_10_pct - baseline.global.recall_at_10_pct;
        let mrr_delta = current.global.mrr_avg - baseline.global.mrr_avg;

        let mut per_intent_recall_delta_pp = BTreeMap::new();
        let mut per_intent_mrr_delta = BTreeMap::new();
        let mut soft_regressions = Vec::new();
        for (intent, base_scores) in &baseline.per_intent {
            let cur = current
                .per_intent
                .get(intent)
                .cloned()
                .unwrap_or_default();
            let r = cur.recall_at_10_pct - base_scores.recall_at_10_pct;
            let m = cur.mrr_avg - base_scores.mrr_avg;
            per_intent_recall_delta_pp.insert(intent.clone(), r);
            per_intent_mrr_delta.insert(intent.clone(), m);
            if r < -threshold_pp || (m * 100.0) < -threshold_pp {
                soft_regressions.push(intent.clone());
            }
        }

        let hard_regression =
            recall_delta_pp < -threshold_pp || (mrr_delta * 100.0) < -threshold_pp;

        let worst_queries = compute_worst_regressed(current, baseline, 5);

        Self {
            recall_delta_pp,
            mrr_delta,
            per_intent_recall_delta_pp,
            per_intent_mrr_delta,
            hard_regression,
            soft_regressions,
            worst_queries,
            threshold_pp,
        }
    }
}

fn compute_worst_regressed(
    current: &RelevanceReport,
    baseline: &RelevanceReport,
    cap: usize,
) -> Vec<String> {
    let mut base_by_id: BTreeMap<&str, &QueryResult> = BTreeMap::new();
    for q in &baseline.queries {
        base_by_id.insert(q.id.as_str(), q);
    }
    let mut deltas: Vec<(String, f64)> = Vec::new();
    for q in &current.queries {
        if let Some(prev) = base_by_id.get(q.id.as_str()) {
            // Negative delta = regression.
            let mrr_drop = q.mrr - prev.mrr;
            // Boolean drop counts as a sentinel -1.0 (recall flipped off).
            let recall_drop = if prev.recall_at_10 && !q.recall_at_10 {
                -1.0
            } else {
                0.0
            };
            let combined = mrr_drop + recall_drop;
            if combined < 0.0 {
                deltas.push((q.id.clone(), combined));
            }
        }
    }
    // Smallest (most negative) first.
    deltas.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    deltas.into_iter().take(cap).map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relevance_eval::harness::ScoredQuery;

    fn scored(id: &str, intent: &str, recall: bool, rank: Option<usize>) -> ScoredQuery {
        ScoredQuery {
            id: id.into(),
            intent: intent.into(),
            query: format!("q-{id}"),
            recall_at_10: recall,
            matched_rank: rank,
            mrr: rank.map(|r| 1.0 / r as f64).unwrap_or(0.0),
            matched_doc_id: None,
            returned: 10,
        }
    }

    #[test]
    fn intent_scores_basics() {
        let s = vec![
            scored("a", "explain", true, Some(1)),
            scored("b", "explain", true, Some(4)),
            scored("c", "explain", false, None),
        ];
        let agg = IntentScores::from_scored(&s);
        assert_eq!(agg.total, 3);
        assert_eq!(agg.matches, 2);
        // recall = 2/3 * 100 = 66.666...
        assert!((agg.recall_at_10_pct - 66.6666).abs() < 0.01);
        // MRR = (1.0 + 0.25 + 0.0) / 3 = 0.4166...
        assert!((agg.mrr_avg - 0.41666).abs() < 0.01);
    }

    #[test]
    fn intent_scores_empty_bucket() {
        let agg = IntentScores::from_scored(&[]);
        assert_eq!(agg.total, 0);
        assert_eq!(agg.matches, 0);
        assert_eq!(agg.recall_at_10_pct, 0.0);
        assert_eq!(agg.mrr_avg, 0.0);
    }

    #[test]
    fn mrr_handles_first_position_match() {
        let s = vec![scored("a", "explain", true, Some(1))];
        let agg = IntentScores::from_scored(&s);
        // First-position match must score MRR=1.0, recall=100%.
        assert_eq!(agg.recall_at_10_pct, 100.0);
        assert!((agg.mrr_avg - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mrr_zero_when_no_match() {
        let s = vec![scored("a", "explain", false, None)];
        let agg = IntentScores::from_scored(&s);
        assert_eq!(agg.recall_at_10_pct, 0.0);
        assert_eq!(agg.mrr_avg, 0.0);
    }

    fn synthesize_report(global: IntentScores, queries: Vec<QueryResult>) -> RelevanceReport {
        RelevanceReport {
            generated_at: "2026-01-01T00:00:00Z".into(),
            git_sha: "deadbeef".into(),
            api_version: Some("0.1.0".into()),
            omitted_intents: Vec::new(),
            per_intent: BTreeMap::new(),
            global,
            queries,
        }
    }

    fn qr(id: &str, recall: bool, rank: Option<usize>) -> QueryResult {
        QueryResult {
            id: id.into(),
            intent: "explain".into(),
            query: format!("q-{id}"),
            recall_at_10: recall,
            matched_rank: rank,
            mrr: rank.map(|r| 1.0 / r as f64).unwrap_or(0.0),
            matched_doc_id: None,
            returned: 10,
        }
    }

    #[test]
    fn regression_within_threshold_is_clean() {
        let baseline = synthesize_report(
            IntentScores {
                total: 10,
                matches: 8,
                recall_at_10_pct: 80.0,
                mrr_avg: 0.7,
            },
            Vec::new(),
        );
        let current = synthesize_report(
            IntentScores {
                total: 10,
                matches: 8,
                recall_at_10_pct: 79.0,
                mrr_avg: 0.69,
            },
            Vec::new(),
        );
        let v = RegressionVerdict::evaluate(&current, &baseline, 2.0);
        assert!(!v.hard_regression);
        assert!((v.recall_delta_pp - -1.0).abs() < 1e-9);
    }

    #[test]
    fn regression_beyond_threshold_fires_hard_gate() {
        let baseline = synthesize_report(
            IntentScores {
                total: 10,
                matches: 8,
                recall_at_10_pct: 80.0,
                mrr_avg: 0.7,
            },
            Vec::new(),
        );
        let current = synthesize_report(
            IntentScores {
                total: 10,
                matches: 5,
                recall_at_10_pct: 50.0,
                mrr_avg: 0.4,
            },
            Vec::new(),
        );
        let v = RegressionVerdict::evaluate(&current, &baseline, 2.0);
        assert!(v.hard_regression);
        assert!(v.recall_delta_pp < -2.0);
    }

    #[test]
    fn worst_queries_ranks_recall_flips_first() {
        let baseline = synthesize_report(
            IntentScores::default(),
            vec![
                qr("a", true, Some(1)),
                qr("b", true, Some(2)),
                qr("c", false, None),
            ],
        );
        let current = synthesize_report(
            IntentScores::default(),
            vec![
                qr("a", false, None),    // recall flip — heaviest
                qr("b", true, Some(10)), // mrr drop only
                qr("c", false, None),    // unchanged
            ],
        );
        let v = RegressionVerdict::evaluate(&current, &baseline, 2.0);
        assert_eq!(v.worst_queries.first().map(|s| s.as_str()), Some("a"));
    }
}
