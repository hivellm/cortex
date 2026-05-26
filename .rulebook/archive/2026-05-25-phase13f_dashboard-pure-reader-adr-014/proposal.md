# Proposal: phase13f_dashboard-pure-reader-adr-014

Source: `docs/analysis/rework/04-architecture.md` §A.6 (promoted from B.3 by opus5.7).

## Why

Dashboard handlers in `cortex-api/src/dashboard.rs` carry per-sweep state-derivation logic (hardcoded `"never"` literals, fallback paths, ad-hoc projections). Phase11v_retention-daemon-recovery already showed that handler-side state is the canonical source of "frozen-as-contract bugs" — the dashboard claimed `last run never` while `cron_jobs` clearly had `last 2026-05-04T03:00:11`.

This task makes the contract structural: dashboard handlers render only what `Sweep::report() / Consolidator::report() / Coverage::report()` produce. CI grep gate makes hardcoded `never|n/a|unknown` impossible by construction.

## What Changes

- New ADR-014 — "Dashboard handlers are pure readers; state lives in domain reports".
- Each domain (Sweep, Consolidator, Coverage, Producer, Identity) exposes a `Report` struct + `report() -> Self::Report` method. Reports are JSON-serialisable.
- Dashboard handler reads only `Report` rows from SQLite. No string fallbacks, no derivations.
- CI grep gate: `rg '"never"|"n/a"|"unknown"' crates/cortex-api/src/dashboard.rs` MUST be empty.
- The GUI (`gui/src/views/*.tsx`) consumes the reports verbatim — handler-side derivation deleted.

## Impact

- Affected specs: `docs/specs/21-dashboard.md` § Pure-reader contract.
- Affected code: `crates/cortex-api/src/dashboard.rs` + `crates/cortex-api/src/dashboard/{consolidations,coverage,producers}.rs` (handlers), `crates/cortex-workers/src/{sweep,producer,coverage}/report.rs` + `crates/cortex-cli/src/bin/cortex-ops/identity_coverage.rs` (per-domain reports), `gui/src/views/{Retention,Consolidations,Coverage,Producers}.tsx`.
- Breaking change: dashboard JSON shape gains fields; GUI updated in same PR.
- User benefit: "everything says never" becomes impossible; new dashboards are 50 lines of trait impl + 100 lines of JSX.

## Scope revision (2026-05-25)

Discovery during §2 implementation:

1. `crates/cortex-api/src/dashboard.rs` is no longer a monolith — it is a thin coordinator (432 LOC) and the 19 handler submodules live in `crates/cortex-api/src/dashboard/{retention,consolidations,…}.rs` (5232 LOC).
2. The `coverage` domain referenced by the original spec does not exist under `crates/cortex-workers/`. Coverage logic lives in `crates/cortex-cli/src/bin/cortex-ops/identity_coverage.rs`. §2.3 therefore creates a thin `cortex-workers/src/coverage/mod.rs` shim that re-exports the cli-side `CoverageReportView`, rather than relocating logic.
3. Consolidations are surfaced through the `cortex-meili-consolidations` Meili index (envelope-projected), not a worker-side SQLite report table. §2.2 introduces `ConsolidationReportView` at the api/handler layer with a `ConsolidationReportSource` trait so the projection is testable without Meili. No new worker-side table.
4. `rg '"never"|"n/a"|"unknown"' crates/cortex-api/src/dashboard.rs crates/cortex-api/src/dashboard/` returns **0 matches** today — phase11v + phase13a already removed every offending literal. phase13f is now a **forward-looking lock** (typed views + CI gate) rather than a migration of live violations. §3.1 and §3.5 are therefore already satisfied; §4.1 expands its scope to cover the split submodules.

These changes preserve the ADR-014 contract (handlers are pure typed readers of domain Reports) while aligning the work with the actual codebase layout. The Why and the Decision are unchanged.
