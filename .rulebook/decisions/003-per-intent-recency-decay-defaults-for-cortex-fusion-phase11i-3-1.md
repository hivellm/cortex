# 3. Per-intent recency-decay λ defaults for Cortex fusion (phase11i §3.1)

**Status**: proposed
**Date**: 2026-05-01
**Related Tasks**: phase11i_claude_archive_indexer_and_relevance

## Context

Phase11i §3.1 introduced an exponential recency-decay multiplier on fused scores: `score *= exp(-λ · days_old)`. The fusion path needs a per-intent λ default so an operator who does NOT supply `Scope.recency_decay` still gets reasonable behaviour. The proposal called for "newer hits should bubble up but a six-month-old ADR must still surface when it's the right answer", which rules out aggressive decay across the board.

Three signals competed:
1. **Newer turns matter more.** When a user asks `pre_change_context` for "how did we wire X yesterday", a result from six months ago should NOT outrank yesterday's work.
2. **Decisions are sticky.** ADRs accepted years ago can still be load-bearing. A blanket `pre_change_context`-shaped decay would bury them.
3. **Laws are evergreen.** A `LAW-CORTEX-001` definition does not get more or less true with time. Recency must not influence ranking at all.

We needed a small, defensible table (5–6 intents, no per-repo tuning) and a way to override per-request via `Scope.recency_decay`.

## Decision

Adopt the following per-intent λ table (units: 1/day; multiplier = `exp(-λ · days_old)`):

| Intent              | λ      | Half-life | Rationale                                              |
| ------------------- | ------ | --------- | ------------------------------------------------------ |
| `pre_change_context`| 0.02   | ~35 days  | Bias toward recent work without burying month-old code |
| `similar_problems`  | 0.02   | ~35 days  | Same — recent debugging more representative            |
| `free_search`       | 0.02   | ~35 days  | Default-ish; matches the pre_change_context profile    |
| `explain`           | 0.02   | ~35 days  | Same                                                   |
| `decision_lookup`   | 0.005  | ~140 days | Decisions are sticky; mild bonus only                  |
| `law_check`         | 0.0    | n/a       | Evergreen — recency must not move the ranking          |

Implementation:
- Constants in `crates/cortex-api/src/fusion.rs`: `DEFAULT_RECENCY_LAMBDA_PRE_CHANGE = 0.02`, `DEFAULT_RECENCY_LAMBDA_DECISION = 0.005`, `DEFAULT_RECENCY_LAMBDA_LAW = 0.0`.
- `FusionConfig::default_recency_lambda_for_intent(intent: &str) -> f32` resolves the table; unknown intents fall back to `0.0` (safe legacy behaviour).
- `Scope.recency_decay: Option<f32>` lets a caller override per-request; `Some(0.0)` disables decay for that one query without disabling the global default.
- `relevance.toml` `[recency]` block ships the same defaults as a SIGHUP-reloadable knob set (phase11i §3.6), so operators can rebalance without a redeploy.
- Hits with `ts == 0` (no timestamp from the upstream) skip the multiplier entirely so a metadata regression cannot accidentally bury them.
- Future-dated hits (clock skew / replay) clamp to `days_old = max(0, …)` so the multiplier stays ≤ 1 and a positive exponent never inflates a score.

## Alternatives Considered

- Single global λ across all intents — rejected because it forces the decision_lookup vs pre_change_context tradeoff onto operators (and the proposal explicitly called out that decisions are sticky).
- Linear decay (1 - days/N) — rejected because the cliff at the cutoff makes ranking unstable around the boundary; exponential decay degrades smoothly.
- Step function (full weight < 30 d, half weight 30-180 d, zero > 180 d) — rejected for the same reason; also harder to tune incrementally.
- Per-repo λ — rejected as premature; the gold-set IT (phase11i §4.5) does not split queries by repo and we have no signal that any repo behaves differently along the recency axis. Easy to add later if needed.
- Reciprocal decay (1 / (1 + λ · days)) — comparable behaviour but slower to compute (no `exp` shortcut on the hot path) and the half-life translation is less intuitive for operator-facing docs.

## Consequences

**Positive:**
- Recent `pre_change_context` queries naturally surface yesterday's work without exiling six-month-old ADRs to the tail (the `decision_lookup` λ is 4x lower, so an ADR that's 35 days old gets a 0.84 multiplier vs a turn at the same age getting 0.50).
- `law_check` ranking stays deterministic across calendar drift — useful for the rulebook's audit pipeline that diffs query results across runs.
- The TOML knob set (phase11i §3.6) lets operators tune without recompilation; the `relevance-tuning.md` handbook documents when to bump λ vs when to re-index instead.
- The fusion gold-set IT (phase11i §4.5) gates regressions: any future tuning that drops MRR@10 below 0.75 fails the IT and blocks merge.

**Negative / tradeoffs:**
- The constants are a starting point, not a proof. The first month of operator use will likely surface intents that need different λ — particularly `similar_problems` (might need higher λ if the corpus is dominated by recent debugging cycles) and `free_search` (might need lower λ if it's used for "where is X defined" queries that should treat code timestamps as irrelevant).
- The 0.005 / 0.02 / 0.0 spread means the `recency_decay` field on `Scope` does meaningful work only for `decision_lookup` callers who want to opt INTO recency. Most callers will rely on the default.
- Exponential decay is multiplicative across the fused score, so it stacks with the cross-repo boost (§3.2) and the outcome multiplier (§3.5). The cumulative multiplier on a 90-day-old foreign-repo error turn is `exp(-1.8) × 0.5 (cross-repo) × 0.5 (error) ≈ 0.04` — that's an 8x compression below the in-repo recent-success baseline. Operators should be aware of the stacking before tuning any single knob.

**Reassessment trigger:** Re-evaluate after the §4.5 gold-set IT runs against a 6-month-old corpus snapshot. If MRR@10 for `decision_lookup` drops below 0.85 OR `pre_change_context` drops below 0.80 against the current 0.75 floor, re-tune the table.
