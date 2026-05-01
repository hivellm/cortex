# 04 — Five-axis relevance framework

Indexing the corpus produces recall. Recall ≠ relevance. To make
the agent loop *actually* prefer useful context over noise, the
query API needs five orthogonal axes, applied as filters or as RRF
score boosts.

This file defines each axis: what data feeds it, what schema fields
it requires, what query-API surface exposes it, what fusion change
applies it.

## Axis 1 — Temporal recency

**Why** — A turn from yesterday about Vectorizer's HNSW tuning is
worth twenty turns from a year ago about CSV parsing. Without
decay, RRF rewards "any historical match" equally, which floods
the bundle with stale context as the corpus grows.

| Aspect | Detail |
|---|---|
| Schema field | `Envelope.occurred_at` (already captured on every event) |
| Lane changes | none — the field round-trips through every backend |
| Query API | `Scope.since` (exists) for hard filter; new `Scope.recency_decay: Option<f32>` for soft boost |
| Fusion | per-hit score multiplied by `exp(-λ * days_old)` where λ defaults to `0.02 ` for `pre_change_context` (90-day half-life), `0.005` for `decision_lookup` (decisions are sticky), `0` for `law_check` (laws are evergreen) |
| Risk | over-decay can starve "we already solved this exact problem 6 months ago" — make λ configurable per intent and per query |

Implementation: phase 11i §3.1.

## Axis 2 — Project scope

**Why** — When editing Vectorizer, results from Vectorizer turns
beat results from Tml turns even at lower lexical similarity. The
same query in two repos should return two different bundles.

| Aspect | Detail |
|---|---|
| Schema field | `Context.repo` (canonical lowercase slug) |
| Lane changes | already implemented — per-repo collections + meili filter |
| Query API | `Scope.repo` (exists, mandatory since phase 6a); new optional `Scope.cross_repo_boost: f32` for permitting cross-repo hits at reduced weight |
| Fusion | when `cross_repo_boost > 0`, run a parallel lane against `cortex-{slug}-{family}` for every other indexed repo and merge with score × `cross_repo_boost` |
| Risk | none if the boost defaults to 0 — current behavior preserved |

Implementation: phase 11i §3.2.

## Axis 3 — Author + model attribution

**Why** — Three signals stack here:
1. Same model = same writing style → embeddings cluster tighter.
2. Opus turns tend to carry deeper rationale; haiku turns tend to be
   terse. Pre-thinking can ask for the model that matches its budget.
3. Distinguishing user prompts from agent prompts lets the bundle
   prioritise human-authored guidance over agent self-talk.

| Aspect | Detail |
|---|---|
| Schema field | `Envelope.model` (already captured in JSONL `message.model`); `Context.user`; `Envelope.tool` ("claude-code" / "openai-codex" / "bootstrap") |
| Lane changes | add `model` and `tool` to Meili `filterableAttributes` and Vectorizer payload; bumps `settings.v1.json` → `v2.json` |
| Query API | new `Scope.models: Vec<String>` (allow-list) and `Scope.tools: Vec<String>` |
| Fusion | when `Scope.models` is set, hard-filter; when unset, boost `claude-opus-*` ×1.2 over `claude-haiku-*` for `pre_change_context` (deep reasoning bias) but the inverse for `free_search` (haiku breadth) |
| Risk | model-name drift over time — add an alias table (`claude-opus-4-7` ≡ `claude-opus-4` family) so historical turns don't drop out when a new minor lands |

Implementation: phase 11i §3.3.

## Axis 4 — Session cohesion

**Why** — A turn cluster from a single 90-minute session is more
coherent context than the same number of turns scattered across a
year. Same-session turns share assumptions, files, and intent.

| Aspect | Detail |
|---|---|
| Schema field | `Envelope.session_id` (always captured) |
| Lane changes | add `session_id` to Meili filterable; Vectorizer payload already carries it |
| Query API | new `Scope.session_id: Option<String>` (active session) and `Scope.session_cohort: Vec<String>` (promote turns from these sessions) |
| Fusion | on hits where `session_id ∈ Scope.session_cohort`: ×1.5; on hits where `session_id == Scope.session_id` (current session): ×2.0; otherwise unchanged |
| Risk | session_id leaking into result text dilutes embedding quality — already redacted at the snippet level |

Implementation: phase 11i §3.4.

## Axis 5 — Outcome / success signal

**Why** — A turn that ended in `outcome: error` and was followed by
six retries is the *opposite* of what the agent should reuse. The
schema already carries outcome on `ToolCall`; we need to propagate
it up the Turn tree and expose it as a filter.

| Aspect | Detail |
|---|---|
| Schema field | `ToolCall.outcome: success | error | partial | blocked_by_law` (exists); new derived `Turn.outcome` computed by the classifier from child tool_call outcomes + assistant `stop_reason` |
| Lane changes | classifier worker computes `Turn.outcome`; emit it as a top-level Meili field; vectorizer payload carries it |
| Query API | new `Scope.outcomes: Vec<String>` (allow-list) and `Scope.exclude_outcomes` (deny-list); default = no filter |
| Fusion | `outcome=success` ×1.2; `outcome=error` ×0.5; `outcome=blocked_by_law` ×0.3 (policy decisions, not implementation patterns) |
| Risk | classifier mistakes propagate; mitigate by allowing the user to override via a future feedback loop (`/cortex feedback was-helpful=no`) |

Implementation: phase 11i §3.5 (later than the other four because
it depends on classifier work).

## Combined RRF formula

The RRF score for a hit becomes:

```
score = Σ_lanes (lane_weight × 1 / (k + rank))
        × recency_decay(occurred_at)
        × scope_multiplier(repo, repo_active)
        × model_multiplier(model, intent)
        × session_multiplier(session_id, scope.session_*)
        × outcome_multiplier(outcome, intent)
```

All multipliers default to 1.0; phase 11i ships a config file
`cortex-api/config/relevance.toml` so tuning is data-driven, not
recompile-driven.

## Surfacing in pre-thinking

The pre-thinking renderer already has sections for `Active laws`,
`Recent decisions`, `Similar past turns`, `Relevant snippets`,
`Graph neighbors`. Phase 11i adds two:

- **Past sessions** — top-3 sessions whose centroid embedding is
  closest to the current prompt; rendered as
  `<sessionId> · <date> · <one-line title from first user prompt> · <num turns>`.
  This gives the agent breadth without flooding the bundle.
- **Outcome marker** — every turn / decision rendered with a
  ✓ / ✗ / ⚠ glyph so the agent visually weighs success patterns.

Both stay within the 32 KiB byte-budget enforced by
[`crates/cortex-api/src/budget.rs`](../../../crates/cortex-api/src/budget.rs).

## Measurement plan

Phase 11i §4.6 ships a relevance-eval IT:

1. Hand-curated 30-question gold set under
   `crates/cortex-api/tests/fixtures/relevance-gold.json`.
2. Each question has 1-3 acceptable result IDs.
3. CI runs `cargo test --test relevance_eval_it` with
   `CORTEX_RELEVANCE_IT=1` and computes `mrr@10` and `ndcg@10`.
4. Gate: `mrr@10 >= 0.75`. Below threshold → CI fail → tune weights
   in `relevance.toml`, re-run.

This is the only way to keep relevance honest as the corpus grows.
Without a gold set we'd be hand-tuning blind.
