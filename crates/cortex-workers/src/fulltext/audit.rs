//! Phase12g §1 — Meili index audit.
//!
//! Walks the declared index set (`cortex_storage::fulltext::INDEXES`)
//! against a live `MeiliClient` and reports per-index document
//! counts plus a classification (`empty` / `populated` / `missing`).
//! The `cortex-rulebook-*` and `cortex-vectorizer-*` indexes were
//! shipping configured-but-empty in production — every query scoped
//! to those repos returned zero hits even though Synap had the
//! events. This module is the operator surface for catching that
//! drift before query time.
//!
//! ## Scope
//!
//! Settings drift is covered by the separate
//! `cortex-ops doctor-meili-indexes` doctor (phase12d §3); document-
//! count drift is covered here. The two surfaces compose: a healthy
//! index has both the declared settings and a non-zero doc count.

use std::collections::BTreeMap;

use crate::fulltext::meili_client::{IndexStat, MeiliClient, MeiliError};

/// Per-index audit verdict.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuditRow {
    /// Index uid (matches `cortex_storage::fulltext::INDEXES[*].name`).
    pub index: String,
    /// `empty` — declared but `numberOfDocuments == 0`.
    /// `populated` — declared and non-zero.
    /// `missing` — declared but absent from the live Meili `/stats`.
    /// `orphan` — present on the live Meili but not declared in
    /// `INDEXES` (operator-injected or stale post-rename).
    pub status: &'static str,
    /// Live document count, or `0` for `missing`.
    pub number_of_documents: u64,
}

/// Roll-up of a single audit pass.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuditReport {
    /// Per-index rows in declared order followed by orphans (if any).
    pub rows: Vec<AuditRow>,
    /// Indexes declared with `numberOfDocuments == 0`.
    pub empty_count: u64,
    /// Indexes declared but absent from Meili.
    pub missing_count: u64,
    /// Indexes present on Meili but not declared.
    pub orphan_count: u64,
    /// Indexes with a non-zero document count.
    pub populated_count: u64,
}

impl AuditReport {
    /// Convenience: any `empty` / `missing` / `orphan` row triggers
    /// the operator gate. Used by the CLI to map the audit to an
    /// exit code.
    pub fn has_drift(&self) -> bool {
        self.empty_count > 0 || self.missing_count > 0 || self.orphan_count > 0
    }
}

