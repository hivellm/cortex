# Proposal: phase10a_query_lane_wiring

## Why

The audit on 2026-04-29 (50-query relevance harness + direct probes
against the live `cortex-api`) revealed three intents that ALWAYS
return an empty body even though the dashboard lanes carry the data
they should be searching:

- `intent=law_check` → `results: {}`. Dashboard reports 37 laws +
  121 violations.
- `intent=decision_lookup` → empty unless the query happens to
  match a decision *title* by keyword. Dashboard reports 26
  decisions whose `rationale` field carries the full ADR body.
- `intent=similar_problems` → `results: {}`. Dashboard reports
  1672 turns + 181 conversations.

Net effect: the relevance harness scored 0% on `law_check`,
0% on `similar_problems`, and 50% recall@10 on `decision_lookup`
only because half the queries hit a decision title. The Sonnet
pre-thinking bundle is therefore useless for any prompt that asks
about laws, prior turns, or decisions whose ADR body (not title)
carries the answer.

## What Changes

1. Wire the **decisions lane** into the
   `decision_lookup` orchestrator: query Vectorizer
   `cortex.decision.fp32` + Meili `cortex_decisions` + Nexus
   `:Decision` graph and fuse via the existing RRF blend.
2. Wire the **laws + violations lane** into `law_check`: query
   Nexus `:Law`/`:LawViolation` plus Meili `cortex_laws` /
   `cortex_violations`. Always include the relevant law's body
   in the snippet payload so the agent can quote it back.
3. Wire the **turns lane** into `similar_problems`: query
   Vectorizer `cortex.turn.fp32`/`pq` + Meili `cortex_turns`,
   include the resolved `session_id` + `occurred_at` so callers
   can deep-link.
4. Update `crates/cortex-api/src/orchestrator.rs` so each intent
   advertises which lanes it consumes (today the table only
   covers `pre_change_context` and `free_search`).
5. Add a relevance harness fixture (`tests/relevance/queries.toml`
   already has the labels) and assert non-zero recall@10 for the
   three intents post-fix.

## Impact

- Affected specs: `docs/specs/11-query-api.md` §lanes,
  `docs/specs/13-laws-dsl.md` §retrieval,
  `docs/specs/16-dashboard.md` §integration.
- Affected code: `crates/cortex-api/src/orchestrator.rs`,
  `crates/cortex-api/src/strategies.rs`,
  `crates/cortex-api/src/meili_lane.rs`,
  `crates/cortex-api/src/vectorizer_lane.rs`,
  `crates/cortex-api/src/nexus_graph_lane.rs`,
  `crates/cortex-api/src/fusion.rs`.
- Breaking change: NO. Pure additive lane plumbing.
- User benefit: closes the 50% recall floor on the relevance
  harness and unblocks the pre-thinking bundle for laws,
  decisions, and prior-turn lookups.
