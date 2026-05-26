# Proposal: phase13a_sweep-trait-adr-009

Source: `docs/analysis/rework/04-architecture.md` §A.1; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.1.

## Why

Seven retention/digest/pruning sweeps were each bolted on standalone, with their own cron, dashboard story, error path, and bookkeeping. Six retention gaps surfaced during phase11v_retention-daemon-recovery were the **same shape bug**: missing trait `Sweep`. Until the contract is uniform, every new sweep added in Phase B/C reintroduces the same defect class.

This is the load-bearing trait of the medium-rework plan and the gate for Phase B.

## What Changes

- New ADR-009 (`rulebook_decision_create`) — "`Sweep` trait as the single contract for retention/digest/pruning". Trade-offs documented per Tier 0.
- New trait `cortex_workers::sweep::Sweep`:
  ```rust
  #[async_trait]
  pub trait Sweep: Send + Sync {
      fn name(&self) -> &'static str;
      fn schedule(&self) -> Schedule;
      async fn run(&self, ctx: &SweepCtx) -> Result<SweepReport>;
      fn report_view(&self, report: &SweepReport) -> SweepReportView;
  }
  ```
- Migrate all 7 sweeps to `impl Sweep`: `tier_sweep`, `parquet_rollup`, `cas_vacuum`, `pii_enforce`, `meili_prune`, `metadata_reap`, `consolidation_prune`.
- The cron supervisor invokes `Sweep::run` and writes one `retention_sweeps` row per invocation. The dashboard reads only `retention_sweeps` (no per-sweep handler logic).
- Gate: zero string literals matching `never|n/a|unknown` in dashboard handlers post-migration.

## Impact

- Affected specs: `docs/specs/19-retention.md` § Sweep contract (new section).
- Affected code: `crates/cortex-workers/src/sweep/{trait.rs,ctx.rs,report.rs}` (new), `crates/cortex-workers/src/retention/*.rs` (each sweep migrates), `crates/cortex-workers/src/retention/scheduler.rs` (uniform invocation), `crates/cortex-api/src/dashboard.rs` (reads `retention_sweeps`).
- Breaking change: NO at the operator surface; INTERNAL refactor.
- User benefit: every new sweep landed via the trait works on day 1 with dashboard, metrics, and bookkeeping wired automatically.
