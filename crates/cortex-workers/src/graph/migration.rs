//! phase18 §2.9-§2.13 — Migration scaffolding for the bitemporal cutover.
//!
//! This module ships the data types + helpers the `cortex-ops
//! migrate-bitemporal` CLI consumes. The CLI itself lives in
//! `cortex-cli/src/bin/cortex-ops/migrate_bitemporal.rs` (phase18
//! §2.9) — kept thin so the heavy lifting + tests live here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::branch::Branch;

/// One row of the migration report (§2.13). One row per project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationReport {
    /// Project slug the report covers.
    pub project: String,
    /// Nodes the bitemporal stamper touched.
    pub nodes_updated: u64,
    /// Edges the SUPERSEDES backfill created.
    pub edges_created: u64,
    /// Branches the auto-create path materialised.
    pub branches_created: u64,
    /// Anomalies the migration flagged (operator-actionable).
    pub anomalies: Vec<MigrationAnomaly>,
}

/// One anomaly flagged during the migration. Carries enough
/// context for the operator to act without re-running the CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationAnomaly {
    /// Anomaly discriminator.
    pub kind: AnomalyKind,
    /// Entity id the anomaly applies to.
    pub entity_id: String,
    /// Free-form human-readable detail.
    pub detail: String,
}

/// Phase18 §2.13 — Anomaly catalogue. Adding a variant requires
/// updating the migration's `record_anomaly` site plus the
/// operator-facing rendering in the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    /// A `Decision.status == "superseded"` row with no
    /// `Decision.supersedes` target — the SUPERSEDES backfill
    /// cannot stamp `valid_to` and `superseded_at` without a
    /// successor.
    SupersededWithoutSuccessor,
    /// `created_at < recorded_at` — the migration clamps
    /// `valid_from = recorded_at` and records the original gap.
    CreatedBeforeRecorded,
    /// `context_repo` absent or empty — the bitemporal stamper
    /// landed `project_id = "unknown"`.
    UnknownProjectFallback,
    /// An ADR carries `supersedes` pointing at a row that has not
    /// landed in Nexus. The SUPERSEDES edge is queued for retry.
    SupersedesTargetMissing,
}

impl AnomalyKind {
    /// Stable lowercase label for telemetry / JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            AnomalyKind::SupersededWithoutSuccessor => "superseded_without_successor",
            AnomalyKind::CreatedBeforeRecorded => "created_before_recorded",
            AnomalyKind::UnknownProjectFallback => "unknown_project_fallback",
            AnomalyKind::SupersedesTargetMissing => "supersedes_target_missing",
        }
    }
}

/// Aggregated per-project report set the CLI emits at the end of
/// the run (`--json` flag flips between plain-text and structured
/// output).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationSummary {
    /// One report per project the migration touched.
    pub reports: Vec<MigrationReport>,
    /// When the run started (RFC3339 second-precision).
    pub started_at: String,
    /// When the run finished (RFC3339 second-precision).
    pub finished_at: String,
}

impl MigrationSummary {
    /// Total nodes touched across every project.
    pub fn total_nodes(&self) -> u64 {
        self.reports.iter().map(|r| r.nodes_updated).sum()
    }

    /// Total SUPERSEDES edges created across every project.
    pub fn total_edges(&self) -> u64 {
        self.reports.iter().map(|r| r.edges_created).sum()
    }

    /// Total anomalies across every project.
    pub fn total_anomalies(&self) -> usize {
        self.reports.iter().map(|r| r.anomalies.len()).sum()
    }
}

/// Phase18 §2.12 — synthesize the `Branch::main_for(project)` row
/// the migration needs to materialise before any per-project node
/// can carry `branch_id = "main"`. The `created_at` value uses the
/// migration's run start time per ADR-018 (second-precision RFC3339).
pub fn main_branch_for_project(project: &str, run_started_at: &str) -> Branch {
    Branch::main_for(project, run_started_at)
}

