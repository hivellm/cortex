//! Boot-time stale-index sweep for the fulltext worker (phase4a §3).
//!
//! After the per-project naming migration to
//! `cortex-{repo_slug}-{family}`, the live Meili cluster still
//! carries a handful of two-token names from the older scheme
//! (`cortex-code`, `cortex-decisions`, …). They are all empty, but
//! they pollute the `/indexes` listing and risk being targeted by
//! stale client code that still hard-codes the un-slugged names.
//!
//! The sweep walks every Meili index, classifies each name with
//! [`is_canonical_index_name`], and:
//!
//! - drops empty non-canonical names — the audit-confirmed stale
//!   set falls into this bucket;
//! - leaves non-empty non-canonical names alone and emits exactly
//!   one warn log line so the operator can investigate.
//!
//! Idempotent: re-running after a successful sweep is a no-op
//! because the deleted indexes are gone (or `delete_index` accepts
//! `404 Not Found` as success).
//!
//! [`is_canonical_index_name`]: crate::routing::is_canonical_index_name

use crate::meili_client::{MeiliClient, MeiliError};
use crate::routing::is_canonical_index_name;

/// Per-run summary the boot path logs and tests assert against.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Total indexes the sweep examined.
    pub examined: u32,
    /// Indexes that matched the canonical naming convention and
    /// were left alone.
    pub kept_canonical: u32,
    /// Empty non-canonical indexes that were dropped.
    pub deleted_stale_empty: u32,
    /// Non-empty non-canonical indexes that were preserved with a
    /// warning. The names land in [`SweepReport::warned_names`] so
    /// the operator can grep them out of structured logs.
    pub kept_warning: u32,
    /// Names that triggered a warning (length matches `kept_warning`).
    pub warned_names: Vec<String>,
}

/// Walk the Meili cluster and apply the stale-sweep policy.
///
/// Errors short-circuit the whole sweep — when Meili is
/// unreachable, the boot path falls back to "no sweep" and the
/// worker keeps booting; the caller decides how to surface the
/// failure (panic vs. warn-and-continue).
pub async fn sweep_stale_indexes<C>(client: &C) -> Result<SweepReport, MeiliError>
where
    C: MeiliClient + ?Sized,
{
    let indexes = client.list_indexes().await?;
    let mut report = SweepReport {
        examined: u32::try_from(indexes.len()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    for index in indexes {
        if is_canonical_index_name(&index.uid) {
            report.kept_canonical += 1;
            continue;
        }
        if index.number_of_documents > 0 {
            tracing::warn!(
                index = %index.uid,
                docs = index.number_of_documents,
                "fulltext sweep: non-canonical index has documents — preserving (operator review needed)"
            );
            report.kept_warning += 1;
            report.warned_names.push(index.uid);
            continue;
        }
        match client.delete_index(&index.uid).await {
            Ok(()) => {
                tracing::info!(
                    index = %index.uid,
                    reason = "stale-naming",
                    "fulltext sweep: dropped empty non-canonical index"
                );
                report.deleted_stale_empty += 1;
            }
            Err(err) => {
                tracing::warn!(
                    index = %index.uid,
                    error = %err,
                    "fulltext sweep: delete failed; preserving for next run"
                );
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meili_client::{MemoryCall, MemoryMeiliClient};

    #[tokio::test]
    async fn sweep_drops_empty_non_canonical_and_preserves_canonical() {
        let client = MemoryMeiliClient::new();
        // Live-cluster shape from the phase4a diagnostic:
        // 7 stale empties + 4 populated 3-token names + 1 populated
        // legacy name (must be preserved with warning).
        for stale in [
            "cortex-code",
            "cortex-decisions",
            "cortex-docs",
            "cortex-governance",
            "cortex-misc",
            "cortex-turns",
            "cortex-analyses",
        ] {
            client.seed_index(stale, 0);
        }
        for canonical in [
            ("cortex-cortex-code", 1862u64),
            ("cortex-rulebook-docs", 1456),
            ("cortex-nexus-turns", 2016),
            ("cortex-tml-code", 184_754),
        ] {
            client.seed_index(canonical.0, canonical.1);
        }
        client.seed_index("legacy-foo", 42);

        let report = sweep_stale_indexes(&client).await.unwrap();
        assert_eq!(report.examined, 12);
        assert_eq!(report.kept_canonical, 4);
        assert_eq!(report.deleted_stale_empty, 7);
        assert_eq!(report.kept_warning, 1);
        assert_eq!(report.warned_names, vec!["legacy-foo".to_string()]);

        // Each canonical name still exists; each stale empty is gone;
        // the populated legacy name was preserved.
        for stale in [
            "cortex-code",
            "cortex-decisions",
            "cortex-docs",
            "cortex-governance",
            "cortex-misc",
            "cortex-turns",
            "cortex-analyses",
        ] {
            assert!(
                !client.index_exists(stale),
                "{stale} should have been deleted",
            );
        }
        for canonical in [
            "cortex-cortex-code",
            "cortex-rulebook-docs",
            "cortex-nexus-turns",
            "cortex-tml-code",
        ] {
            assert!(
                client.index_exists(canonical),
                "{canonical} should be preserved",
            );
        }
        assert!(
            client.index_exists("legacy-foo"),
            "non-empty legacy index must be preserved",
        );

        // Calls audit: exactly seven DeleteIndex calls.
        let calls = client.calls_snapshot();
        let deletes: Vec<&str> = calls
            .iter()
            .filter_map(|c| match c {
                MemoryCall::DeleteIndex { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deletes.len(), 7);
    }

    #[tokio::test]
    async fn sweep_is_idempotent_under_replay() {
        let client = MemoryMeiliClient::new();
        client.seed_index("cortex-code", 0);
        client.seed_index("cortex-cortex-code", 100);

        let first = sweep_stale_indexes(&client).await.unwrap();
        assert_eq!(first.deleted_stale_empty, 1);

        let second = sweep_stale_indexes(&client).await.unwrap();
        // Re-run sees only the canonical index — nothing to delete.
        assert_eq!(second.examined, 1);
        assert_eq!(second.kept_canonical, 1);
        assert_eq!(second.deleted_stale_empty, 0);
        assert_eq!(second.kept_warning, 0);
    }

    #[tokio::test]
    async fn sweep_with_no_indexes_reports_zero_counters() {
        let client = MemoryMeiliClient::new();
        let report = sweep_stale_indexes(&client).await.unwrap();
        assert_eq!(report, SweepReport::default());
    }
}
