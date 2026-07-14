//! Phase14c consolidation suite — entity-recall + fact-recall
//! against `crates/cortex-eval/tests/golden/consolidation.csv`.
//!
//! Each row pairs a session_id with the entities and facts the
//! ideal Session-grain consolidation MUST cover. The harness:
//!
//! 1. Reads the published `ConsolidationPayload` for the session
//!    via `cortex-api /v1/dashboard/consolidations`.
//! 2. Extracts entities (proper-noun phrases) + facts (load-bearing
//!    statements) from `summary_markdown + takeaways`.
//! 3. Computes set-recall against `expected_entities` +
//!    `expected_facts`.
//!
//! Extraction is intentionally cheap (regex / heuristic) — the
//! goldens are curated to use stable phrases the regex will catch.
//!
//! Golden CSV shape:
//!
//! ```csv
//! id,session_id,expected_entities,expected_facts
//! c-001,01HXSESS00,"ef_search;HNSW;recall@10","ef_search=128;recall>=0.92"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::set_recall;
use crate::report::{MetricRow, SuiteReport};
use crate::suite::AcceptanceVerdict;

/// One row in `crates/cortex-eval/tests/golden/consolidation.csv`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ConsolidationRow {
    /// Stable row id.
    pub id: String,
    /// The consolidation route id (envelope ULID) — fetched via
    /// `/v1/dashboard/consolidations/{id}`. phase0 2026-06-22: re-keyed
    /// from `session_id` (the dashboard exposes no session_id; it keys
    /// consolidations by this route id / `consolidation_id`).
    pub consolidation_id: String,
    /// `;`-delimited entities the ideal consolidation MUST mention.
    pub expected_entities: String,
    /// `;`-delimited facts (claim-shaped phrases) the ideal
    /// consolidation MUST carry.
    pub expected_facts: String,
}

impl ConsolidationRow {
    /// Split helper for `expected_entities`.
    pub fn expected_entity_list(&self) -> Vec<String> {
        split_semi(&self.expected_entities)
    }
    /// Split helper for `expected_facts`.
    pub fn expected_fact_list(&self) -> Vec<String> {
        split_semi(&self.expected_facts)
    }
}

fn split_semi(s: &str) -> Vec<String> {
    s.split(';')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Acceptance floor for entity-recall per the phase14c proposal.
pub const ENTITY_RECALL_FLOOR: f64 = 0.85;
/// Acceptance floor for fact-recall. Looser than entity-recall
/// because fact phrases vary lexically more — keeping the floor
/// at 0.70 acknowledges the heuristic extractor's brittleness.
pub const FACT_RECALL_FLOOR: f64 = 0.70;

/// Phase14c §3.4 acceptance gate for the consolidation suite.
pub fn consolidation_acceptance(report: &SuiteReport) -> AcceptanceVerdict {
    let mut failed = Vec::new();
    if report
        .metric("entity_recall")
        .map(|m| m.value < ENTITY_RECALL_FLOOR)
        .unwrap_or(true)
    {
        failed.push("entity_recall".into());
    }
    if report
        .metric("fact_recall")
        .map(|m| m.value < FACT_RECALL_FLOOR)
        .unwrap_or(true)
    {
        failed.push("fact_recall".into());
    }
    AcceptanceVerdict {
        passed: failed.is_empty(),
        failed_metrics: failed,
    }
}

/// Read every [`ConsolidationRow`] out of the golden CSV at `path`.
pub fn load_csv(path: &std::path::Path) -> Result<Vec<ConsolidationRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("open consolidation golden csv {}", path.display()))?;
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        let r: ConsolidationRow = row.context("parse consolidation csv row")?;
        out.push(r);
    }
    Ok(out)
}

/// Build a [`SuiteReport`] from per-row observations. `observed`
/// supplies the `summary_markdown + takeaways` corpus the suite
/// scans for entities + facts.
pub fn build_report(rows: &[ConsolidationRow], observed: &[String]) -> SuiteReport {
    assert_eq!(
        rows.len(),
        observed.len(),
        "row/observation length mismatch"
    );
    let mut sum_entity = 0.0;
    let mut sum_fact = 0.0;
    let mut per_row = std::collections::BTreeMap::new();
    for (row, body) in rows.iter().zip(observed.iter()) {
        let body_lc = body.to_lowercase();
        let entity_hits: Vec<String> = row
            .expected_entity_list()
            .into_iter()
            .filter(|e| body_lc.contains(&e.to_lowercase()))
            .collect();
        let fact_hits: Vec<String> = row
            .expected_fact_list()
            .into_iter()
            .filter(|f| body_lc.contains(&f.to_lowercase()))
            .collect();
        let entities_expected = row.expected_entity_list();
        let facts_expected = row.expected_fact_list();
        let entity_recall = set_recall(&entity_hits, &entities_expected);
        let fact_recall = set_recall(&fact_hits, &facts_expected);
        sum_entity += entity_recall;
        sum_fact += fact_recall;
        per_row.insert(
            row.id.clone(),
            serde_json::json!({
                "entity_recall": entity_recall,
                "fact_recall": fact_recall,
                "entities_expected": entities_expected,
                "entities_hit": entity_hits,
                "facts_expected": facts_expected,
                "facts_hit": fact_hits,
            }),
        );
    }
    let n = rows.len().max(1) as f64;
    let entity_recall = sum_entity / n;
    let fact_recall = sum_fact / n;
    SuiteReport {
        suite: "consolidation".into(),
        finished_at: chrono::Utc::now().to_rfc3339(),
        rows_total: rows.len() as u32,
        rows_errored: 0,
        metrics: vec![
            MetricRow {
                name: "entity_recall".into(),
                value: entity_recall,
                floor: Some(ENTITY_RECALL_FLOOR),
                passed: entity_recall >= ENTITY_RECALL_FLOOR,
            },
            MetricRow {
                name: "fact_recall".into(),
                value: fact_recall,
                floor: Some(FACT_RECALL_FLOOR),
                passed: fact_recall >= FACT_RECALL_FLOOR,
            },
        ],
        per_row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, entities: &str, facts: &str) -> ConsolidationRow {
        ConsolidationRow {
            id: id.into(),
            consolidation_id: format!("01SESS{id}"),
            expected_entities: entities.into(),
            expected_facts: facts.into(),
        }
    }

    #[test]
    fn split_semi_drops_empties_and_trims() {
        assert_eq!(split_semi("a; b ;; c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn build_report_full_recall_passes_acceptance() {
        let rows = vec![row("c1", "HNSW;ef_search", "ef_search=128")];
        let observed = vec!["Tuned HNSW so ef_search=128 holds recall.".to_string()];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("entity_recall").unwrap().value, 1.0);
        assert_eq!(r.metric("fact_recall").unwrap().value, 1.0);
        let v = consolidation_acceptance(&r);
        assert!(v.passed);
    }

    #[test]
    fn build_report_zero_recall_fails_acceptance() {
        let rows = vec![row("c1", "HNSW;ef_search", "ef_search=128")];
        let observed = vec!["Unrelated narrative.".to_string()];
        let r = build_report(&rows, &observed);
        assert_eq!(r.metric("entity_recall").unwrap().value, 0.0);
        let v = consolidation_acceptance(&r);
        assert!(!v.passed);
        assert_eq!(v.failed_metrics.len(), 2);
    }
}
