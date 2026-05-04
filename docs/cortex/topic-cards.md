# Topic cards — operator handbook

Phase 11r adds a **living-synthesis tier** on top of consolidations. A topic card is a slug-keyed prose synthesis the orchestrator **rewrites in place** as new evidence accumulates. One card per `(topic_slug, repo_scope)`; the deterministic id (`topic-{24-hex}` derived from `sha256(slug ⊕ repo_scope)`) means re-emitting the same card lands on the same node and bumps `revision`.

This doc is the operator runbook. For the conceptual layering see [`docs/architecture.md` §6.0a](../architecture.md). For the typed contract see `crates/cortex-core/src/events.rs::TopicCardPayload` + `crates/cortex-core/schemas/kinds/topic_card.schema.json`. For the choice to layer a separate kind on top of consolidations rather than mutating the consolidation kind see [ADR-006](../../.rulebook/decisions/006-topic-card-as-living-synthesis-vs-consolidation-as-snapshot.md).

## 1. Trigger heuristics — when does a rewrite fire?

The orchestrator's `Trigger::evaluate(card, new_event, distance, now)` returns `Rewrite` when **any** of:

| # | Trigger              | Condition                                                                                          | Constant                              |
|---|----------------------|----------------------------------------------------------------------------------------------------|---------------------------------------|
| 1 | **Burst**            | `events_since_last_rev ≥ 8`                                                                        | `TRIGGER_EVENTS_THRESHOLD = 8`        |
| 2 | **High-impact proximity** | A new event lands within `distance < 0.30` AND is a `Decision`, `LawViolation`, or high-impact-outcome event | `TRIGGER_DISTANCE_THRESHOLD = 0.30`   |
| 3 | **Stale + new evidence**  | `synthesis_age_d ≥ 14` AND ≥ 1 new evidence is cited                                          | `TRIGGER_AGE_DAYS = 14`               |

When **none** fire, the trigger emits `Hold { reason: HoldReason }` where `HoldReason ∈ {Cooldown, LowImpact, NotRelevant}`. Holds are silent — they don't produce envelopes; the orchestrator just proceeds to the next event.

The thresholds are **heuristic-tuned** for the live HiveLLM corpus (see phase 11r `cortex-topic-cards estimate` runs in [`docs/cortex/2026-05-03-consolidation-estimate.md`](2026-05-03-consolidation-estimate.md) for the calibration data). Operators can override per-card by passing `force_deep` to the synthesiser (see §4 below).

## 2. Contradiction detector classes

On every rewrite, the contradiction scanner runs three detector classes against the union of `existing_evidence ∪ new_evidence`:

| Class                       | Triggers when…                                                                                                    | Status default |
|-----------------------------|-------------------------------------------------------------------------------------------------------------------|----------------|
| `DecisionSupersession`      | A pair where one decision's `supersedes` matches another's `decision_id`.                                         | `Open`         |
| `LawViolationMismatch`      | A `LawViolation` cites a different `law_version` than the currently-active `Law` definition with the same `law_id`. | `Open`         |
| `OutcomeDivergence`         | Two `Consolidation` events have overlapping `temporal_span` and different `outcome_majority`.                     | `Open`         |

Each emitted `Contradiction` carries `surfaced_at_rev` (the revision the rewrite landed under) and `status = Open` by default. Operators can flip status to `Reconciled` (the issue was addressed downstream) or `Deprecated` (the contradiction no longer applies, e.g. a superseded decision was withdrawn) by hand-editing the card via `cortex_synthesize --persist=true` after curating the `contradictions[]` array.

The contradiction scanner is **heuristic** — false positives are expected for novel patterns (e.g. a Decision with the same id but different `decision_id` due to a manual id rewrite). Detectors **never block a rewrite**; they surface contradictions for the agent to read.

## 3. Staleness contract

The pre-thinking renderer's section-ordering matrix is **staleness-aware**:

```
fresh card        →  laws → topic_cards → consolidations → decisions → similar_turns → past_sessions → snippets
stale card        →  laws → consolidations → topic_cards (with advisory) → decisions → similar_turns → past_sessions → snippets
```

A card is **stale** when **either**:

