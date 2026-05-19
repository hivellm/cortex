//! Phase13a §3.4 — `PiiEnforceSweep` wraps the existing PII
//! enforcement orchestration ([`run_enforcement`]) behind the
//! [`Sweep`] trait.
//!
//! The wrapper holds:
//!
//! - `provider` — `Arc<dyn PiiTargetProvider>` that returns the
//!   per-run target slice. Production wires the live archive
//!   walker; the synthetic preview suite the `cortex-ops
//!   pii-enforce` bin runs today fits this slot too.
//! - `backend` — `Arc<dyn PiiBackend>`; either the live storage
//!   client or [`MemoryPiiBackend`] under test.
//! - `plan_template` — `EnforcementPlan` template; the wrapper
//!   clones it each run and stamps `now` from the ctx.
//!
//! Legacy [`EnforcementReport`] folds into [`SweepReport`] as
//! follows:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `examined` | `tier_transitions["examined"]` |
//! | `applied` | `rows_processed` (+ `tier_transitions["applied"]`) |
//! | `skipped` | `tier_transitions["skipped"]` |
//! | `null_safety_warnings` | `tier_transitions["null_safety_warnings"]` |
//! | per-cohort counts | `tier_transitions["cohort:<name>"]` |
//!
//! Bytes are not tracked at this layer (PII enforcement mutates
//! rows, it doesn't reclaim disk directly — that lands with
//! `cas_vacuum` afterwards), so `bytes_reclaimed` is always 0.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cron::Schedule;

use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::pii_enforce::{
    run_enforcement, EnforcementPlan, PiiBackend, PiiTarget,
};
use super::scheduler::parse_schedule;

/// Canonical sweep name.
pub const PII_ENFORCE_NAME: &str = "pii_enforce";

/// Default schedule — daily 05:00 UTC, matches `default_jobs()`
/// row `retention.pii_enforce` (5-field `"0 5 * * *"`).
pub const PII_ENFORCE_SCHEDULE: &str = "0 5 * * *";

/// Target provider — produces the per-run target slice. Splitting
/// this out keeps `PiiEnforceSweep` testable without a live archive
/// walker.
#[async_trait]
pub trait PiiTargetProvider: Send + Sync {
    /// Enumerate the targets the sweep should evaluate at `now`.
    /// The wrapper passes the result straight into
    /// [`run_enforcement`].
    async fn targets(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<PiiTarget>>;
}

/// Static target provider — returns a fixed slice. The synthetic
/// preview suite the `cortex-ops pii-enforce` bin uses fits this
/// shape, and tests author scenarios directly without a mock.
pub struct StaticTargets {
    targets: Vec<PiiTarget>,
}

impl StaticTargets {
    /// Build a static provider with the supplied target slice.
    pub fn new(targets: Vec<PiiTarget>) -> Self {
        Self { targets }
    }
}

#[async_trait]
impl PiiTargetProvider for StaticTargets {
    async fn targets(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<PiiTarget>> {
        Ok(self.targets.clone())
    }
}

/// PII enforcement sweep wrapped behind the [`Sweep`] trait.
pub struct PiiEnforceSweep {
    provider: Arc<dyn PiiTargetProvider>,
    backend: Arc<dyn PiiBackend>,
    plan_template: EnforcementPlan,
}

impl PiiEnforceSweep {
    /// Build the sweep over `provider` + `backend` with
    /// `plan_template` as the per-run enforcement template.
    pub fn new(
        provider: Arc<dyn PiiTargetProvider>,
        backend: Arc<dyn PiiBackend>,
        plan_template: EnforcementPlan,
    ) -> Self {
        Self {
            provider,
            backend,
            plan_template,
        }
    }
}

#[async_trait]
impl Sweep for PiiEnforceSweep {
    fn name(&self) -> &'static str {
        PII_ENFORCE_NAME
    }

