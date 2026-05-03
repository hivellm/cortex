# Consolidation pre-flight estimate — 2026-05-03

> Phase11q §1 — operator gate. Generated via `cortex-consolidator estimate --json`. **No Anthropic API calls fired.**

## Headline

| Pass | Model | Input tokens | Output tokens | Cost USD |
|---|---|---:|---:|---:|
| session-grain | Haiku 4.5 (Shallow) | 20,251,777 | 0 | **$0.02** |
| topic-grain | Haiku 4.5 (Shallow) | 5,062,944 | 512 | **$0.00** |
| decision-trace | Opus 4.7 (Deep) | 300,000 | 102,400 | **$12.18** |
| **TOTAL** | | | | **$12.20** |

## Per-repo input volume

The estimator walked every `cortex-{slug}-turns` Meili index and aggregated `body` bytes per repo:

| Repo | Turn+AgentCall envelopes | Body bytes | Est. input tokens |
|---|---:|---:|---:|
| `vectorizer` | 6,823 | 30.6 MB | 7,652,894 |
| `nexus` | 5,223 | 20.3 MB | 5,080,757 |
| `tml` | 9,788 | 12.4 MB | 3,092,723 |
| `rulebook` | 3,661 | 3.6 MB | 911,249 |
| `synap` | 1,950 | 3.7 MB | 920,772 |
| `cortex` | 1,410 | 3.3 MB | 830,885 |
| `expert` | 45 | 3.1 MB | 775,139 |
| (12 others) | 2,440 | 7.6 MB | 1,887,358 |
| **Total** | **31,485** | **81.0 MB** | **20,251,777** |

(token estimate uses the conservative 4 chars/token ratio.)

## Discovery gap

`sessions` count = **0** for every repo. The pre-phase11i envelopes do NOT carry `session_id` at the top level of the Meili document; spec-11 lane projection contract added the field but legacy documents were indexed under the old schema. The session-grain consolidation pass needs a session-id discovery layer either of:

1. The phase11k §1 settings v5 PATCH fanned out across every per-repo index (which post-phase11k re-indexed envelopes carry `turn_id` / `session_id` — but the OLD ones still don't). A backfill pass would be needed to surface session_id on pre-phase11k documents.
2. Discovery from a different source — Synap stream replay + groupby `session_id` on the live envelope tier. More expensive but works against the current corpus shape.

The session-grain estimate ($0.02) above assumes the tokens are processed once total — under-estimates if discovery groups them into N sessions and produces N consolidations. Realistic upper bound after a backfill: still under $0.10 because Haiku 4.5 is exceptionally cheap.

## Recommendation

The total of **$12.20 USD** is operator-friendly — the dominant cost is Opus DecisionTrace ($12.18 for 100 ADRs at $15/M input + $75/M output). Two paths:

1. **Approve full $12.20 budget.** Run all three grains. Decision-trace gets Opus depth (best recall on supersession / rationale chains).
2. **Approve $0.10 budget.** Run session + topic only at Shallow/Haiku; defer decision-trace until the budget is approved separately. Trade-off: decisions get the same Shallow Haiku treatment as turns, lower recall on the supersession chain.

## Blocked items before the actual passes can fire

The `cortex-consolidator` binary today ships only the `estimate` subcommand. Triggering the actual passes (the lib's `Orchestrator::run_session` / `run_topic` / `run_decision_trace`) requires:

1. **Discovery layer wiring.** The lib API takes pre-built `SessionInput` / `TopicCluster` / `DecisionTraceInput` structs; the binary needs to hydrate them from Meili + Synap + Nexus before invoking the orchestrator.
2. **Anthropic API key configuration.** `ANTHROPIC_API_KEY` env in the runtime; today's estimator path does NOT need it.
3. **Cost ceiling configuration.** `Orchestrator::with_budget` enforces a per-call cap; the binary needs to thread an operator-supplied budget through to the orchestrator.

These three gaps justify carving the actual run into a follow-up task (`phase11r_corpus_consolidation_apply`) once the operator approves the USD budget. Phase11q ships the operator gate.

## Re-run

```sh
CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:17004 \
CORTEX_FULLTEXT_MEILI_API_KEY=$MEILI_MASTER_KEY \
  cortex-consolidator estimate --json > docs/cortex/<date>-consolidation-estimate.json
```

Optional `--repo <slug>` to scope to a single repo.
