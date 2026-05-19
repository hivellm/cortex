//! The [`Sweep`] trait — the single contract every retention /
//! digest / pruning sweep implements.
//!
//! Defined in its own file per ADR-009 §2.1; the parent module
//! `sweep/mod.rs` mounts it via `#[path = "trait.rs"] pub mod
//! sweep_trait;` because `trait` is a reserved keyword.

use async_trait::async_trait;
use cron::Schedule;

use super::{SweepCtx, SweepReport, SweepReportView};

/// The single contract every retention / digest / pruning sweep
/// implements. Phase13a §2.2.
///
/// Invariants every implementor SHALL uphold:
///
/// 1. `name()` returns a stable `'static` identifier used as the
///    `cron_jobs.name` and the `retention_sweeps` row key. Two
///    sweeps MUST NOT share a name.
/// 2. `schedule()` returns the canonical cron expression for the
///    sweep. The scheduler picks the next-run time from this; the
///    sweep itself does not own timing.
/// 3. `run(ctx)` is idempotent within the abandon-grace window. A
///    second invocation while a previous one is still `running`
///    SHOULD short-circuit via the metadata-level advisory lock —
///    `SweepCtx::metadata::start_retention_sweep` already enforces
///    this; the trait method just has to surface the error.
/// 4. `report_view(&self, report)` is a pure projection — same
///    report in, same view out. The dashboard reads only views, so
///    deriving anything else there is a Law violation
///    (ADR-014 / §4.3).
///
/// Object-safety: the trait MUST stay object-safe so the scheduler
/// can own a `Vec<Box<dyn Sweep>>` registry (§4.1). The unit test
/// `sweep_trait_is_object_safe` in `sweep/mod.rs` is the regression
/// guard.
#[async_trait]
pub trait Sweep: Send + Sync {
    /// Stable `'static` identifier — `tier_sweep`, `parquet_rollup`,
    /// etc. Used as the `cron_jobs.name` and as the
    /// `retention_sweeps` row key.
    fn name(&self) -> &'static str;

    /// Canonical cron expression for this sweep. The scheduler
    /// queries this on each tick to recompute `next_run_at`.
    fn schedule(&self) -> Schedule;

    /// Execute the sweep. Returns a [`SweepReport`] on success;
    /// surfaces `anyhow::Error` on unrecoverable failure (the
    /// scheduler logs the message into `SweepReport::error_message`
    /// of the abandoned row).
    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport>;

    /// Project a `SweepReport` into the dashboard view. Pure — see
    /// invariant 4 above.
    fn report_view(&self, report: &SweepReport) -> SweepReportView;
}
