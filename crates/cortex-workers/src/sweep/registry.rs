//! Phase13a §4.1 + §4.2 — the [`SweepRegistry`] is the in-process
//! supervisor that invokes every registered [`Sweep`] uniformly and
//! writes one `retention_sweeps` row per invocation.
//!
//! Architecture (ADR-009):
//!
//! ```text
//!   ┌────────────────────────────────────────────────────┐
//!   │  SweepRegistry<Vec<Box<dyn Sweep>>>                │
//!   │                                                    │
//!   │  for each sweep:                                   │
//!   │    1. mint sweep_id (ULID)                         │
//!   │    2. metadata.start_retention_sweep(…)            │
//!   │    3. sweep.run(&ctx)  ──► SweepReport             │
//!   │    4. metadata.finish_retention_sweep(             │
//!   │         sweep_id, finished_at, rows_processed,     │
//!   │         dropped, full_report_json, status)         │
//!   │                                                    │
//!   └────────────────────────────────────────────────────┘
//! ```
//!
//! The supervisor writes the **full** [`SweepReport`] (serialised
//! as JSON) into the existing
//! `retention_sweeps.tier_transitions_json` column. Dashboard /
//! API readers should parse this back into [`SweepReport`] to
//! materialise the [`SweepReportView`] without re-deriving anything
//! on the handler side (ADR-014).
//!
//! Schema delta for Phase B: the existing schema retains
//! tier-specific column names (`records_demoted`,
//! `records_dropped`) — fine for the tier sweep, lossy for the
//! other six. We fold the trait-level `rows_processed` into
//! `records_demoted` so legacy readers continue to see a non-zero
//! invocation total, and the full report lives in the JSON column
//! for new readers. Phase B's schema migration (`name`,
//! `bytes_reclaimed`, `rows_processed`, `error_message` columns)
//! retires this fold.
//!
//! Concurrency: sweeps run **serially** within one `run_all` call
//! because the existing `start_retention_sweep` advisory lock is
//! global — two `running` rows are not allowed simultaneously.
//! Phase B introduces per-sweep locks; for now we walk the
//! registry in declaration order.

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::Mutex;

use cortex_storage::{MetadataError, MetadataStore};

use super::ctx::SweepCtx;
use super::report::SweepReport;
use super::sweep_trait::Sweep;

/// Errors surfaced by the supervisor when persistence fails.
/// Sweep-internal failures land in the per-sweep `SweepReport`
/// and do NOT short-circuit the registry walk.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// `start_retention_sweep` failed — most likely the global
    /// advisory lock is held by a stuck `running` row older than
    /// the abandon-grace window.
    #[error("metadata.start_retention_sweep: {0}")]
    Start(MetadataError),
    /// `finish_retention_sweep` failed after the sweep itself ran
    /// successfully. The sweep work landed in the underlying
    /// backends; only the bookkeeping row is missing.
    #[error("metadata.finish_retention_sweep: {0}")]
    Finish(MetadataError),
}

/// Owned registry of `Sweep` implementations. The supervisor walks
/// these in declaration order on every `run_all`.
pub struct SweepRegistry {
    sweeps: Vec<Box<dyn Sweep>>,
}

impl SweepRegistry {
    /// Build an empty registry. Use [`Self::with`] when the full
    /// sweep set is known up front.
    pub fn new() -> Self {
        Self { sweeps: Vec::new() }
    }

    /// Build a registry seeded with `sweeps`. Declaration order is
    /// preserved across the walk.
    pub fn with(sweeps: Vec<Box<dyn Sweep>>) -> Self {
        Self { sweeps }
    }

    /// Register one more sweep.
    pub fn push(&mut self, sweep: Box<dyn Sweep>) {
        self.sweeps.push(sweep);
    }

    /// Canonical names of every registered sweep, declaration
    /// order. Used by `/v1/dashboard/retention/sweeps` to render
    /// the empty-state UI even before the first invocation.
    pub fn names(&self) -> Vec<&'static str> {
        self.sweeps.iter().map(|s| s.name()).collect()
    }

    /// Run every registered sweep against `ctx`. Returns the
    /// per-sweep reports in declaration order. Persistence failures
    /// short-circuit and return [`RegistryError`]; sweep-internal
    /// failures land in the per-sweep `SweepReport` (status =
    /// `Failed`, `error_message` populated) and the walk continues.
    pub async fn run_all(&self, ctx: &SweepCtx) -> Result<Vec<SweepReport>, RegistryError> {
        let mut reports = Vec::with_capacity(self.sweeps.len());
        for sweep in &self.sweeps {
            let report = run_one(sweep.as_ref(), ctx).await?;
            reports.push(report);
        }
        Ok(reports)
    }
}

