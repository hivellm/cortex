//! Phase13a — `Sweep` trait, the single contract for retention /
//! digest / pruning sweeps.
//!
//! Reference: ADR-009 (`adr-009-sweep-trait-as-single-contract-for-
//! retention-digest-pruning-sweeps`),
//! `docs/analysis/rework/04-architecture.md` §A.1.
//!
//! Background. Seven sweeps — `tier_sweep`, `parquet_rollup`,
//! `cas_vacuum`, `pii_enforce`, `meili_prune`, `metadata_reap`,
//! `consolidation_prune` — were each bolted on standalone with their
//! own cron wiring, dashboard story, error path, and bookkeeping. The
//! 2026-05-05 retention-daemon learning called out the shared shape
//! bug: "no shared 'I am running as a sweep' wrapper exists". Until
//! the contract is uniform, every new sweep added in Phase B/C
//! reintroduces the same defect class (dashboard hardcoded
//! `next_run: "never"`, missing `retention_sweeps` rows, drifting
//! status taxonomies).
//!
//! The trait surfaces a four-method contract:
//!
//! ```ignore
//! #[async_trait]
//! pub trait Sweep: Send + Sync {
//!     fn name(&self) -> &'static str;
//!     fn schedule(&self) -> Schedule;
//!     async fn run(&self, ctx: &SweepCtx) -> Result<SweepReport>;
//!     fn report_view(&self, report: &SweepReport) -> SweepReportView;
//! }
//! ```
//!
//! Layering:
//! - [`Sweep`] (`r#trait.rs`) — the contract every sweep implements.
//! - [`SweepCtx`] (`ctx.rs`) — shared environment passed to
//!   `Sweep::run`: metadata store handle, reference time, common
//!   config knobs, logger target. Per-backend handles (Vectorizer,
//!   Meili, Nexus) live on the `impl Sweep` struct itself so we do
//!   not pull every backend SDK into the trait's surface.
//! - [`SweepReport`] (`report.rs`) — uniform per-invocation outcome
//!   the scheduler writes to `retention_sweeps`; [`SweepReportView`]
//!   is the dashboard projection that dropper handler-side state
//!   literals.
//!
//! Status taxonomy. `SweepStatus` mirrors the values
//! `retention_sweeps.status` already uses (`running`, `success`,
//! `failed`, `abandoned`). Phase13a does NOT migrate the SQLite
//! schema; the trait layer maps the existing columns through
//! `SweepReport`. Phase B (Consolidator-as-Sweep, Pruner-as-Sweep)
//! introduces a follow-up task to add `name`, `bytes_reclaimed`,
//! `rows_processed`, and `error_message` columns so the scheduler
//! can persist the full report without per-sweep ad-hoc joins.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ctx;
pub mod registry;
pub mod report;
#[path = "trait.rs"]
pub mod sweep_trait;

pub use ctx::{MetadataHandle, SweepConfig, SweepCtx};
pub use registry::{canonical_registry, into_handle, RegistryError, SweepRegistry};
pub use report::{SweepReport, SweepReportView, SweepStatus};
pub use sweep_trait::Sweep;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use cron::Schedule;
    use std::str::FromStr;

    /// A minimal in-test sweep that proves the trait shape compiles
    /// and is object-safe. Used by the §2.4 unit tests as a stand-in
    /// for the seven production sweeps before §3.x lands.
    struct NopSweep {
        name: &'static str,
    }

    #[async_trait]
    impl Sweep for NopSweep {
        fn name(&self) -> &'static str {
            self.name
        }
        fn schedule(&self) -> Schedule {
            Schedule::from_str("0 0 * * * * *").expect("valid cron")
        }
        async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
            Ok(SweepReport::started(self.name, ctx.now).finish_success(ctx.now, 0, 0))
        }
        fn report_view(&self, report: &SweepReport) -> SweepReportView {
            report.view()
        }
    }

    /// Trait must be object-safe so [`Sweep`] can be stored as
    /// `Box<dyn Sweep>` (§4.1 registry).
    #[test]
    fn sweep_trait_is_object_safe() {
        let _: Box<dyn Sweep> = Box::new(NopSweep { name: "noop" });
    }

    /// `SweepCtx` is `Send + Sync` so the scheduler can hand it
    /// across the tokio runtime boundary.
    #[test]
    fn sweep_ctx_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SweepCtx>();
    }

    /// `SweepReport` round-trips through serde so the scheduler can
    /// persist it to `retention_sweeps.tier_transitions_json` (and,
    /// once the schema migration lands in Phase B, to the new
    /// columns) without re-shaping.
    #[test]
    fn sweep_report_round_trips_via_serde() {
        let start: DateTime<Utc> = "2026-05-19T12:00:00Z".parse().unwrap();
        let end = start + chrono::Duration::seconds(30);
        let report = SweepReport::started("tier_sweep", start)
            .with_tier_transition("turn:fp32->pq", 4)
            .finish_success(end, 4, 1024);
        let json = serde_json::to_string(&report).unwrap();
        let parsed: SweepReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);
    }

    /// `report_view` is a pure projection — same report, same view.
    #[tokio::test]
    async fn report_view_is_pure_projection() {
        let now: DateTime<Utc> = "2026-05-19T12:00:00Z".parse().unwrap();
        let s = NopSweep { name: "noop" };
        let r1 = s.report_view(&SweepReport::started("noop", now).finish_success(now, 0, 0));
        let r2 = s.report_view(&SweepReport::started("noop", now).finish_success(now, 0, 0));
        assert_eq!(r1, r2);
    }
}
