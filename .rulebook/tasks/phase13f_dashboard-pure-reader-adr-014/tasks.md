## 1. ADR-014
- [ ] 1.1 `rulebook_decision_create` ADR-014 — "Dashboard handlers pure readers; state in domain reports". Status `proposed`.
- [ ] 1.2 Trade-off: refactor every handler; gain is structural correctness (CI gate makes "never" literals impossible).

## 2. Per-domain Report types
- [ ] 2.1 `cortex-workers/src/sweep/report.rs::SweepReportView` (lands together with Phase 13a Sweep trait).
- [ ] 2.2 `cortex-workers/src/consolidator/report.rs::ConsolidationReportView`.
- [ ] 2.3 `cortex-workers/src/coverage/report.rs::CoverageReportView`.
- [ ] 2.4 `cortex-workers/src/producer/report.rs::ProducerReportView` (lands together with Phase 13b EnvelopeProducer trait).
- [ ] 2.5 Round-trip serde tests for every Report type.

## 3. Handler rewire
- [ ] 3.1 `dashboard::retention_state` reads `retention_sweeps` directly and projects `SweepReportView`.
- [ ] 3.2 `dashboard::consolidations_state` reads from the consolidator report table.
- [ ] 3.3 `dashboard::coverage_state` reads from the coverage report table.
- [ ] 3.4 `dashboard::producers_state` (new endpoint) reads from `producer_checkpoints` joined with the producer report.
- [ ] 3.5 Hardcoded literals (`"never"`, `"n/a"`, `"unknown"`) deleted from every handler.

## 4. CI gate
- [ ] 4.1 Add a CI step: `rg '"never"|"n/a"|"unknown"' crates/cortex-api/src/dashboard.rs` MUST exit 1 (no matches).
- [ ] 4.2 Document the gate in `.github/workflows/ci.yml` and `docs/specs/21-dashboard.md`.

## 5. GUI sync
- [ ] 5.1 `gui/src/views/Retention.tsx` consumes `SweepReportView` verbatim. Local fallback paths deleted.
- [ ] 5.2 `gui/src/views/Consolidations.tsx` consumes `ConsolidationReportView`.
- [ ] 5.3 `gui/src/views/Coverage.tsx` consumes `CoverageReportView`.
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
