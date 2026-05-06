# Proposal: phase13e_cortex-config-crate-adr-016

Source: `docs/analysis/rework/opus5.7/02-blind-spots.md` §1 (config sprawl); `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.5.

## Why

`grep -r "CORTEX_[A-Z_]+" crates/` returns **344 matches across 50+ files**. Top offenders: `cortex-api/src/main.rs` (82 refs), `cortex-api/src/http.rs` (32), `cortex-api/src/config_audit.rs` (26). Many env vars overlap (3 affect retention, 2 affect scope resolution). Without a single typed `Config` struct, every Phase B subsystem rewrite reintroduces ad-hoc env-var reads and the 60-day rework cycle repeats.

This task lands inside Phase A so Phase B subsystems bind to `Config::*`, not `std::env::var`.

## What Changes

- New ADR-016 — "Schema-evolution policy + typed Config crate".
- New crate `crates/cortex-config/` exposing `pub struct Config { ... }` (serde-versioned via `#[serde(tag = "schema_version")]`).
- Config sources, resolved in order: CLI flag → env var → `cortex.toml` → built-in default.
- `cortex-config::load()` is the only public entry point. `std::env::var(...)` is forbidden in `cortex-api`, `cortex-workers`, `cortex-cli` (CI grep gate).
- New `cortex-ops doctor config-audit` reports any ad-hoc env-var read remaining in the workspace; exit 2 if non-zero.
- Existing `CORTEX_*` env names preserved as deserialiser aliases (zero operator-script breakage).

## Impact

- Affected specs: `docs/specs/00-architecture.md` § Configuration (new); ADR-016.
- Affected code: `crates/cortex-config/` (new), every `std::env::var("CORTEX_*")` call site (~344) migrated to `cfg.<field>`.
- Breaking change: NO at the operator surface (env-var aliases preserve backwards compat).
- User benefit: every config knob has one type, one default, one documentation row, and is testable.
