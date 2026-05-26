# 10. ADR-009 — Sweep trait as single contract for retention/digest/pruning sweeps

**Status**: proposed
**Date**: 2026-05-19
**Related Tasks**: phase13a_sweep-trait-adr-009, phase11v_retention-daemon-recovery

## Context

Seven retention/digest/pruning sweeps (`tier_sweep`, `parquet_rollup`, `cas_vacuum`, `pii_enforce`, `meili_prune`, `metadata_reap`, `consolidation_prune`) were each bolted on standalone, with their own cron, dashboard story, error path, and bookkeeping. The 2026-05-05 retention daemon learning explicitly observed: *"each sweep was implemented as a self-contained CLI with its own dashboard story, then bolted on to the cron scheduler later. No shared 'I am running as a sweep' wrapper exists. Six independent gaps all live in the same gap: 'what does it mean to BE a sweep' was never codified."* The six retention gaps surfaced in `phase11v_retention-daemon-recovery` were the **same shape bug**: missing trait. Dashboard hardcoded `next_run: "never"` for 7 of 9 sweeps. Until the contract is uniform, every new sweep added in Phase B/C reintroduces this defect class.

Reference: `docs/analysis/rework/04-architecture.md` §A.1; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.1. This is the load-bearing trait of the medium-rework plan and the gate for Phase B.

## Decision

Introduce `cortex_workers::sweep::Sweep` as the single trait every retention/digest/pruning job implements.

```rust
#[async_trait]
pub trait Sweep: Send + Sync {
    fn name(&self) -> &'static str;
    fn schedule(&self) -> Schedule;
    async fn run(&self, ctx: &SweepCtx) -> Result<SweepReport>;
    fn report_view(&self, report: &SweepReport) -> SweepReportView;
}
```

Supporting types:
- `SweepCtx` — handles for `MetadataStore`, `Vectorizer`, `Meili`, `Nexus`, worker config, logger.
- `SweepReport { name, started_at, finished_at, status, bytes_reclaimed, rows_processed, tier_transitions, error_message }`.
- `SweepReportView` — dashboard projection.

The `RetentionScheduler` invokes `Sweep::run` uniformly via a `Vec<Box<dyn Sweep>>` registry. The cron supervisor writes one `retention_sweeps` row per invocation (start + finish). The dashboard reads only `retention_sweeps`; per-sweep ad-hoc handler code is deleted. CI grep gate: zero matches for `"never"|"n/a"|"unknown"` in `cortex-api/src/dashboard.rs` post-migration.

All 7 sweeps migrate to `impl Sweep`: `tier_sweep`, `parquet_rollup`, `cas_vacuum`, `pii_enforce`, `meili_prune`, `metadata_reap`, `consolidation_prune`.

Status: `proposed`. Promote to `accepted` once §3.1 lands (first migrated sweep proves the contract holds in production code).

## Alternatives Considered

- Keep ad-hoc per-sweep implementations and patch the dashboard to read each sweep's individual state surface — rejected: this is the status quo that produced the 6-gap retention daemon failure; patches don't fix shape bugs.
- Adopt a generic 'Job' trait covering sweeps + consolidation + bootstrap producers in one — rejected: conflates two contracts (Sweep = idempotent cleanup with bookkeeping; EnvelopeProducer = streaming source with checkpoint). ADR-010 will introduce EnvelopeProducer separately for clean separation of concerns.
- Replace cron sweeps with event-driven triggers (e.g. high-watermark on retention metrics) — rejected: out of scope for Phase A; requires Synap event schema changes. Can be added on top of Sweep later as a different scheduler impl.
- Use trait objects keyed by a `SweepKind` enum rather than `&'static str` name — rejected: enum growth requires recompile of cortex-workers for every new sweep, defeating the 'zero-cost addition' goal.

## Consequences

**Cost** (one-week structural refactor):
- 7 sweeps must each migrate to the trait; risk of regressions during conversion mitigated by per-sweep integration test (§3.8) asserting one `retention_sweeps` row per invocation.
- New trait surface (`sweep/mod.rs`, `trait.rs`, `ctx.rs`, `report.rs`) plus scheduler rewrite. Internal-only — no operator-facing breaking change.
- Adds one SQLite write per sweep invocation (`retention_sweeps` row). Negligible (sweeps run on minute/hourly cadence, not per-event).

**Gain** (uniform observability + dashboard correctness):
- Dashboard becomes a pure reader over `retention_sweeps`; the `"never"` class of bug is impossible by construction.
- Every new sweep landed via the trait works on day 1 with dashboard, metrics, and bookkeeping wired automatically. Phase B (consolidator-as-Sweep, pruning-as-Sweep) and Phase C (Codex/Cursor/Gemini adapters that include sweep behaviour) inherit the surface for free.
- Per-sweep error handling becomes uniform — failures land in `SweepReport.error_message`, not lost in worker logs.

**Risk** (medium): the chosen abstraction may need revising if a future sweep doesn't fit (e.g. one that needs streaming progress, not just start/finish). Mitigation: trait is internal; revision via superseding ADR is cheap. Trait deliberately omits a `progress(&self)` hook until a concrete use case appears (YAGNI).

**Reversibility**: reversible. If the trait proves wrong, sweeps can return to ad-hoc impls without a data migration (the `retention_sweeps` table is append-only audit and remains valid).
