//! Phase13a §3.3 — `CasVacuumSweep` wraps the existing CAS vacuum
//! orchestration ([`cas_vacuum::run`](super::cas_vacuum::run))
//! behind the [`Sweep`] trait.
//!
//! The wrapper carries a `cas_path: PathBuf` + a `VacuumOpts`
//! template. Each invocation opens its own `CasStore` inside a
//! blocking task so the !Sync rusqlite connection never leaves the
//! tokio blocking pool. The legacy `VacuumReport` maps onto the
//! trait-level [`SweepReport`] as follows:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `blobs_dropped` | `rows_processed` (+ `tier_transitions["blobs_dropped"]`) |
//! | `bytes_reclaimed` | `bytes_reclaimed` |
//! | `total_blobs` | `tier_transitions["total_blobs"]` |
//! | `did_vacuum` | `tier_transitions["did_vacuum"]` (0/1) |
//! | `safeguard_tripped` | `tier_transitions["safeguard_tripped"]` (0/1) |
//! | `vacuum_ms` | `tier_transitions["vacuum_ms"]` |

use std::path::PathBuf;

use async_trait::async_trait;
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::cas_vacuum::{open_store, run as run_vacuum, VacuumOpts};
use super::scheduler::parse_schedule;

/// Canonical sweep name.
pub const CAS_VACUUM_NAME: &str = "cas_vacuum";

/// Default schedule — `30 4 * * 1` (Monday 04:30 UTC) per
/// `default_jobs()` row `retention.cas_vacuum`.
pub const CAS_VACUUM_SCHEDULE: &str = "30 4 * * 1";

/// CAS-vacuum sweep wrapped behind the [`Sweep`] trait.
pub struct CasVacuumSweep {
    cas_path: PathBuf,
    opts_template: VacuumOpts,
}

impl CasVacuumSweep {
    /// Build the sweep over the SQLite CAS DB at `cas_path` with
    /// `opts_template` as the per-run template. The wrapper clones
    /// the template each run and stamps `now` from the ctx.
    pub fn new(cas_path: PathBuf, opts_template: VacuumOpts) -> Self {
        Self {
            cas_path,
            opts_template,
        }
    }
}

#[async_trait]
impl Sweep for CasVacuumSweep {
    fn name(&self) -> &'static str {
        CAS_VACUUM_NAME
    }

    fn schedule(&self) -> Schedule {
        parse_schedule(CAS_VACUUM_SCHEDULE)
            .expect("CAS_VACUUM_SCHEDULE is a constant valid 5-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let cas_path = self.cas_path.clone();
        let mut opts = self.opts_template.clone();
        opts.now = ctx.now;
        opts.dry_run = ctx.config.dry_run || opts.dry_run;
        let started_at = ctx.now;

        let outcome = tokio::task::spawn_blocking(move || {
            let mut store = open_store(&cas_path).map_err(anyhow::Error::from)?;
            run_vacuum(&mut store, &opts).map_err(anyhow::Error::from)
        })
        .await?;

        let report = SweepReport::started(CAS_VACUUM_NAME, started_at);
        match outcome {
            Ok(legacy) => {
                let rows_processed = legacy.blobs_dropped;
                let bytes_reclaimed = legacy.bytes_reclaimed;
                let next = report
                    .with_tier_transition("blobs_dropped", legacy.blobs_dropped)
                    .with_tier_transition("total_blobs", legacy.total_blobs)
                    .with_tier_transition("did_vacuum", u64::from(legacy.did_vacuum))
                    .with_tier_transition("safeguard_tripped", u64::from(legacy.safeguard_tripped))
                    .with_tier_transition("vacuum_ms", legacy.vacuum_ms);
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
        DateTime::parse_from_rfc3339("2026-05-19T04:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.cas").with_now(now)
    }

    #[tokio::test]
    async fn cas_vacuum_on_empty_store_yields_success_zero_rows() {
        let dir = TempDir::new().unwrap();
        let cas_path = dir.path().join("cas.sqlite");
        let sweep = CasVacuumSweep::new(cas_path, VacuumOpts::default_for(fixed_now()));
        let ctx = make_ctx(fixed_now());
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, CAS_VACUUM_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
        assert_eq!(*report.tier_transitions.get("total_blobs").unwrap(), 0);
    }

    #[tokio::test]
    async fn cas_vacuum_dry_run_propagates_through_ctx() {
        let dir = TempDir::new().unwrap();
        let cas_path = dir.path().join("cas.sqlite");
        let sweep = CasVacuumSweep::new(cas_path, VacuumOpts::default_for(fixed_now()));
        let mut ctx = make_ctx(fixed_now());
        ctx.config.dry_run = true;
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
    }

    #[test]
    fn cas_vacuum_schedule_parses_and_name_is_canonical() {
        let dir = TempDir::new().unwrap();
        let cas_path = dir.path().join("cas.sqlite");
        let sweep = CasVacuumSweep::new(cas_path, VacuumOpts::default_for(fixed_now()));
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "cas_vacuum");
    }
}
