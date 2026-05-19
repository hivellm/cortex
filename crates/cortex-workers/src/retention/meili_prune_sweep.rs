//! Phase13a §3.5 — `MeiliPruneSweep` wraps the existing Meili
//! archival prune orchestration ([`run_meili_prune`]) behind the
//! [`Sweep`] trait.
//!
//! The wrapper holds:
//!
//! - `backend` — `Arc<dyn MeiliBackend>` (production live SDK or
//!   [`MemoryMeiliBackend`] under test).
//! - `plan_template` — `PrunePlan` template; the wrapper clones it
//!   each run and stamps `now` from the ctx.
//!
//! Legacy [`PruneReport`] folds into the trait-level
//! [`SweepReport`]:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `examined` | `tier_transitions["examined"]` |
//! | `pruned` | `rows_processed` (+ `tier_transitions["pruned"]`) |
//! | `summaries_capped` | `tier_transitions["summaries_capped"]` |
//! | `skipped` | `tier_transitions["skipped"]` |
//! | `per_index[<index>]` | `tier_transitions["index:<index>"]` |
//!
//! Bytes reclaimed are not measurable from the Meili partial-update
//! payload (Meili compaction happens lazily), so `bytes_reclaimed`
//! is always 0.

use std::sync::Arc;

use async_trait::async_trait;
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::meili_prune::{run_meili_prune, MeiliBackend, PrunePlan};
use super::scheduler::parse_schedule;

/// Canonical sweep name.
pub const MEILI_PRUNE_NAME: &str = "meili_prune";

/// Default schedule — daily 05:30 UTC, matches `default_jobs()`
/// row `retention.meili_prune` (5-field `"30 5 * * *"`).
pub const MEILI_PRUNE_SCHEDULE: &str = "30 5 * * *";

/// Meili-prune sweep wrapped behind the [`Sweep`] trait.
pub struct MeiliPruneSweep {
    backend: Arc<dyn MeiliBackend>,
    plan_template: PrunePlan,
}

impl MeiliPruneSweep {
    /// Build the sweep over `backend` with `plan_template` as the
    /// per-run template.
    pub fn new(backend: Arc<dyn MeiliBackend>, plan_template: PrunePlan) -> Self {
        Self {
            backend,
            plan_template,
        }
    }
}

#[async_trait]
impl Sweep for MeiliPruneSweep {
    fn name(&self) -> &'static str {
        MEILI_PRUNE_NAME
    }

    fn schedule(&self) -> Schedule {
        parse_schedule(MEILI_PRUNE_SCHEDULE)
            .expect("MEILI_PRUNE_SCHEDULE is a constant valid 5-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let mut plan = self.plan_template.clone();
        plan.now = ctx.now;
        plan.dry_run = ctx.config.dry_run || plan.dry_run;
        let started_at = ctx.now;

        let outcome = run_meili_prune(&plan, self.backend.as_ref()).await;

        let report = SweepReport::started(MEILI_PRUNE_NAME, started_at);
        match outcome {
            Ok(legacy) => {
                let mut next = report
                    .with_tier_transition("examined", legacy.examined)
                    .with_tier_transition("pruned", legacy.pruned)
                    .with_tier_transition("summaries_capped", legacy.summaries_capped)
                    .with_tier_transition("skipped", legacy.skipped);
                for (index, count) in &legacy.per_index {
                    next = next.with_tier_transition(&format!("index:{index}"), *count);
                }
                Ok(next.finish_success(ctx.now, legacy.pruned, 0))
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
    use crate::retention::meili_prune::{MemoryMeiliBackend, PrunePlan};
    use crate::sweep::SweepStatus;
    use chrono::{DateTime, Utc};
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T05:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.meili").with_now(now)
    }

    #[tokio::test]
    async fn meili_prune_empty_backend_yields_zero_rows() {
        let backend = Arc::new(MemoryMeiliBackend::new());
        let sweep =
            MeiliPruneSweep::new(backend, PrunePlan::default_for(fixed_now()));
        let ctx = make_ctx(fixed_now());
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, MEILI_PRUNE_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
        assert_eq!(*report.tier_transitions.get("examined").unwrap(), 0);
    }

    #[tokio::test]
    async fn meili_prune_dry_run_via_ctx_does_not_mutate() {
        let backend = Arc::new(MemoryMeiliBackend::new());
        let sweep =
            MeiliPruneSweep::new(backend, PrunePlan::default_for(fixed_now()));
        let mut ctx = make_ctx(fixed_now());
        ctx.config.dry_run = true;
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
    }

    #[test]
    fn meili_prune_schedule_parses_and_name_is_canonical() {
        let backend = Arc::new(MemoryMeiliBackend::new());
        let sweep =
            MeiliPruneSweep::new(backend, PrunePlan::default_for(fixed_now()));
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "meili_prune");
    }
}
