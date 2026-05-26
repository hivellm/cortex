# 15. ADR-014 — Dashboard handlers are pure readers; state lives in domain reports

**Status**: proposed
**Date**: 2026-05-25
**Related Tasks**: phase13f_dashboard-pure-reader-adr-014, phase13a_sweep-trait-adr-009, phase13b_envelope-producer-trait-adr-010, phase11v_retention-daemon-recovery

## Context

`cortex-api/src/dashboard.rs` carries per-sweep state-derivation logic — hardcoded `"never"` / `"n/a"` / `"unknown"` literals, fallback paths, ad-hoc projections. phase11v_retention-daemon-recovery (archived 2026-05-07) demonstrated the failure mode: handler-side state derivation is the canonical source of "frozen-as-contract bugs". The dashboard claimed `last run: never` while `cron_jobs` clearly held `last 2026-05-04T03:00:11`. Root cause: handler computed display string from a default rather than projecting a domain report row. Each new domain (Sweep, Consolidator, Coverage, Producer) reintroduces the same pattern; CI cannot grep for "the handler invented a string" because invented strings look like data. Source: `docs/analysis/rework/04-architecture.md` §A.6 (promoted from B.3 by opus5.7).

## Decision

Dashboard HTTP handlers in `crates/cortex-api/src/dashboard.rs` SHALL be pure readers. Each domain (Sweep, Consolidator, Coverage, Producer) MUST expose a serde-serialisable `Report` struct and a `report() -> Self::Report` method whose output lands in a SQLite report table. Handlers render only what the report row contains — no fallback strings, no derivations, no `unwrap_or("never")`. New per-domain types: `cortex_workers::sweep::report::SweepReportView`, `consolidator::report::ConsolidationReportView`, `coverage::report::CoverageReportView`, `producer::report::ProducerReportView`. CI gate: `rg '"never"|"n/a"|"unknown"' crates/cortex-api/src/dashboard.rs` MUST exit 1 (no matches); documented in `.github/workflows/ci.yml` and `docs/specs/21-dashboard.md` § Pure-reader contract. GUI (`gui/src/views/{Retention,Consolidations,Coverage,Producers}.tsx`) consumes Report JSON verbatim — local fallback branches deleted. Adding a new dashboard panel is bounded to ~50 lines of trait impl + ~100 lines of JSX.

## Alternatives Considered

- Leave handlers as-is and rely on code review to catch fallback strings — rejected, phase11v proved review misses them.
- Centralise fallbacks in a helper (`render_or_never(opt)`) — rejected, hides the contract violation; CI grep can't distinguish helper output from real data.
- Use OpenAPI codegen to enforce response shape — rejected, doesn't prevent the handler from inventing values to satisfy required fields.
- Push Report into the domain trait directly (no separate table) — rejected, couples HTTP latency to domain runtime; report tables let dashboards stay fast and observability stay decoupled.

## Consequences

Positive: "everything says never" becomes impossible by construction — the only way to render a string is to write a row, and the only way to write a row is to run the domain. CI grep gate makes regression mechanical. Report types are the contract — schema drift surfaces as serde failure at handler boundary, not as misleading UI text. New dashboards bound to trait+JSX; no handler logic to review. Negative: refactor touches every dashboard handler (4 endpoints + 1 new `producers_state` endpoint) and every existing GUI view. Breaking change to dashboard JSON shape — fields added; GUI ships in same PR to keep contract synchronous. §2.1 depends on phase13a Sweep trait (ADR-009, archived 2026-05-19); §2.4 depends on phase13b EnvelopeProducer trait (ADR-010, archived 2026-05-19). Neutral: handler test surface shrinks (pure projection of fixture rows); domain test surface grows (Report round-trip + serde stability).