- `confidence < 0.6` (the `TOPIC_CARD_CONFIDENCE_FLOOR` constant in `formatter.rs`), OR
- `synthesis_age_d > 30` AND `events_since_last_rev > 0` (synthesis is old AND new evidence has accumulated since).

When stale, the renderer:

1. Demotes the topic card section behind consolidations (so consolidations land first in the agent's prompt).
2. Stamps a `> stale-topic-card: <reason>` advisory line directly under the `## Topic card` heading.

A stale card is not skipped — it is rendered with a warning. The agent gets to read both the (potentially out-of-date) synthesis and the fresh consolidation lane, and the advisory tells it to weight the consolidation higher.

## 4. Operator runbook

### Force-rewrite a card

Bypass the trigger heuristics for one card (e.g. you just curated the contradictions array and want a fresh synthesis):

```bash
cortex-topic-cards rewrite auth-rewrite --repo cortex
```

Add `--deep` to escalate to Opus instead of Haiku (cost: ~4 000 cents per rewrite vs. ~100 for Haiku):

```bash
cortex-topic-cards rewrite auth-rewrite --repo cortex --deep
```

### Replay since a timestamp

Re-run the trigger evaluation for every event since `<ts>` — useful for catching cards that should have rewritten but were held due to a bug or a stale heuristic:

```bash
cortex-topic-cards replay --since 2026-04-01T00:00:00Z
```

### Nightly dry-run

Print the cost cap + remaining budget without touching any backend (handy in CI / smoke):

```bash
cortex-topic-cards nightly --dry-run
```

### Force a fresh synthesis through the MCP tool

When you want the synthesis but **not** the envelope (e.g. preview before committing):

```jsonc
{
  "tool": "cortex_synthesize",
  "input": {
    "query": "auth rewrite",
    "scope": { "repo": "cortex" },
    "force": true,
    "persist": false
  }
}
```

`persist: true` emits a `Kind::TopicCard` envelope normally; `persist: false` returns the payload to the caller without indexing. Either way the cost ledger records the burn, and exhausting the configured cap returns `BudgetExhausted { used_cents, cap_cents }`.

## 5. MCP tool reference

| Tool                       | Purpose                                                                  | See                              |
|----------------------------|--------------------------------------------------------------------------|----------------------------------|
| `cortex_topic_get`         | Fetch the top topic card for a slug or query (confidence ≥ 0.6 floor on the search lane). | spec 11r §4.1                    |
| `cortex_topic_drill`       | Drill into one dimension: `evidence` / `contradictions` / `history` / `open_questions` / `related`. | spec 11r §4.2                    |
| `cortex_topic_neighbors`   | Walk the topic-card subgraph one to N hops (depth default = 2, clip = 64 nodes). | spec 11r §4.3                    |
| `cortex_topic_diff`        | Compute the diff between revision `since_rev` and the current head (unified diff + set diffs). | spec 11r §4.4                    |
| `cortex_synthesize`        | Operator escape hatch — run the synthesiser ad-hoc with `force` + `persist` flags. | spec 11r §4.5                    |

All five tools emit a `topic_card_mcp_audit` envelope through the configured `AuditPublisher` so the dashboard surfaces every drill / get / synthesize call alongside the existing `cortex_query` audit lane. Failure paths (`ScopeRepoRequired`, `Invalid`, `BudgetExhausted`, `Backend`) record the rejection envelope so the dashboard's misconfig detection has a complete trail.

## 6. Cost & budget

The synthesiser composition reuses the consolidator's `Summariser` trait. The orchestrator escalates Haiku → Opus when **any** of:

- `force_deep` is set (caller explicit).
- The existing card has ≥ 3 open contradictions.
- The existing evidence already trips a `decision_supersession` per the contradiction scanner.

| Model     | Per-rewrite cost (cents, ballpark) |
|-----------|------------------------------------|
| Haiku     | ~100                               |
| Opus      | ~4 000                             |

The cost ledger records every rewrite under the `topic_card` grain bucket; `/v1/health/coverage` surfaces the burn alongside the consolidator's `session` / `topic` / `decision_trace` grains. The budget gate is per-budget-window (default monthly cap `100 000` cents = $1 000); exhausting the cap returns `BudgetExhausted` and the rewrite is dropped without emitting an envelope.
