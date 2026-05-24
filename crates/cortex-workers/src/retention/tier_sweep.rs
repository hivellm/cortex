//! Phase13a §3.1 — `TierSweep` wraps the existing tier-transition
//! logic ([`run_sweep`]) behind the [`Sweep`] trait. This is the
//! first sweep to migrate; once landed, ADR-009 is promoted from
//! `proposed` to `accepted`.
//!
//! The wrapper carries:
//!
//! - `ops` — `Arc<dyn VectorizerOps>` so the production daemon and
//!   integration tests both fit one concrete `TierSweep` type
//!   without per-test generics polluting the registry.
//! - `plan` — a `SweepPlan` template; the wrapper clones it each
//!   `run` and stamps `now` from `SweepCtx::now`, so the trait
//!   surface is time-travellable.
//!
//! The `impl Sweep` translation is mechanical:
//!
//! | Sweep method | Body |
//! |---|---|
//! | `name` | `"tier_sweep"` (matches `cron_jobs.name = retention.sweep`'s leaf) |
//! | `schedule` | `"0 3 * * *"` — daily 03:00 UTC per `default_jobs()` |
//! | `run` | call `run_sweep(plan, ops)`, fold the legacy [`SweepReport`](super::SweepReport) into the new [`cortex_workers::sweep::SweepReport`] |
//! | `report_view` | `report.view()` |

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::{run_sweep, SweepPlan, VectorizerOps};

/// Canonical sweep name. Matches the leaf of the cron-jobs row
/// `retention.sweep` — the prefix lives at the scheduler layer and
/// the sweep itself only owns its short name.
pub const TIER_SWEEP_NAME: &str = "tier_sweep";

/// Default cron schedule — daily 03:00 UTC — matches the
/// `default_jobs()` entry for `retention.sweep`.
pub const TIER_SWEEP_SCHEDULE: &str = "0 0 3 * * * *";

/// Tier-transition sweep wrapped behind the [`Sweep`] trait. Holds
/// the Vectorizer backend handle plus the `SweepPlan` template the
/// daemon was using before the trait landed.
pub struct TierSweep {
    ops: Arc<dyn VectorizerOps>,
    plan_template: SweepPlan,
}

impl TierSweep {
    /// Build a `TierSweep` over `ops` with `plan_template` as the
    /// per-run template. The wrapper clones the template every
    /// `run` and stamps `now` from the ctx so tests are
    /// time-travellable without mutating shared state.
    pub fn new(ops: Arc<dyn VectorizerOps>, plan_template: SweepPlan) -> Self {
        Self { ops, plan_template }
    }
}

#[async_trait]
impl Sweep for TierSweep {
    fn name(&self) -> &'static str {
        TIER_SWEEP_NAME
    }

    fn schedule(&self) -> Schedule {
        Schedule::from_str(TIER_SWEEP_SCHEDULE)
            .expect("TIER_SWEEP_SCHEDULE is a constant valid 7-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let mut plan = self.plan_template.clone();
        plan.now = ctx.now;
        plan.dry_run = ctx.config.dry_run || plan.dry_run;
        let started_at = ctx.now;
        let report = SweepReport::started(TIER_SWEEP_NAME, started_at);
        match run_sweep(&plan, self.ops.as_ref()).await {
            Ok(legacy) => {
                // Map the legacy per-pair counters into the new
                // trait-level report. `rows_processed` is the union
                // of demoted + dropped — both pieces matter for the
                // dashboard's "did anything happen?" view.
                let mut next = report;
                for (key, count) in &legacy.tier_transitions {
                    next = next.with_tier_transition(key, *count);
                }
                let bytes_reclaimed = legacy.transitions.iter().map(|_| 0u64).sum::<u64>();
                let rows_processed = legacy.records_demoted + legacy.records_dropped;
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
    use crate::retention::{MemoryVectorizerOps, RecordRef, SweepKind, SweepPlan};
    use crate::sweep::SweepStatus;
    use chrono::{DateTime, Duration, Utc};
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn rec(id: &str, age_days: i64, now: DateTime<Utc>) -> RecordRef {
        RecordRef {
            event_id: id.to_string(),
            kind: SweepKind::Turn.as_str().to_string(),
            occurred_at: now - Duration::days(age_days),
            bytes: vec![0u8; 16],
        }
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.tier").with_now(now)
    }

    #[tokio::test]
    async fn tier_sweep_emits_success_report_with_per_pair_counters() {
        let now = fixed_now();
        let ops = Arc::new(MemoryVectorizerOps::new());
        ops.seed("cortex.turn.fp32", vec![rec("01TURN", 31, now)])
            .await;
        let sweep = TierSweep::new(ops.clone(), SweepPlan::default_for(now));
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, TIER_SWEEP_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 1);
        assert_eq!(*report.tier_transitions.get("turn:fp32->pq").unwrap(), 1);
        let view = sweep.report_view(&report);
        assert_eq!(view.status, SweepStatus::Success);
        assert_eq!(view.rows_processed, 1);
    }

    #[tokio::test]
    async fn tier_sweep_no_eligible_records_yields_zero_rows() {
        let now = fixed_now();
        let ops = Arc::new(MemoryVectorizerOps::new());
        // Single fresh record — younger than every threshold.
        ops.seed("cortex.turn.fp32", vec![rec("01FRESH", 5, now)])
            .await;
        let sweep = TierSweep::new(ops, SweepPlan::default_for(now));
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
        assert!(report.tier_transitions.is_empty());
    }

    #[tokio::test]
    async fn tier_sweep_error_rate_failure_lands_as_failed_status() {
        let now = fixed_now();
        let ops = Arc::new(MemoryVectorizerOps::new());
        // 3 eligible records, 1 upsert fails → 33% drop ≫ 5% ceiling.
        ops.seed(
            "cortex.turn.fp32",
            vec![
                rec("01A", 31, now),
                rec("01B", 31, now),
                rec("01C", 31, now),
            ],
        )
        .await;
        ops.inject_upsert_error_once("cortex.turn.pq", "synthetic")
            .await;
        let sweep = TierSweep::new(ops, SweepPlan::default_for(now));
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Failed);
        assert!(report
            .error_message
            .as_deref()
            .unwrap_or("")
            .contains("error rate"));
    }

    #[tokio::test]
    async fn tier_sweep_dry_run_via_ctx_does_not_mutate_collections() {
        let now = fixed_now();
        let ops = Arc::new(MemoryVectorizerOps::new());
        ops.seed("cortex.turn.fp32", vec![rec("01DRY", 31, now)])
            .await;
        let mut plan = SweepPlan::default_for(now);
        plan.dry_run = false; // Drive dry-run from the ctx, not the plan.
        let sweep = TierSweep::new(ops.clone(), plan);
        let mut ctx = make_ctx(now);
        ctx.config.dry_run = true;

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        // Counters acknowledge the eligible record …
        assert_eq!(report.rows_processed, 1);
        // … but the source collection is untouched.
        assert_eq!(ops.snapshot("cortex.turn.fp32").await.len(), 1);
        assert!(ops.snapshot("cortex.turn.pq").await.is_empty());
    }

    #[test]
    fn tier_sweep_schedule_parses() {
        let ops = Arc::new(MemoryVectorizerOps::new());
        let sweep = TierSweep::new(ops, SweepPlan::default_for(fixed_now()));
        let _schedule = sweep.schedule(); // panics on parse failure.
        assert_eq!(sweep.name(), "tier_sweep");
    }
}
