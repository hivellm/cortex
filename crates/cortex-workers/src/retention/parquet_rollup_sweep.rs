//! Phase13a §3.2 — `ParquetRollupSweep` wraps the existing archive
//! rollup orchestration ([`enumerate_compactable`] +
//! [`compact_partition`] / [`apply_three_year_drop`] +
//! [`quarantine_pre_existing`]) behind the [`Sweep`] trait.
//!
//! The wrapper mirrors the loop the `cortex-ops rollup` binary ran
//! before phase13a: pre-flight quarantine, then per-granularity
//! enumerate-then-compact pass. The legacy [`RollupCounts`] map onto
//! the trait-level [`SweepReport`] as follows:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `files_in` | `tier_transitions["files_in"]` |
//! | `files_out` | `tier_transitions["files_out"]` |
//! | `bytes_reclaimed` | `bytes_reclaimed` |
//! | `quarantined` | `tier_transitions["quarantined"]` |
//! | `records_dropped` | `tier_transitions["records_dropped"]` (also folded into `rows_processed`) |
//! | `records_preserved` | `tier_transitions["records_preserved"]` (also folded into `rows_processed`) |
//!
//! `rows_processed = records_preserved + records_dropped` so the
//! dashboard's invocation-level bar shows the work the rollup
//! actually did, not the no-op file moves.

use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::parquet_rollup::{
    apply_three_year_drop, compact_partition, enumerate_compactable, quarantine_pre_existing,
    Granularity, RollupCounts,
};

/// Canonical sweep name.
pub const PARQUET_ROLLUP_NAME: &str = "parquet_rollup";

/// Default schedule — daily 04:00 UTC, matches `default_jobs()` row
/// `retention.rollup` (`"0 4 * * *"` 5-field ↦ `"0 0 4 * * * *"` 7-field).
pub const PARQUET_ROLLUP_SCHEDULE: &str = "0 0 4 * * * *";

/// Archive-rollup sweep wrapped behind the [`Sweep`] trait.
pub struct ParquetRollupSweep {
    archive_root: PathBuf,
    granularities: Vec<Granularity>,
}

impl ParquetRollupSweep {
    /// Build the sweep with the default granularity set
    /// (`HourlyToDaily`, `DailyToMonthly`, `ThreeYearDrop`).
    pub fn new(archive_root: PathBuf) -> Self {
        Self {
            archive_root,
            granularities: vec![
                Granularity::HourlyToDaily,
                Granularity::DailyToMonthly,
                Granularity::ThreeYearDrop,
            ],
        }
    }

    /// Builder shim — override the granularity set (used by
    /// integration tests + the operator's `--granularity` flag).
    #[must_use]
    pub fn with_granularities(mut self, granularities: Vec<Granularity>) -> Self {
        self.granularities = granularities;
        self
    }
}

#[async_trait]
impl Sweep for ParquetRollupSweep {
    fn name(&self) -> &'static str {
        PARQUET_ROLLUP_NAME
    }

    fn schedule(&self) -> Schedule {
        Schedule::from_str(PARQUET_ROLLUP_SCHEDULE)
            .expect("PARQUET_ROLLUP_SCHEDULE is a constant valid 7-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let dry_run = ctx.config.dry_run;
        let archive_root = self.archive_root.clone();
        let granularities = self.granularities.clone();
        let started_at = ctx.now;
        let now = ctx.now;

        // The rollup is synchronous filesystem work. Offload onto the
        // tokio blocking pool so the scheduler tick is not held for
        // the duration of a multi-gig compaction.
        let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<RollupCounts> {
            let mut totals = RollupCounts::default();
            if !dry_run {
                let pre = quarantine_pre_existing(&archive_root);
                totals.merge(&pre);
            }
            let mut had_error: Option<String> = None;
            for g in &granularities {
                let plans = enumerate_compactable(&archive_root, now, *g);
                let mut sub = RollupCounts::default();
                if !dry_run {
                    for plan in &plans {
                        let result = match g {
                            Granularity::ThreeYearDrop => {
                                apply_three_year_drop(&archive_root, plan)
                            }
                            _ => compact_partition(&archive_root, plan),
                        };
                        match result {
                            Ok(c) => sub.merge(&c),
                            Err(e) => {
                                if had_error.is_none() {
                                    had_error = Some(e.to_string());
                                }
                                tracing::warn!(
                                    error = %e,
                                    granularity = %g.as_str(),
                                    "parquet_rollup: partition compaction failed"
                                );
                            }
                        }
                    }
                }
                totals.merge(&sub);
            }
            if let Some(msg) = had_error {
                Err(anyhow::anyhow!(msg))
            } else {
                Ok(totals)
            }
        })
        .await?;

        let report = SweepReport::started(PARQUET_ROLLUP_NAME, started_at);
        match outcome {
            Ok(totals) => {
                let rows_processed = totals.records_preserved + totals.records_dropped;
                let bytes_reclaimed = totals.bytes_reclaimed;
                let next = report
                    .with_tier_transition("files_in", totals.files_in)
                    .with_tier_transition("files_out", totals.files_out)
                    .with_tier_transition("quarantined", totals.quarantined)
                    .with_tier_transition("records_dropped", totals.records_dropped)
                    .with_tier_transition("records_preserved", totals.records_preserved);
                Ok(next.finish_success(ctx.now, rows_processed, bytes_reclaimed))
            }
            Err(err) => Ok(report.finish_failed(ctx.now, 0, 0, err.to_string())),
        }
    }

    fn report_view(&self, report: &SweepReport) -> SweepReportView {
        report.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweep::SweepStatus;
    use chrono::{DateTime, Utc};
    use cortex_storage::MetadataStore;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T04:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.rollup").with_now(now)
    }

    #[tokio::test]
    async fn rollup_on_empty_archive_yields_success_zero_rows() {
        let dir = TempDir::new().unwrap();
        let sweep = ParquetRollupSweep::new(dir.path().to_path_buf());
        let ctx = make_ctx(fixed_now());
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, PARQUET_ROLLUP_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
        assert_eq!(report.bytes_reclaimed, 0);
    }

    #[tokio::test]
    async fn rollup_dry_run_via_ctx_skips_quarantine_pass() {
        let dir = TempDir::new().unwrap();
        let sweep = ParquetRollupSweep::new(dir.path().to_path_buf());
        let mut ctx = make_ctx(fixed_now());
        ctx.config.dry_run = true;
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(*report.tier_transitions.get("quarantined").unwrap_or(&0), 0);
    }

    #[test]
    fn rollup_sweep_schedule_parses_and_name_is_canonical() {
        let dir = TempDir::new().unwrap();
        let sweep = ParquetRollupSweep::new(dir.path().to_path_buf());
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "parquet_rollup");
    }
}
