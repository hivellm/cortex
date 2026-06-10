# 26. Phantom-link verifier action policy: flag-first rollout, filter after measured confidence

**Status**: proposed
**Date**: 2026-06-10
**Related Tasks**: phase17_cdc-code-doc-correlation

## Context

Phase17 P3 verifies every cited (path, symbol) pair in retrieved snippets against the working tree (tree-sitter for Rust, string scan for Markdown). The open question was what to DO with an unverified snippet: drop it immediately (maximize precision, risk losing good snippets to resolver false negatives) or only annotate it (observe first, enforce later).

## Decision

VerifyConfig.action defaults to "flag": unverified snippets get verified=false + a SymbolVerdict (not_found/file_missing) attached but are NOT dropped; a `phantom_link_dropped` cortex_audit event records the per-query count either way. After ~2 weeks of live operation, once the audit stream shows the false-positive rate of the resolvers is acceptably low, the default flips to "filter" (drop unverified snippets) via CORTEX_VERIFY_ACTION — a config change, not a code change. Unsupported languages (no resolver) are never flagged or dropped: verdict=unsupported passes through untouched.

## Alternatives Considered

- filter from day one — rejected: resolver false negatives (macro-generated symbols, re-exports, non-ATX anchors) would silently drop valid snippets with no baseline to detect it
- flag forever (never filter) — rejected: leaves the model to interpret verified=false on its own; the point of P3 is to stop phantom links from reaching the model
- score penalty instead of drop — rejected: adds a tuning dimension (penalty weight) with no principled value before live data exists

## Consequences

Pros: zero retrieval regression during rollout; the audit event gives a measured phantom-link rate (gate: ≤1%, phase17 §3.10) before enforcement; flag→filter switch is a single env var. Cons: during the flag window the model still sees phantom links (only annotated); somebody must actually review the audit stream and flip the switch — the 2-week deadline lives in config docs (docs/specs/28), not in code.
