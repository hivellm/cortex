# Proposal: phase14d_gui-rust-contract-test

Source: `docs/analysis/rework/opus5.7/03-recommendation.md` Phase B.5.

## Why

`gui/src/lib/api.ts` is hand-maintained TypeScript that mirrors Rust route signatures. Drift is silent: the GUI compiles fine when a Rust route changes shape, then crashes at runtime against the live API. The dashboard `Consolidations` view shipping in the current working tree is one such instance — it was written against the proposed JSON shape but never validated against the live handler.

## What Changes

- Generate `gui/src/lib/api.ts` from Rust types via a build-time script (e.g. `ts-rs` derive on `cortex-api` types).
- Alternative: a contract test that diffs the GUI types against Rust route signatures and fails CI when they diverge.
- One CI step: `pnpm -C gui run check-contract` runs the diff and fails if any route's request/response shape does not match.

## Impact

- Affected specs: `docs/specs/21-dashboard.md` § Contract test.
- Affected code: `crates/cortex-api/src/types.rs` (add `#[derive(TS)]` or equivalent), `gui/src/lib/api.ts` (generated), new `scripts/generate-gui-types.{sh,ps1}`.
- Breaking change: NO.
- User benefit: dashboard never silently breaks against API changes; new endpoints are 1 derive + 1 build step away from being callable.
