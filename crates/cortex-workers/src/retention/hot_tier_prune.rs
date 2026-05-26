//! Phase14b §2 — [`HotTierPrune`] wraps the
//! [`super::identity_prune::run_identity_cascade`] driver behind the
//! [`Sweep`] trait. Runs daily at 04:00 UTC with a default cutoff
//! of 90 days; cascades per-event deletes across Meili + Nexus +
//! Vectorizer FP32/PQ, leaving the parquet archive intact so the
//! cold-tier sweep (§3) can rewrite it on the weekly schedule.
//!
//! The wrapper composes with [`super::identity_prune::IdentitySource`]
//! plus [`super::identity_prune::IdentityCascadeOps`] so production
//! wires the SQLite plus per-backend SDK impls while the unit and IT
//! tests drive the cascade through the in-memory recorder.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Duration;
use cron::Schedule;

use crate::retention::identity_prune::{
    render_cascade_summary, run_identity_cascade, CascadePolicy, IdentityCascadeOps,
    IdentitySource,
};
use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

/// Canonical sweep name — surfaces in `retention_sweeps.name` and
/// in dashboard rows.
pub const HOT_TIER_PRUNE_NAME: &str = "hot_tier_prune";

/// Default cron schedule — daily at 04:00 UTC. Picked one hour
/// after the existing `tier_sweep` (03:00 UTC) so the tier-transition
/// pass lands first and the hot-tier eviction sees the post-transition
/// state.
pub const HOT_TIER_PRUNE_SCHEDULE: &str = "0 0 4 * * * *";

/// Default cutoff in days — events older than this leave the
/// query-side backends. Operator override sits behind the cron
/// config layer (phase19/retention spec).
pub const HOT_TIER_DEFAULT_CUTOFF_DAYS: i64 = 90;

/// Identity-driven hot-tier prune Sweep. Drops Meili + Nexus +
/// per-event Vectorizer rows; the parquet archive stays.
pub struct HotTierPrune {
    source: Arc<dyn IdentitySource>,
    ops: Arc<dyn IdentityCascadeOps>,
    cutoff_days: i64,
}

impl HotTierPrune {
    /// Build a hot-tier prune with the default 90-day cutoff.
    pub fn new(source: Arc<dyn IdentitySource>, ops: Arc<dyn IdentityCascadeOps>) -> Self {
        Self {
            source,
            ops,
            cutoff_days: HOT_TIER_DEFAULT_CUTOFF_DAYS,
        }
    }

    /// Override the cutoff (for tests + operator overrides).
    #[must_use]
    pub fn with_cutoff_days(mut self, days: i64) -> Self {
        self.cutoff_days = days;
        self
    }

    /// Active cutoff in days. Public so the dashboard can render it.
    pub fn cutoff_days(&self) -> i64 {
        self.cutoff_days
    }
}

