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
- Affected code: `crates/cortex-api/src/dashboard.rs`, `crates/cortex-workers/src/*/report.rs` (per-domain reports), `gui/src/views/{Retention,Consolidations,Coverage,Producer}.tsx`.
- Breaking change: dashboard JSON shape gains fields; GUI updated in same PR.
- User benefit: "everything says never" becomes impossible; new dashboards are 50 lines of trait impl + 100 lines of JSX.
