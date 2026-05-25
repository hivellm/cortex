## 1. ADR-014
- [x] 1.1 `rulebook_decision_create` ADR-014 — "Dashboard handlers pure readers; state in domain reports". Status `proposed`. (decision id 15, slug `adr-014-dashboard-handlers-are-pure-readers-state-lives-in-domain-reports`, 2026-05-25)
- [x] 1.2 Trade-off: refactor every handler; gain is structural correctness (CI gate makes "never" literals impossible). (captured in ADR-014 Consequences)

## 2. Per-domain Report types

> **Scope revision (2026-05-25)**: phase11v + phase13a already removed every `"never" / "n/a" / "unknown"` literal from `crates/cortex-api/src/dashboard.rs` and the 18 submodules under `crates/cortex-api/src/dashboard/`. phase13f now LOCKS the contract structurally — produce typed `*ReportView` projections so future handlers cannot reintroduce drift. Spec drift fixes: dashboard is split into submodules (not a monolith); coverage domain lives in `cortex-cli/src/bin/cortex-ops/identity_coverage.rs`, not in `cortex-workers`; consolidator surfaces consolidations via the `cortex-meili-consolidations` index (envelope-projected), not via a worker-side SQLite report table.

- [x] 2.1 `cortex-workers/src/sweep/report.rs::SweepReportView` (lands together with Phase 13a Sweep trait). (already shipped in phase13a; type at `sweep/report.rs:191-211`, `SweepReport::view()` at `:172-186`, serde round-trip test at `:328-335`)
- [x] 2.2 `cortex-api/src/dashboard/consolidations.rs::ConsolidationReportView` — typed projection of `ConsolidationRow + ConsolidationDetail` so the handler returns a `ReportView`, not an ad-hoc `Vec<ConsolidationRow>`. Source stays Meili (envelope index). Add `consolidator::report::ConsolidationReportSource` trait so the projection is testable without Meili. (`ConsolidationReportView` + `ConsolidationFilter` + `ConsolidationReportSource` at `dashboard/consolidations.rs:70-129`; `ConsolidationRow`/`Detail` gained `Deserialize`+`PartialEq`+`Eq`; 5 unit tests pass; trait housed at api layer per scope-revision note — projection layer, not worker crate)
- [x] 2.3 `cortex-cli/src/bin/cortex-ops/identity_coverage.rs::CoverageReportView` + `cortex-workers/src/coverage/mod.rs` shim exposing the type to `cortex-api` (re-export, no logic move). Dashboard handler reads from the existing identity-coverage SQLite table via the shim. (Types moved into `cortex-workers/src/coverage/mod.rs` — `cortex-cli` already depends on `cortex-workers` so the workers crate is the natural home; CLI binary now imports `cortex_workers::coverage::{BackendCoverage, CoverageReport}`. `CoverageReportView` + `BackendCoverageEntry` + `CoverageReport::view()` + `CoverageReportSource` trait at `coverage/mod.rs:25-141`; 6 workers tests + 5 unchanged CLI tests pass)
- [x] 2.4 `cortex-workers/src/producer/report.rs::ProducerReportView` — move `ProducerReport` from `producer/checkpoint.rs:66-79` into `producer/report.rs`; add `ProducerReportView` + `ProducerReport::view()` mirroring the Sweep pattern. Keep re-export from `producer/mod.rs` so call sites do not change. (`producer/report.rs:24-99` + `ProducerReport::view()` at `:46-65`; re-export at `producer/mod.rs:60`; 4 unit tests in `report::tests` pass)
- [x] 2.5 Round-trip serde tests for every Report type (Sweep already covered by phase13a — `sweep/report.rs:328-335`; add for §2.2, §2.3, §2.4). (Sweep ✓ phase13a; Producer ✓ 4 tests in `producer/report.rs::tests`; Consolidation ✓ 5 tests in `dashboard/consolidations.rs::tests`; Coverage ✓ 6 tests in `coverage/mod.rs::tests`)

## 3. Handler rewire
- [x] 3.1 `dashboard::retention::*` reads `retention_sweeps` directly and projects `SweepReportView`. (already wired by phase13a — see `crates/cortex-api/src/dashboard/retention.rs:71-95` and the `last_run`/`last_status` projection sourced from `retention_sweeps`)
- [ ] 3.2 `dashboard::consolidations::*` returns `ConsolidationReportView` (handler signature change; envelope source unchanged).
- [ ] 3.3 `dashboard::coverage::*` (new submodule `crates/cortex-api/src/dashboard/coverage.rs`) returns `CoverageReportView`. Reads the identity-coverage SQLite table via the §2.3 shim.
- [ ] 3.4 `dashboard::producers::*` (new submodule `crates/cortex-api/src/dashboard/producers.rs`) reads from `producer_checkpoints` joined with the most recent `ProducerReport` row per `(producer_name, scope)` and returns `ProducerReportView`.
- [x] 3.5 Hardcoded literals (`"never"`, `"n/a"`, `"unknown"`) deleted from every handler. (verified 2026-05-25 — `rg '"never"\|"n/a"\|"unknown"' crates/cortex-api/src/dashboard.rs crates/cortex-api/src/dashboard/` returns 0 matches)

## 4. CI gate
- [ ] 4.1 Add a CI step: `rg '"never"\|"n/a"\|"unknown"' crates/cortex-api/src/dashboard.rs crates/cortex-api/src/dashboard/` MUST exit 1 (no matches). Scope expanded to cover the split submodules.
- [ ] 4.2 Document the gate in `.github/workflows/ci.yml` and `docs/specs/21-dashboard.md` § Pure-reader contract.

## 5. GUI sync
- [ ] 5.1 `gui/src/views/Retention.tsx` consumes `SweepReportView` verbatim. Local fallback paths deleted. (verify; phase13a may have landed this already)
- [ ] 5.2 `gui/src/views/Consolidations.tsx` consumes `ConsolidationReportView`.
- [ ] 5.3 `gui/src/views/Coverage.tsx` consumes `CoverageReportView` (new view file if absent).
- [ ] 5.4 New `gui/src/views/Producers.tsx` consumes `ProducerReportView`.
- [ ] 5.5 Snapshot tests updated for every view.

## 6. Tail (mandatory)
- [ ] 6.1 Update `docs/specs/21-dashboard.md` § Pure-reader contract + `CHANGELOG.md`.
- [ ] 6.2 Tests: §2.5 × 4 + handler ITs against fixture rows + §4.1 CI gate + §5.5 snapshots.
- [ ] 6.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace && pnpm -C gui test` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