#[async_trait]
impl Sweep for HotTierPrune {
    fn name(&self) -> &'static str {
        HOT_TIER_PRUNE_NAME
    }

    fn schedule(&self) -> Schedule {
        Schedule::from_str(HOT_TIER_PRUNE_SCHEDULE)
            .expect("HOT_TIER_PRUNE_SCHEDULE is a constant valid 7-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let started = ctx.now;
        let report = SweepReport::started(HOT_TIER_PRUNE_NAME, started);
        if ctx.config.dry_run {
            // Dry-run path — still query the source so the operator
            // sees how many rows WOULD have been touched, but skip
            // every cascade leg. The shared driver does not yet have
            // a dry-run knob; phase14b ships dry-run by short-
            // circuiting at the sweep boundary.
            let cutoff = ctx.now - Duration::days(self.cutoff_days);
            let expired = self
                .source
                .expired_identities(cutoff)
                .await
                .map_err(|e| anyhow::anyhow!("dry-run expired query: {e}"))?;
            let next = report.with_tier_transition("hot:dry_run", expired.len() as u64);
            return Ok(next.finish_success(ctx.now, expired.len() as u64, 0));
        }
        let cutoff = ctx.now - Duration::days(self.cutoff_days);
        let cascade = run_identity_cascade(
            Arc::clone(&self.source),
            Arc::clone(&self.ops),
            CascadePolicy::HOT,
            cutoff,
        )
        .await;
        match cascade {
            Ok(c) => {
                let next = report
                    .with_tier_transition("hot:cascade_ok", c.ok)
                    .with_tier_transition("hot:cascade_failed", c.failed);
                let summary = render_cascade_summary(CascadePolicy::HOT, &c);
                tracing::info!(sweep = HOT_TIER_PRUNE_NAME, %summary, "hot-tier prune done");
                if c.failed > 0 {
                    Ok(next.finish_failed(
                        ctx.now,
                        c.processed,
                        0,
                        format!("{summary} ({} of {} cascade legs failed)", c.failed, c.processed),
                    ))
                } else {
                    Ok(next.finish_success(ctx.now, c.processed, 0))
                }
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
    use crate::retention::identity_prune::{RecordingCascadeOps, StaticIdentitySource};
    use crate::sweep::SweepStatus;
    use chrono::{DateTime, Utc};
    use cortex_storage::identity::EventIdentity;
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex as TokioMutex;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("rfc3339")
    }

    fn row(event: &str, partition: &str) -> EventIdentity {
        EventIdentity {
            event_id: event.into(),
            nexus_id: Some(format!("nxs-{event}")),
            vec_id: Some(format!("vec-{event}")),
            meili_id: Some(format!("mli-{event}")),
            archive_partition: Some(partition.into()),
        }
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(TokioMutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.hot_tier_prune").with_now(now)
    }

    #[tokio::test]
    async fn hot_tier_prune_drops_expired_and_reports_success() {
        let now = ts("2026-05-25T04:00:00Z");
        let source = Arc::new(StaticIdentitySource::new(vec![
            // 100 days old → expired at the 90-day cutoff.
            row(
                "e-old",
                "events/year=2026/month=02/day=15/hour=00/raw.parquet",
            ),
            // 10 days old → kept.
            row(
                "e-young",
                "events/year=2026/month=05/day=15/hour=00/raw.parquet",
            ),
        ]));
        let ops = Arc::new(RecordingCascadeOps::new());
        let sweep = HotTierPrune::new(source.clone(), ops.clone());
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, HOT_TIER_PRUNE_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 1, "one expired event");

        assert_eq!(ops.meili_calls().await, vec!["mli-e-old"]);
        assert_eq!(ops.nexus_calls().await, vec!["nxs-e-old"]);
        assert_eq!(ops.vector_calls().await, vec!["vec-e-old"]);
        assert!(
            ops.archive_calls().await.is_empty(),
            "hot tier keeps archive intact"
        );
        // Identity row dropped for the expired event, young event preserved.
        let known = source.known_ids().await;
        assert!(!known.contains("e-old"));
        assert!(known.contains("e-young"));
    }

    #[tokio::test]
    async fn hot_tier_prune_dry_run_via_ctx_skips_every_leg() {
        let now = ts("2026-05-25T04:00:00Z");
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-old",
            "events/year=2026/month=02/day=15/hour=00/raw.parquet",
        )]));
        let ops = Arc::new(RecordingCascadeOps::new());
        let sweep = HotTierPrune::new(source.clone(), ops.clone());
        let mut ctx = make_ctx(now);
        ctx.config.dry_run = true;

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 1);
        assert!(ops.meili_calls().await.is_empty(), "dry-run hits no legs");
        // Identity row preserved on dry-run.
        assert!(source.known_ids().await.contains("e-old"));
    }

    #[tokio::test]
    async fn hot_tier_prune_partial_failure_lands_as_failed_status() {
        let now = ts("2026-05-25T04:00:00Z");
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-1",
            "events/year=2026/month=02/day=15/hour=00/raw.parquet",
        )]));
        let ops = Arc::new(RecordingCascadeOps::new());
        ops.inject_meili_failure("synthetic").await;
        let sweep = HotTierPrune::new(source.clone(), ops);
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Failed);
        assert!(report
            .error_message
            .as_deref()
            .unwrap_or("")
            .contains("cascade legs failed"));
        // Identity row preserved — next sweep retries.
        assert!(source.known_ids().await.contains("e-1"));
    }

    #[test]
    fn hot_tier_prune_schedule_parses_and_cutoff_default_is_90_days() {
        let source = Arc::new(StaticIdentitySource::new(vec![]));
        let ops = Arc::new(RecordingCascadeOps::new());
        let sweep = HotTierPrune::new(source, ops);
        let _ = sweep.schedule(); // panics on parse failure
        assert_eq!(sweep.name(), "hot_tier_prune");
        assert_eq!(sweep.cutoff_days(), HOT_TIER_DEFAULT_CUTOFF_DAYS);
    }
}
