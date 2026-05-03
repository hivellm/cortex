# Consolidation tier — operator handbook

Operators running the phase11j consolidation tier (`cortex-consolidator` crate + cortex-side routing) work this handbook. Every cost / fidelity / template knob the consolidator exposes lives below.

## What this tier does

`cortex-consolidator` reads raw envelopes the rest of the pipeline already captured, summarises them with an Anthropic model (Haiku 4.5 by default; Opus 4.7 for `decision_trace`), and emits one `Kind::Consolidation` envelope per produced summary. Three grains:

| Grain            | Trigger                                            | Default model | Depth   |
|------------------|----------------------------------------------------|---------------|---------|
| `session`        | `run-session <session_id>`                         | Haiku 4.5     | Shallow |
| `topic`          | HDBSCAN cluster of ≥ 3 sessions on a single repo   | Haiku 4.5     | Shallow |
| `decision_trace` | `Kind::Decision` envelope + parent chain (≤ 16 hops) | Opus 4.7    | Deep    |

The pre-thinking renderer's `## Consolidated context` section reads consolidations through the standard query lanes; nothing in the rest of the pipeline knows the difference between a raw event and a consolidation except the new `consolidation:<grain>` symbol and the dedicated index / collection (`cortex_consolidations`, `cortex.consolidation.fp32` + `.pq`).

## Cost guardrails

The consolidator ships a `CostBudget` defaulting to **$1 000 / month** (100 000 cents). Every producer invocation calls `gate_budget` first; the orchestrator returns `SummariserError::CostCeiling` rather than overshoot. CLI override:

```
cortex-consolidator --monthly-cents-cap 50000  # halve the budget
cortex-consolidator nightly --dry-run          # show remaining cents, no API call
```

Per-call estimates the gate uses:

| Grain            | Conservative cost estimate | Why that number |
|------------------|----------------------------|-----------------|
| `session`        | 100 cents (Haiku, ~30 turns) | 95th-percentile session × Haiku price |
| `topic`          | 100 cents | Same Haiku-on-cluster shape |
| `decision_trace` | 4 000 cents (Opus, 16-hop chain) | 99th-percentile chain × Opus price |

`CostLedger` records the **realised** cost per grain; surfaced through the `/v1/health/coverage` block (lands once §5.7 unblocks via phase11o). Until then, the orchestrator's `metrics()` snapshot is the operator-side read: per-grain `consolidations` count + total `cost_cents` + `mean_cost_cents()`.

### When the budget gate trips

Symptoms: producer returns `Err(ProducerError::Summariser(SummariserError::CostCeiling))`; nightly CLI exits non-zero with `apply-settings-only: budget exhausted`.

Fix order:

1. **Confirm the realised spend matches the gate's estimate.** Realised typically lands at ~30-60% of estimate. If realised is at the cap, the gate is doing its job.
2. **Bump the cap, not the estimate.** `--monthly-cents-cap` lets you raise the ceiling without touching the per-call estimate (which is intentionally conservative — the gate's job is to refuse before a charge that breaks the operator's budget).
3. **If realised is consistently far below the estimate**, lower the conservative estimate in `orchestrator::ProducerSelection::estimated_cents` so the gate stops blocking under-budget producers.
4. **Never reset the ledger mid-month.** `CostLedger::reset()` exists for tests and for the start-of-month rollover; calling it from a daemon pretends the prior spend never happened and defeats the cap.

## Fidelity threshold tuning

The fidelity IT (`consolidation_fidelity_it`, lands once §6.2 unblocks) samples raw → consolidation pairs and asks Haiku 4.5 to score whether every `takeaways[]` entry is supported by at least one `source_event_id`. Acceptance gate per the proposal:

| Depth   | Threshold |
|---------|-----------|
| Shallow | ≥ 90 % takeaways supported |
| Deep    | ≥ 98 % takeaways supported |

When the IT fails:

