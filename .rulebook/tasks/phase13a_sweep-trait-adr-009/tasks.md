## 1. ADR-009
- [ ] 1.1 `rulebook_decision_create` ADR-009 — "Sweep trait as single contract for retention/digest/pruning". Status `proposed`.
- [ ] 1.2 Trade-off section: cost is one-week refactor of 7 sweeps; gain is dashboard correctness + uniform metrics + zero-cost addition of new sweeps.
- [ ] 1.3 Promote to `accepted` once §3.1 lands.

## 2. Trait + supporting types
- [ ] 2.1 New module `crates/cortex-workers/src/sweep/`. Files: `mod.rs`, `trait.rs`, `ctx.rs`, `report.rs`.
- [ ] 2.2 `Sweep` trait per the proposal signature. `SweepCtx` carries handles to `MetadataStore`, `Vectorizer`, `Meili`, `Nexus`, the worker config, and a logger.
- [ ] 2.3 `SweepReport { name, started_at, finished_at, status, bytes_reclaimed, rows_processed, tier_transitions, error_message }`. `SweepReportView` is the dashboard projection.
- [ ] 2.4 7 unit tests on the trait shapes (round-trip serde, status-transitions, error-message truncation).

## 3. Migrate the 7 sweeps
- [ ] 3.1 `tier_sweep` — wrap existing logic in `impl Sweep for TierSweep`.
- [ ] 3.2 `parquet_rollup` — same.
- [ ] 3.3 `cas_vacuum` — same.
- [ ] 3.4 `pii_enforce` — same.
- [ ] 3.5 `meili_prune` — same.
- [ ] 3.6 `metadata_reap` — same.
- [ ] 3.7 `consolidation_prune` — same.
- [ ] 3.8 Each migration carries a per-sweep IT that runs the trait and asserts a `retention_sweeps` row materialised.

## 4. Scheduler + dashboard rewire
- [ ] 4.1 `RetentionScheduler` invokes `Sweep::run` uniformly via a `Vec<Box<dyn Sweep>>` registry. Per-sweep ad-hoc handler code deleted.
- [ ] 4.2 Cron supervisor writes one `retention_sweeps` row per `Sweep::run` invocation (start + finish). Dashboard reads only this table.
- [ ] 4.3 `crates/cortex-api/src/dashboard.rs::retention_state` projection rewritten to read `retention_sweeps`. Hardcoded handler-side state literals deleted. CI grep gate: zero matches for `"never"|"n/a"|"unknown"` in the dashboard handler module.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/19-retention.md` + `CHANGELOG.md` Changed.
- [ ] 5.2 Tests: §2.4 + §3.8 × 7 + §4.3 grep CI step.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