/// Run the audit pass against the supplied `MeiliClient`.
///
/// Iterates the declared `INDEXES` set and the live Meili `/stats`
/// listing once each, classifies every entry, and returns the
/// roll-up. The function is `async` because `MeiliClient::list_indexes`
/// is async; callers in sync contexts (the CLI) wrap it in a
/// current-thread tokio runtime.
pub async fn audit_indexes<C: MeiliClient>(client: &C) -> Result<AuditReport, MeiliError> {
    let live: Vec<IndexStat> = client.list_indexes().await?;
    let live_by_uid: BTreeMap<String, u64> = live
        .iter()
        .map(|s| (s.uid.clone(), s.number_of_documents))
        .collect();
    let declared: Vec<&'static str> = cortex_storage::fulltext::INDEXES
        .iter()
        .map(|d| d.name)
        .collect();
    let declared_set: std::collections::BTreeSet<&str> = declared.iter().copied().collect();

    let mut rows: Vec<AuditRow> = Vec::with_capacity(declared.len() + live.len());
    let mut empty_count = 0u64;
    let mut missing_count = 0u64;
    let mut populated_count = 0u64;

    for name in &declared {
        match live_by_uid.get(*name) {
            Some(0) => {
                rows.push(AuditRow {
                    index: name.to_string(),
                    status: "empty",
                    number_of_documents: 0,
                });
                empty_count += 1;
            }
            Some(&n) => {
                rows.push(AuditRow {
                    index: name.to_string(),
                    status: "populated",
                    number_of_documents: n,
                });
                populated_count += 1;
            }
            None => {
                rows.push(AuditRow {
                    index: name.to_string(),
                    status: "missing",
                    number_of_documents: 0,
                });
                missing_count += 1;
            }
        }
    }

    let mut orphan_count = 0u64;
    for stat in &live {
        if !declared_set.contains(stat.uid.as_str()) {
            rows.push(AuditRow {
                index: stat.uid.clone(),
                status: "orphan",
                number_of_documents: stat.number_of_documents,
            });
            orphan_count += 1;
        }
    }

    Ok(AuditReport {
        rows,
        empty_count,
        missing_count,
        orphan_count,
        populated_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulltext::meili_client::{
        IndexStat, MemoryMeiliClient, TaskStatus, TaskUid, UpsertReport,
    };

    /// Test double for `MeiliClient` — backed by a fixed list of
    /// `IndexStat` so each test pins the live shape it wants. The
    /// shipped `MemoryMeiliClient` simulates upserts but does not
    /// expose a configurable `list_indexes` return; this minimal
    /// implementation fills that gap.
    struct FixedMeiliClient {
        live: Vec<IndexStat>,
    }

    #[async_trait::async_trait]
    impl MeiliClient for FixedMeiliClient {
        async fn ensure_index(
            &self,
            _index: &str,
            _settings: &serde_json::Value,
        ) -> Result<bool, MeiliError> {
            Ok(false)
        }
        async fn upsert_documents(
            &self,
            _index: &str,
            _docs: &[crate::fulltext::Document],
        ) -> Result<UpsertReport, MeiliError> {
            Ok(UpsertReport {
                documents_upserted: 0,
                documents_deduped: 0,
                task_uid: 0 as TaskUid,
            })
        }
        async fn wait_task(
            &self,
            _task: TaskUid,
            _timeout: std::time::Duration,
        ) -> Result<TaskStatus, MeiliError> {
            Ok(TaskStatus::Succeeded)
        }
        async fn list_indexes(&self) -> Result<Vec<IndexStat>, MeiliError> {
            Ok(self.live.clone())
        }
        async fn delete_index(&self, _index: &str) -> Result<(), MeiliError> {
            Ok(())
        }
    }

    fn fixed(live: Vec<(&str, u64)>) -> FixedMeiliClient {
        FixedMeiliClient {
            live: live
                .into_iter()
                .map(|(uid, n)| IndexStat {
                    uid: uid.to_string(),
                    number_of_documents: n,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn empty_meili_reports_every_declared_index_as_missing() {
        let client = fixed(vec![]);
        let report = audit_indexes(&client).await.unwrap();
        let declared_count = cortex_storage::fulltext::INDEXES.len() as u64;
        assert_eq!(report.missing_count, declared_count);
        assert_eq!(report.populated_count, 0);
        assert_eq!(report.empty_count, 0);
        assert_eq!(report.orphan_count, 0);
        assert!(report.has_drift());
    }

    #[tokio::test]
    async fn fully_populated_meili_reports_no_drift() {
        let live: Vec<(&str, u64)> = cortex_storage::fulltext::INDEXES
            .iter()
            .map(|d| (d.name, 100u64))
            .collect();
        let client = fixed(live);
        let report = audit_indexes(&client).await.unwrap();
        assert_eq!(
            report.populated_count,
            cortex_storage::fulltext::INDEXES.len() as u64
        );
        assert_eq!(report.empty_count, 0);
        assert_eq!(report.missing_count, 0);
        assert_eq!(report.orphan_count, 0);
        assert!(!report.has_drift());
    }

    #[tokio::test]
    async fn empty_index_classified_as_empty_not_populated() {
        let live: Vec<(&str, u64)> = cortex_storage::fulltext::INDEXES
            .iter()
            .map(|d| {
                if d.name == "cortex_turns" {
                    (d.name, 0u64)
                } else {
                    (d.name, 100u64)
                }
            })
            .collect();
        let client = fixed(live);
        let report = audit_indexes(&client).await.unwrap();
        assert_eq!(report.empty_count, 1);
        assert_eq!(
            report.populated_count,
            (cortex_storage::fulltext::INDEXES.len() - 1) as u64
        );
        let turns_row = report
            .rows
            .iter()
            .find(|r| r.index == "cortex_turns")
            .unwrap();
        assert_eq!(turns_row.status, "empty");
        assert_eq!(turns_row.number_of_documents, 0);
        assert!(report.has_drift());
    }

    #[tokio::test]
    async fn orphan_index_on_meili_surfaces_in_report() {
        let mut live: Vec<(&str, u64)> = cortex_storage::fulltext::INDEXES
            .iter()
            .map(|d| (d.name, 50u64))
            .collect();
        live.push(("cortex_legacy_shadow", 10));
        let client = fixed(live);
        let report = audit_indexes(&client).await.unwrap();
        assert_eq!(report.orphan_count, 1);
        let orphan = report
            .rows
            .iter()
            .find(|r| r.index == "cortex_legacy_shadow")
            .unwrap();
        assert_eq!(orphan.status, "orphan");
        assert_eq!(orphan.number_of_documents, 10);
        assert!(report.has_drift());
    }

    #[tokio::test]
    async fn baseline_when_every_declared_index_is_empty() {
        // Phase12g target shape: `cortex-rulebook-*` and
        // `cortex-vectorizer-*` ship empty in production. This pins
        // the contract that the audit reports every declared index
        // as `empty` when Meili holds every index at zero docs.
        let live: Vec<(&str, u64)> = cortex_storage::fulltext::INDEXES
            .iter()
            .map(|d| (d.name, 0u64))
            .collect();
        let client = fixed(live);
        let report = audit_indexes(&client).await.unwrap();
        assert_eq!(
            report.empty_count,
            cortex_storage::fulltext::INDEXES.len() as u64
        );
        assert!(report.has_drift());
    }

    #[tokio::test]
    async fn audit_runs_against_memory_meili_client_default() {
        let client = MemoryMeiliClient::new();
        let report = audit_indexes(&client).await.unwrap();
        // MemoryMeiliClient::list_indexes returns an empty list by
        // default — every declared index surfaces as `missing`.
        assert_eq!(
            report.missing_count,
            cortex_storage::fulltext::INDEXES.len() as u64
        );
    }
}
