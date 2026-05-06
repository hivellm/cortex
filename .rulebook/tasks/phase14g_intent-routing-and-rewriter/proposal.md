# Proposal: phase14g_intent-routing-and-rewriter

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-002 + F-006 (MEDIUM each).

## Why

Two intent-related gaps:

1. `intent_select.rs::DEFAULT_RULES` uses 55 keyword rules in a flat linear scan, first-match-wins. Compound prompts mismatch silently (e.g. `"explain why did we pick hnsw"` → `explain` fires before `why did` → wrong intent). No `intent_mismatch_rate` metric exists.
2. Query rewriting in deterministic mode strips noun-phrases and loses intent signal (`"Refactor the HNSW configurator"` → `"HNSW configurator"`). Sonnet rewrite is opt-in via `CORTEX_QUERY_REWRITER=sonnet` but nobody enables it because there is no graceful fallback when Sonnet times out.

## What Changes

- Reorder + split `DEFAULT_RULES`: longer / more-specific rules (`why did we`, `decided to pick`) precede single-word triggers (`explain`, `change`).
- Add metric `cortex_pre_thinking_intent_mismatch_total{from, to}` driven by feedback and implicit_score (low score on intent X promotes "user probably meant Y" diagnosis).
- Cascade rewriter: try Sonnet (with response cache); on timeout / 5xx, fall through to deterministic; on Sonnet success, cache for 24h.
- New `cortex-ops intent-stats [--since <duration>]` reports per-intent mismatch rate, used to drive future rule tuning.

## Impact

- Affected specs: `docs/specs/12-pre-thinking-injection.md` § Intent selector + § Query rewriter cascade.
- Affected code: `crates/cortex-pre-thinking/src/{intent_select.rs,rewriter.rs,metrics.rs}`, `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO.
- User benefit: compound prompts route to the correct intent; query rewriting preserves intent signal without breaking when Sonnet is unavailable.