/// Phase18 §2.11 — pair a superseded ADR with its successor.
/// Returns the bitemporal stamps the migration applies to the
/// target row + the SUPERSEDES edge metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionPair {
    /// ADR being superseded.
    pub target_id: String,
    /// New ADR taking its place.
    pub successor_id: String,
    /// The successor's `valid_from` — copied to `target.valid_to`.
    pub valid_to: String,
    /// The successor's `recorded_at` — copied to
    /// `target.superseded_at`.
    pub superseded_at: String,
}

impl SupersessionPair {
    /// Build the property delta the migration writes onto the
    /// superseded ADR.
    pub fn target_props(&self) -> BTreeMap<String, serde_json::Value> {
        let mut props = BTreeMap::new();
        props.insert(
            super::bitemporal::keys::VALID_TO.to_string(),
            serde_json::Value::String(self.valid_to.clone()),
        );
        props.insert(
            super::bitemporal::keys::SUPERSEDED_AT.to_string(),
            serde_json::Value::String(self.superseded_at.clone()),
        );
        props.insert(
            super::bitemporal::keys::LIFECYCLE.to_string(),
            serde_json::Value::String("superseded".to_string()),
        );
        props
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::branch::BranchStatus;

    #[test]
    fn anomaly_kind_round_trips_through_as_str() {
        for k in [
            AnomalyKind::SupersededWithoutSuccessor,
            AnomalyKind::CreatedBeforeRecorded,
            AnomalyKind::UnknownProjectFallback,
            AnomalyKind::SupersedesTargetMissing,
        ] {
            let s = k.as_str();
            assert!(!s.is_empty());
            assert!(s.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn migration_summary_aggregates_per_project_rows() {
        let summary = MigrationSummary {
            reports: vec![
                MigrationReport {
                    project: "cortex".into(),
                    nodes_updated: 1000,
                    edges_created: 5,
                    branches_created: 1,
                    anomalies: vec![MigrationAnomaly {
                        kind: AnomalyKind::UnknownProjectFallback,
                        entity_id: "mem-x".into(),
                        detail: "context_repo absent".into(),
                    }],
                },
                MigrationReport {
                    project: "nexus".into(),
                    nodes_updated: 500,
                    edges_created: 2,
                    branches_created: 1,
                    anomalies: vec![],
                },
            ],
            started_at: "2026-05-29T07:00:00Z".into(),
            finished_at: "2026-05-29T07:05:00Z".into(),
        };
        assert_eq!(summary.total_nodes(), 1500);
        assert_eq!(summary.total_edges(), 7);
        assert_eq!(summary.total_anomalies(), 1);
    }

    #[test]
    fn main_branch_factory_uses_run_start_time() {
        let b = main_branch_for_project("cortex", "2026-05-29T07:00:00Z");
        assert_eq!(b.id, "cortex:main");
        assert_eq!(b.project_id, "cortex");
        assert_eq!(b.created_at, "2026-05-29T07:00:00Z");
        assert_eq!(b.created_by, "system");
        assert_eq!(b.status, BranchStatus::Active);
        assert!(b.parent_branch_id.is_none());
    }

    #[test]
    fn supersession_pair_target_props_carry_three_columns() {
        let pair = SupersessionPair {
            target_id: "DEC-014".into(),
            successor_id: "DEC-016".into(),
            valid_to: "2026-03-12T00:00:00Z".into(),
            superseded_at: "2026-03-13T00:00:00Z".into(),
        };
        let props = pair.target_props();
        assert_eq!(props.len(), 3);
        assert_eq!(
            props
                .get(super::super::bitemporal::keys::VALID_TO)
                .and_then(|v| v.as_str()),
            Some("2026-03-12T00:00:00Z")
        );
        assert_eq!(
            props
                .get(super::super::bitemporal::keys::SUPERSEDED_AT)
                .and_then(|v| v.as_str()),
            Some("2026-03-13T00:00:00Z")
        );
        assert_eq!(
            props
                .get(super::super::bitemporal::keys::LIFECYCLE)
                .and_then(|v| v.as_str()),
            Some("superseded")
        );
    }
}
