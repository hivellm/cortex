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
//! [`is_canonical_index_name`]: super::routing::is_canonical_index_name

use super::meili_client::{MeiliClient, MeiliError};
use super::routing::is_canonical_index_name;

/// Phase11p §1.1 — one row in the canonical-empty audit list. The
/// caller decides whether to drop or preserve; the lib never deletes
/// canonical indexes itself because empty-canonical can be a legitimate
/// transient state (per-repo lazy materialisation right after settings
/// PATCH, before the first upsert lands). The `cortex-ops sweep-empty`
/// CLI gates the destructive call on operator `--apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyCanonical {
    /// Canonical Meili index uid (`cortex-{slug}-{family}`).
    pub uid: String,
}

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

/// Phase11p §1.1 — list canonical Meili indexes that hold zero
/// documents. Returns the candidates so the caller (the
/// `cortex-ops sweep-empty` CLI) can dry-run + operator-confirm
/// before the destructive `delete_index` call.
///
/// Why a separate sibling to [`sweep_stale_indexes`]: the existing
/// sweep targets non-canonical names (the phase4a stale set) and
/// auto-deletes them on boot because their presence is by definition
/// a migration artefact. Canonical-but-empty is different — a fresh
/// per-repo index after a settings PATCH but before the first upsert
/// is also empty-canonical, and dropping it would force the next
/// upsert to recreate it. The audit-time use is "operator wants to
/// reclaim long-abandoned repo names"; the lib returns the list, the
/// CLI decides.
pub async fn sweep_empty_canonical<C>(client: &C) -> Result<Vec<EmptyCanonical>, MeiliError>
where
    C: MeiliClient + ?Sized,
{
    let indexes = client.list_indexes().await?;
    let mut out: Vec<EmptyCanonical> = Vec::new();
    for index in indexes {
        if !is_canonical_index_name(&index.uid) {
            continue;
        }
        if index.number_of_documents > 0 {
            continue;
        }
        out.push(EmptyCanonical { uid: index.uid });
    }
    out.sort_by(|a, b| a.uid.cmp(&b.uid));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulltext::meili_client::{MemoryCall, MemoryMeiliClient};

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

    #[tokio::test]
    async fn sweep_empty_canonical_buckets_inputs_correctly() {
        // Phase11p §1.3 — exercise every (canonical?, empty?) bucket:
        // canonical-populated stays out of the result; canonical-empty
        // lands in the result; non-canonical (populated or empty)
        // never lands here (handled by sweep_stale_indexes).
        let client = MemoryMeiliClient::new();
        // Canonical, populated — must NOT appear in result.
        client.seed_index("cortex-cortex-code", 4_748);
        client.seed_index("cortex-tml-code", 189_872);
        // Canonical, empty — must appear in result.
        client.seed_index("cortex-csharp-code", 0);
        client.seed_index("cortex-rust-governance", 0);
        client.seed_index("cortex-tests-decisions", 0);
        // Non-canonical, populated — never appears here.
        client.seed_index("legacy-foo", 42);
        // Non-canonical, empty — handled by sweep_stale_indexes; never
        // appears in this list.
        client.seed_index("cortex-code", 0);

        let candidates = sweep_empty_canonical(&client).await.unwrap();
        // Result is sorted alphabetically; pin the exact list.
        assert_eq!(
            candidates,
            vec![
                EmptyCanonical {
                    uid: "cortex-csharp-code".to_string(),
                },
                EmptyCanonical {
                    uid: "cortex-rust-governance".to_string(),
                },
                EmptyCanonical {
                    uid: "cortex-tests-decisions".to_string(),
                },
            ],
        );
        // Crucially: the function MUST NOT call delete_index on its
        // own — operator-only via the cortex-ops CLI.
        let calls = client.calls_snapshot();
        let deletes: Vec<&str> = calls
            .iter()
            .filter_map(|c| match c {
                MemoryCall::DeleteIndex { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            deletes.is_empty(),
            "sweep_empty_canonical MUST be read-only; saw deletes: {deletes:?}",
        );
    }

    #[tokio::test]
    async fn sweep_empty_canonical_returns_empty_when_every_index_is_populated() {
        let client = MemoryMeiliClient::new();
        client.seed_index("cortex-cortex-code", 100);
        client.seed_index("cortex-cortex-decisions", 17);
        let candidates = sweep_empty_canonical(&client).await.unwrap();
        assert!(candidates.is_empty());
    }

    #[tokio::test]
    async fn sweep_empty_canonical_skips_non_canonical_empty_names() {
        // sweep_stale_indexes owns the non-canonical bucket; this
        // function MUST NOT poach those names.
        let client = MemoryMeiliClient::new();
        client.seed_index("cortex-code", 0); // non-canonical, empty
        client.seed_index("legacy-foo", 0); // non-canonical, empty
        let candidates = sweep_empty_canonical(&client).await.unwrap();
        assert!(candidates.is_empty());
    }
}
