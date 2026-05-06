# Proposal: phase15d_laws-dsl-v1-shipped

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-008 + F-009 (HIGH).

## Why

Laws DSL (spec 13) and Governance Engine (spec 14) have been "Drafted" since the spec index was created. The blocking-law enforcement (PreToolUse) exists only as a mock in spec 10. Until real laws ship, Cortex governance is advisory rather than enforced. minmax2.7 names this the largest unaddressed risk in the system.

## What Changes

- Ship Laws DSL v1: a typed YAML schema for `.rulebook/laws/*.yml` with `id`, `severity`, `trigger`, `rule`, `rationale` fields.
- New crate `crates/cortex-laws/` exposing `Law`, `LawRegistry`, `evaluate(action, ctx) -> Vec<Verdict>`.
- Governance Engine v1: `cortex-api /v1/laws/check` endpoint that the adapter PreToolUse hook calls before each tool invocation.
- 6 starter laws: `LAW-CORTEX-001` (task sequence), `LAW-007` (no destructive git without auth), `LAW-008` (no `--no-verify`), `LAW-009` (sequential editing), `LAW-010` (research before implement), `LAW-011` (fail-twice escalate).
- `cortex laws lint laws/*.yml` validates schema; CI gate.

## Impact

- Affected specs: `docs/specs/13-laws-dsl.md` (graduate from Drafted → v1), `docs/specs/14-governance-engine.md` (same).
- Affected code: `crates/cortex-laws/` (new), `crates/cortex-api/src/laws.rs` (new endpoint), `crates/cortex-adapter-claude-code/src/dispatcher.rs` (call /v1/laws/check on PreToolUse).
- Breaking change: NO at the wire; laws denied actions return a structured 403.
- User benefit: governance becomes enforceable; Tier 0 / Tier 1 rules detected at runtime, not retroactively.