    fn schedule(&self) -> Schedule {
        parse_schedule(PII_ENFORCE_SCHEDULE)
            .expect("PII_ENFORCE_SCHEDULE is a constant valid 5-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let mut plan = self.plan_template.clone();
        plan.now = ctx.now;
        plan.dry_run = ctx.config.dry_run || plan.dry_run;
        let started_at = ctx.now;

        let targets = self.provider.targets(ctx.now).await?;
        let outcome = run_enforcement(&plan, self.backend.as_ref(), targets).await;

        let report = SweepReport::started(PII_ENFORCE_NAME, started_at);
        match outcome {
            Ok(legacy) => {
                let mut next = report
                    .with_tier_transition("examined", legacy.examined)
                    .with_tier_transition("applied", legacy.applied)
                    .with_tier_transition("skipped", legacy.skipped)
                    .with_tier_transition(
                        "null_safety_warnings",
                        legacy.null_safety_warnings,
                    );
                for (cohort, count) in &legacy.cohort_counts {
                    next = next.with_tier_transition(&format!("cohort:{cohort}"), *count);
                }
                Ok(next.finish_success(ctx.now, legacy.applied, 0))
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
    use crate::retention::pii_enforce::{
        EnforcementPlan, MemoryPiiBackend, PiiRisk, PiiTarget,
    };
    use crate::sweep::SweepStatus;
    use chrono::Duration;
    use cortex_storage::MetadataStore;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T05:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.pii").with_now(now)
    }

    fn synthetic_targets(now: DateTime<Utc>) -> Vec<PiiTarget> {
        vec![
            PiiTarget {
                event_id: "01HIGH".into(),
                kind: "turn".into(),
                pii_risk: Some(PiiRisk::High),
                occurred_at: now - Duration::days(31),
                body_ref: Some("sha256:high".into()),
                redacted: None,
            },
            PiiTarget {
                event_id: "01MEDIUM".into(),
                kind: "turn".into(),
                pii_risk: Some(PiiRisk::Medium),
                occurred_at: now - Duration::days(91),
                body_ref: Some("sha256:medium".into()),
                redacted: None,
            },
            PiiTarget {
                event_id: "01FRESH".into(),
                kind: "turn".into(),
                pii_risk: Some(PiiRisk::High),
                occurred_at: now - Duration::days(5),
                body_ref: None,
                redacted: None,
            },
        ]
    }

    #[tokio::test]
    async fn pii_enforce_with_static_targets_records_cohort_counts() {
        let now = fixed_now();
        let provider = Arc::new(StaticTargets::new(synthetic_targets(now)));
        let backend = Arc::new(MemoryPiiBackend::new());
        let sweep = PiiEnforceSweep::new(provider, backend, EnforcementPlan::default_for(now));
        let ctx = make_ctx(now);
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(*report.tier_transitions.get("examined").unwrap(), 3);
        // High + Medium applied = 2; Fresh skipped.
        assert_eq!(*report.tier_transitions.get("applied").unwrap(), 2);
        assert_eq!(*report.tier_transitions.get("skipped").unwrap(), 1);
        assert!(report.tier_transitions.contains_key("cohort:high_30d"));
        assert!(report.tier_transitions.contains_key("cohort:medium_90d"));
    }

    #[tokio::test]
    async fn pii_enforce_dry_run_via_ctx_applies_zero_rows() {
        let now = fixed_now();
        let provider = Arc::new(StaticTargets::new(synthetic_targets(now)));
        let backend = Arc::new(MemoryPiiBackend::new());
        let sweep = PiiEnforceSweep::new(provider, backend, EnforcementPlan::default_for(now));
        let mut ctx = make_ctx(now);
        ctx.config.dry_run = true;
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        // Dry-run path: examined > 0, applied == 0.
        assert_eq!(*report.tier_transitions.get("examined").unwrap(), 3);
        assert_eq!(*report.tier_transitions.get("applied").unwrap(), 0);
    }

    #[test]
    fn pii_enforce_schedule_parses_and_name_is_canonical() {
        let provider = Arc::new(StaticTargets::new(Vec::new()));
        let backend = Arc::new(MemoryPiiBackend::new());
        let sweep =
            PiiEnforceSweep::new(provider, backend, EnforcementPlan::default_for(fixed_now()));
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "pii_enforce");
    }
}
