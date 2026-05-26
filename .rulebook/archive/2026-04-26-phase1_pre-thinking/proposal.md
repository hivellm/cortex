# Proposal: phase1_pre-thinking

## Why

The query API returns raw bundles; the model needs a compact, deterministic, section-ordered block with a hard byte budget. This module wraps `cortex-api /v1/query` with adapter-side heuristics: scope derivation, intent selection, budget-aware formatting, and fail-open semantics. It is the last mile of the "before-change context" user story (US-01) and the place we protect the hook's latency budget.

## What Changes

- `cortex-adapters/common/pre_thinking/` module (shared across adapters, per spec 17 §common).
- `scope_derive` mapping `(user_prompt, cwd, recent_files)` → `QueryRequest.scope`.
- Rule-based intent selection (keyword table).
- Deterministic Markdown formatter with fixed section order (laws → decisions → similar turns → snippets → optional graph).
- Budget clipper with documented 6-step trim order; laws are always preserved.
- `query_id` embedded in a leading comment for audit correlation.

## Impact

- **Affected specs:** [`docs/specs/12-pre-thinking-injection.md`](../../../docs/specs/12-pre-thinking-injection.md).
- **Affected code:** new `cortex-adapters/common/pre_thinking.rs`, consumed by `cortex-adapters/claude-code/`.
- **Breaking change:** NO — greenfield.
- **User benefit:** Claude Code sessions receive focused, deterministic pre-thinking bundles that respect the 32 KB + 600 ms budgets.

## Source

`docs/specs/12-pre-thinking-injection.md` · depends on specs 10 + 11 · PRD FR-12.