1. **Read the per-takeaway score breakdown the IT logs.** A single low-confidence row tells you which takeaway slipped.
2. **Check the source set.** If the consolidator was given < 5 source events, fidelity collapses fast. Bump the producer's minimum input set (currently no floor; consider one for low-recall sessions).
3. **Inspect the prompt template.** Each grain's template lives under `crates/cortex-consolidator/templates/{session,topic,decision_trace}.md`. The output-contract block at the bottom pins the `{title, summary_markdown, takeaways}` JSON shape — a regression here surfaces as parse errors before fidelity drops.
4. **For Opus regressions**, check the model id pin in `summariser::SummariserKind::model_id`. Opus 4.7 is the contract; downgrading to 4.6 will miss the Deep-depth fidelity gate.

## Prompt template iteration

Templates are plain `{{slot}}`-substitution Markdown files. To iterate:

1. Edit `crates/cortex-consolidator/templates/<grain>.md`.
2. Run `cargo test -p cortex-consolidator templates::tests` — the contract tests pin slot names + the takeaways count per grain (3 / 5 / 7 for Session / Topic / DecisionTrace).
3. Run a single producer end-to-end against a `CannedSummariser` to compare output JSON shape: `cargo test -p cortex-consolidator end_to_end_it`.
4. Only after both pass, run a real Anthropic call: `ANTHROPIC_API_KEY=... cortex-consolidator run-session 01EXAMPLESESSION` and inspect the produced envelope.

The output-contract block at the bottom of each template is load-bearing — `producer::*::parse_model_response` looks for the `{title, summary_markdown, takeaways}` JSON object (with optional ```json``` code fences). Drop the contract block and parsing breaks.

### Slot reference

| Slot                     | Available in   | Notes                                        |
|--------------------------|----------------|----------------------------------------------|
| `{{session_id}}`         | session        | The triggering session id.                   |
| `{{repo}}`               | session, topic | Repo slug (lowercase).                       |
| `{{started_at}}`         | session        | RFC-3339 of the earliest envelope.           |
| `{{ended_at}}`           | session        | RFC-3339 of the latest envelope.             |
| `{{turn_count}}`         | session        | Turn count in the source set.                |
| `{{outcome_summary}}`    | session        | Pre-computed `success/error/partial` totals. |
| `{{source_turns}}`       | session        | Newline-joined turn previews.                |
| `{{topic_label}}`        | topic          | HDBSCAN cluster label.                       |
| `{{cluster_size}}`       | topic          | Session count in the cluster.                |
| `{{temporal_span}}`      | topic          | `<earliest> .. <latest>`.                    |
| `{{outcome_distribution}}` | topic        | Aggregated outcome counts.                   |
| `{{source_sessions}}`    | topic          | One-line digest per session.                 |
| `{{decision_id}}`        | decision_trace | The triggering decision id.                  |
| `{{decision_title}}`     | decision_trace | Decision title (≤ 80 chars).                 |
| `{{decision_status}}`    | decision_trace | `accepted` / `superseded` / etc.             |
| `{{decided_at}}`         | decision_trace | RFC-3339 of the decision envelope.           |
| `{{chain_hops}}`         | decision_trace | Length of the parent chain.                  |
| `{{source_chain}}`       | decision_trace | Oldest-first chain envelopes.                |

Untouched slots stay literal in the rendered output — a typo surfaces in the model's response, not silently in production.

## Dashboard / GUI

The Memory view's filter strip carries the consolidation lane once the dashboard picks it up:

- Filter chip **Consolidations** scopes the lane to `kind = consolidation`.
- Sub-filters **Grain** (`session` / `topic` / `decision_trace`), **Depth** (`Shallow` / `Deep`), and **Model** drill in further. All four chip values come from the v4 Meili settings (`ext.consolidation.{grain, depth, model, consolidation_id}` are filterable).
- Sort by **Date** (default) or **Consolidation id** for deterministic re-grouping.

When the chip is empty: the consolidator either has not run for this project, or the cost gate stopped it before the first emission. Check `cortex-consolidator nightly --dry-run` for the latter.

## Cross-references

- Spec 12 (`docs/specs/12-pre-thinking-injection.md`) — `## Consolidated context` section format + §4.3 fallback rule.
- Spec 16 (`docs/specs/16-dashboard.md`) — Memory view layout (consolidations chip lands once the GUI picks it up).
- Spec 19 (`docs/specs/19-retention.md`) — pruning daemon contract; lives blocked on phase11o.
- ADR-005 (`.rulebook/decisions/005-consolidation-grain-choice-session-topic-decisiontrace.md`) — rationale for the three-grain split.
