//! Phase13a §3.6 — `MetadataReapSweep` wraps the existing SQLite
//! metadata reaper ([`metadata_reap::run`]) behind the [`Sweep`]
//! trait.
//!
//! The wrapper holds a `ReapPlan` template; the SQLite connection
//! comes from `SweepCtx::metadata` so the reaper writes back to the
//! same `metadata.sqlite` the scheduler already owns. Each run
//! acquires the async lock, snapshots `now` from the ctx, and calls
//! `run(&mut conn, &plan)` synchronously — the reaper is a single
//! `BEGIN IMMEDIATE` transaction per target so the lock is held
//! only long enough to commit.
//!
//! Legacy [`ReapReport`] folds into [`SweepReport`]:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `bootstrap_jobs_collapsed` | `tier_transitions["bootstrap_jobs_collapsed"]` |
//! | `bootstrap_daily_buckets` | `tier_transitions["bootstrap_daily_buckets"]` |
//! | `sessions_collapsed` | `tier_transitions["sessions_collapsed"]` |
//! | `sessions_monthly_buckets` | `tier_transitions["sessions_monthly_buckets"]` |
//! | `spend_collapsed` | `tier_transitions["spend_collapsed"]` |
//! | `spend_monthly_buckets` | `tier_transitions["spend_monthly_buckets"]` |
//! | `did_vacuum` | `tier_transitions["did_vacuum"]` (0/1) |
//! | `vacuum_ms` | `tier_transitions["vacuum_ms"]` |
//!
//! `rows_processed = bootstrap_jobs_collapsed + sessions_collapsed +
//! spend_collapsed`. `bytes_reclaimed` is 0 — SQLite reports the
//! free-pages ratio rather than absolute bytes; the trait-level
//! field is preserved for future schema migrations.

use async_trait::async_trait;
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::metadata_reap::{run as run_reap, ReapPlan};
use super::scheduler::parse_schedule;

/// Canonical sweep name.
pub const METADATA_REAP_NAME: &str = "metadata_reap";

/// Default schedule — daily 05:45 UTC, matches `default_jobs()`
/// row `retention.metadata_reap` (5-field `"45 5 * * *"`).
pub const METADATA_REAP_SCHEDULE: &str = "45 5 * * *";

/// SQLite metadata-reap sweep wrapped behind the [`Sweep`] trait.
pub struct MetadataReapSweep {
    plan_template: ReapPlan,
}

impl MetadataReapSweep {
    /// Build the sweep with `plan_template` as the per-run template.
    pub fn new(plan_template: ReapPlan) -> Self {
        Self { plan_template }
    }
}

#[async_trait]
impl Sweep for MetadataReapSweep {
    fn name(&self) -> &'static str {
        METADATA_REAP_NAME
    }

    fn schedule(&self) -> Schedule {
        parse_schedule(METADATA_REAP_SCHEDULE)
            .expect("METADATA_REAP_SCHEDULE is a constant valid 5-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let mut plan = self.plan_template.clone();
        plan.now = ctx.now;
        plan.dry_run = ctx.config.dry_run || plan.dry_run;
        let started_at = ctx.now;

        // Lock + run synchronously. The reaper holds the lock only
        // for the duration of one `BEGIN IMMEDIATE` transaction per
        // target plus an optional `VACUUM` outside the txn.
        let mut store = ctx.metadata.lock().await;
        let outcome = run_reap(store.conn_mut(), &plan);
        drop(store);

        let report = SweepReport::started(METADATA_REAP_NAME, started_at);
        match outcome {
            Ok(legacy) => {
                let rows_processed = legacy.bootstrap_jobs_collapsed
                    + legacy.sessions_collapsed
                    + legacy.spend_collapsed;
                let next = report
                    .with_tier_transition(
                        "bootstrap_jobs_collapsed",
                        legacy.bootstrap_jobs_collapsed,
                    )
                    .with_tier_transition(
                        "bootstrap_daily_buckets",
                        legacy.bootstrap_daily_buckets,
                    )
                    .with_tier_transition("sessions_collapsed", legacy.sessions_collapsed)
                    .with_tier_transition(
                        "sessions_monthly_buckets",
                        legacy.sessions_monthly_buckets,
                    )
                    .with_tier_transition("spend_collapsed", legacy.spend_collapsed)
                    .with_tier_transition(
                        "spend_monthly_buckets",
                        legacy.spend_monthly_buckets,
                    )
                    .with_tier_transition("did_vacuum", u64::from(legacy.did_vacuum))
                    .with_tier_transition("vacuum_ms", legacy.vacuum_ms);
                Ok(next.finish_success(ctx.now, rows_processed, 0))
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
    use crate::retention::metadata_reap::ReapPlan;
    use crate::sweep::SweepStatus;
    use chrono::{DateTime, Utc};
    use cortex_storage::MetadataStore;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T05:45:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.metadata_reap").with_now(now)
    }

    #[tokio::test]
    async fn metadata_reap_on_empty_store_yields_zero_rows() {
        let sweep = MetadataReapSweep::new(ReapPlan::default_for(fixed_now()));
        let ctx = make_ctx(fixed_now());
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, METADATA_REAP_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
    }

    #[tokio::test]
    async fn metadata_reap_dry_run_via_ctx_propagates() {
        let sweep = MetadataReapSweep::new(ReapPlan::default_for(fixed_now()));
        let mut ctx = make_ctx(fixed_now());
        ctx.config.dry_run = true;
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
    }

    #[test]
    fn metadata_reap_schedule_parses_and_name_is_canonical() {
        let sweep = MetadataReapSweep::new(ReapPlan::default_for(fixed_now()));
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "metadata_reap");
    }
}
