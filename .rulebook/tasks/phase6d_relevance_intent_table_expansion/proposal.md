# Proposal: phase6d_relevance_intent_table_expansion

## Why

The intent selector at `crates/cortex-pre-thinking/src/intent_select.rs:21-112` covers four intents with narrow keyword sets: 5 keywords for `decision_lookup`, 6 for `similar_problems`, 4 for `law_check`, 5 for `pre_change_context`. Common operator phrasings — *"how does X work"*, *"what is X"*, *"explain X"*, *"show me where X"*, *"find usages of X"* — match none of these and fall through to `pre_change_context`. That plan fans out to three lanes plus four overlays (decisions, laws, similar turns, graph), running `edge_artifact_touched_neighbours` on the graph lane.

For navigational / explanatory prompts, this is the right *retrieval* but the wrong *post-fusion*: the decisions overlay activates on a "how does X work" query, surfacing irrelevant decisions to the bundle and burning the trim ladder's budget. The user's experience is "Cortex always returns decision noise even when I'm just trying to read code".

R2 step 6 in the relevance plan, closes F-006.

Source: `docs/analysis/relevance/01-findings.md` §F-006; `docs/analysis/relevance/02-execution-plan.md` §R2.

## What Changes

### New `Intent::Explain`
A purpose-built navigational/explanatory intent. Plan: vector + keyword fan-out on `code` + `docs` topics only; **no decision / law / similar-turn overlays**. The graph lane runs `edge_artifact_definitions` (introduced by `phase4c`) instead of `edge_artifact_touched_neighbours` so symbol-level questions resolve to definitions.

### Trigger keywords (case-insensitive substring match, in priority order)
- `Intent::Explain` — `how does`, `what is`, `what's`, `explain`, `show me`, `where is`, `where does`, `find usages`, `find references`, `look up`, `definition of`
- `Intent::DecisionLookup` — keep today's keywords; add `why did we pick`, `why do we use`, `who decided`, `history of`
- `Intent::SimilarProblems` — keep today's keywords; add `have we seen`, `did we hit`
- `Intent::LawCheck` — keep today's keywords; add `is this allowed`, `am i allowed`, `would this violate`

### Plan table
Extend `crates/cortex-pre-thinking/src/strategies.rs` (or wherever `pre_change_context`'s plan is built) with an `explain_plan` factory that returns:
- Vector lane: `topics = ["code", "docs"]`, `limit = 8`
- Keyword lane: same topics + same limit
- Graph lane: `edge_artifact_definitions` overlay only (when `phase4c` has shipped; until then, no graph leg)
- Overlays: `snippets` only — no `decisions`, no `violations`, no `similar_turns`

### Backwards-compatible defaults
The intent selector still falls through to `pre_change_context` when no keyword matches — that remains the safe default for prompts that don't fit any of the five buckets. The change widens explicit coverage; it does not narrow the fallback.

### Observability
Stamp the matched intent + the keyword that triggered it on the audit envelope (`intent`, `intent_trigger`) so phase6e's harness can attribute regressions to intent-routing changes.

## Impact

- Affected specs: [`docs/specs/12-pre-thinking.md`](../../../docs/specs/12-pre-thinking.md) (intent table + plan factories).
- Affected code: `crates/cortex-pre-thinking/src/intent_select.rs` (keyword tables + new `Intent::Explain` variant); `crates/cortex-pre-thinking/src/strategies.rs` (or equivalent — the `explain_plan` factory); `crates/cortex-api/src/types.rs` (extend the `Intent` enum if it lives there); `crates/cortex-api/src/audit.rs` (stamp `intent_trigger`).
- Breaking change: NO — the new variant is additive; existing callers passing `Intent::PreChangeContext` round-trip unchanged.
- Depends on: `phase4c` for the `edge_artifact_definitions` graph leg. Until `phase4c` lands, the `Explain` plan ships without the graph leg and the proposal is honest about that — no fake graph results.
- User benefit: navigational and explanatory prompts get a focused retrieval bundle (snippets only, no decision / law noise), saving budget for the actual code/docs hits the user wanted.