impl Default for SweepRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Drive one sweep through the start → run → finish path. Returns
/// the final `SweepReport` (which already carries the failed
/// status when the sweep itself returned an error).
async fn run_one(sweep: &dyn Sweep, ctx: &SweepCtx) -> Result<SweepReport, RegistryError> {
    let sweep_id = ulid::Ulid::new().to_string();
    let started_at = ctx.now;

    // Persist the `running` row up front. The advisory lock is
    // honoured here: `start_retention_sweep` returns Busy when
    // another sweep is mid-flight within the grace window.
    {
        let store = ctx.metadata.lock().await;
        store
            .start_retention_sweep(&sweep_id, started_at, ctx.config.abandon_grace_secs)
            .map_err(RegistryError::Start)?;
    }

    // Run the sweep itself. `Sweep::run` is allowed to return
    // `anyhow::Err`; we translate that into a `Failed` report so
    // the bookkeeping row carries the message and the walk
    // continues with the next sweep.
    let report = match sweep.run(ctx).await {
        Ok(r) => r,
        Err(e) => SweepReport::started(sweep.name(), started_at).finish_failed(
            ctx.now,
            0,
            0,
            e.to_string(),
        ),
    };

    // Stamp the terminal row.
    let finished_at = report.finished_at.unwrap_or(ctx.now);
    let payload = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
    // The legacy `records_dropped` column tracks the tier sweep's
    // drop count specifically. For the trait-level report, pull a
    // dropped count from `tier_transitions` when the sweep emits
    // one; otherwise 0.
    let records_dropped = report
        .tier_transitions
        .get("records_dropped")
        .copied()
        .unwrap_or(0);
    {
        let store = ctx.metadata.lock().await;
        store
            .finish_retention_sweep(
                &sweep_id,
                finished_at,
                report.rows_processed,
                records_dropped,
                &payload,
                report.status.as_str(),
            )
            .map_err(RegistryError::Finish)?;
    }

    Ok(report)
}

/// Convenience constructor — wires the seven canonical Phase13a
/// sweeps into a single registry. The supervisor uses this to
/// bootstrap a production-shaped instance in one call.
///
/// Each argument is the per-sweep `impl Sweep` already built by
/// the daemon; the registry just owns them. Callers that want a
/// subset build their own [`SweepRegistry::with`] manually.
pub fn canonical_registry(
    tier: Box<dyn Sweep>,
    parquet: Box<dyn Sweep>,
    cas: Box<dyn Sweep>,
    pii: Box<dyn Sweep>,
    meili: Box<dyn Sweep>,
    metadata_reap: Box<dyn Sweep>,
    consolidation: Box<dyn Sweep>,
) -> SweepRegistry {
    SweepRegistry::with(vec![
        tier,
        parquet,
        cas,
        pii,
        meili,
        metadata_reap,
        consolidation,
    ])
}

/// Wrap a `MetadataStore` into the `MetadataHandle` shape the
/// `SweepCtx` expects. Pure convenience for callers that just
/// opened the store.
pub fn into_handle(store: MetadataStore) -> Arc<Mutex<MetadataStore>> {
    Arc::new(Mutex::new(store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweep::report::SweepStatus;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use cortex_storage::MetadataStore;
    use cron::Schedule;
    use std::str::FromStr;

    /// Per-test sweep — name + a `should_fail` flag drive the
    /// outcome so we can verify success + failure rows side by
    /// side.
    struct ScriptedSweep {
        name: &'static str,
        should_fail: bool,
        rows: u64,
    }

    #[async_trait]
    impl Sweep for ScriptedSweep {
        fn name(&self) -> &'static str {
            self.name
        }
        fn schedule(&self) -> Schedule {
            Schedule::from_str("0 0 * * * * *").unwrap()
        }
        async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
            if self.should_fail {
                Err(anyhow::anyhow!("scripted failure"))
            } else {
                Ok(SweepReport::started(self.name, ctx.now).finish_success(ctx.now, self.rows, 0))
            }
        }
        fn report_view(&self, report: &SweepReport) -> crate::sweep::SweepReportView {
            report.view()
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx() -> (SweepCtx, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = into_handle(store);
        let ctx = SweepCtx::new(handle.clone(), "cortex.sweep.registry").with_now(fixed_now());
        (ctx, handle)
    }

    #[tokio::test]
    async fn registry_walks_every_sweep_and_writes_one_row_per_invocation() {
        let (ctx, handle) = make_ctx();
        let registry = SweepRegistry::with(vec![
            Box::new(ScriptedSweep {
                name: "sweep_a",
                should_fail: false,
                rows: 5,
            }),
            Box::new(ScriptedSweep {
                name: "sweep_b",
                should_fail: false,
                rows: 7,
            }),
        ]);
        let reports = registry.run_all(&ctx).await.unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].rows_processed, 5);
        assert_eq!(reports[1].rows_processed, 7);
        let rows = handle.lock().await.list_recent_sweeps(10).expect("list");
        assert_eq!(rows.len(), 2, "two retention_sweeps rows expected");
        assert!(rows.iter().all(|r| r.status == "success"));
    }

    #[tokio::test]
    async fn registry_records_failure_status_and_continues_walk() {
        let (ctx, handle) = make_ctx();
        let registry = SweepRegistry::with(vec![
            Box::new(ScriptedSweep {
                name: "sweep_ok",
                should_fail: false,
                rows: 1,
            }),
            Box::new(ScriptedSweep {
                name: "sweep_boom",
                should_fail: true,
                rows: 0,
            }),
        ]);
        let reports = registry.run_all(&ctx).await.unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].status, SweepStatus::Success);
        assert_eq!(reports[1].status, SweepStatus::Failed);
        let rows = handle.lock().await.list_recent_sweeps(10).expect("list");
        let statuses: Vec<_> = rows.iter().map(|r| r.status.clone()).collect();
        assert!(statuses.contains(&"failed".to_string()));
        assert!(statuses.contains(&"success".to_string()));
    }

    #[tokio::test]
    async fn registry_names_returns_declaration_order() {
        let registry = SweepRegistry::with(vec![
            Box::new(ScriptedSweep {
                name: "first",
                should_fail: false,
                rows: 0,
            }),
            Box::new(ScriptedSweep {
                name: "second",
                should_fail: false,
                rows: 0,
            }),
        ]);
        assert_eq!(registry.names(), vec!["first", "second"]);
    }
}
