# Proposal: phase10h_query_scope_filters

## Why

`/v1/query` advertises a `scope` object with `repo`, `files`,
`since`, `topics`. Only `repo` actually filters today:

- `scope.since` — accepted by the deserializer, dropped by the
  orchestrator.
- `scope.topics` — dropped silently. Yet the classifier already
  tags every event with topics (`memory:281`, `code:169`,
  `governance:86`, `law:85`, …). The dashboard's
  `/v1/dashboard/classifications` exposes the full taxonomy, but
  no query path can consume it.
- `scope.files` — partially honored: it's used as a path prefix
  filter on the keyword lane only; the vector + graph lanes
  ignore it.

For the agent this is the difference between "give me decisions
about Meili from the last 30 days" (currently impossible) and
"give me everything that mentions Meili since the dawn of the
project" (current behavior).

## What Changes

1. `since` — translates to `occurred_at >= since` on every lane
   (Meili filter, Vectorizer payload filter, Nexus Cypher
   `WHERE n.occurred_at >= $since`).
2. `topics` — every event carries `payload.topic` (or
   `extras.classifier.top_topic`); add an OR filter on those
   columns. Multi-topic ORs.
3. `files` — extend to the Vectorizer + Nexus lanes by
   prefix-matching `payload.path`.
4. Audit envelope (`cortex.events.query_audit`) records the
   resolved scope so the dashboard can show "you searched for X
   in topic=law since 2026-01-01" alongside the result count.
5. The pre-thinking pipeline forwards `topics` derived from
   recent files (file extension → topic guess) so the bundle
   automatically scopes to the relevant corpus.

## Impact

- Affected specs: `docs/specs/11-query-api.md` §scope (full
  contract), `docs/specs/12-pre-thinking-injection.md` §scope
  inference.
- Affected code: `crates/cortex-api/src/types.rs` (scope
  resolver), `crates/cortex-api/src/meili_lane.rs`,
  `crates/cortex-api/src/vectorizer_lane.rs`,
  `crates/cortex-api/src/nexus_graph_lane.rs`,
  `crates/cortex-pre-thinking/src/scope.rs`.
- Breaking change: NO. Filters that were silently dropped now
  apply.
- User benefit: queries actually scope; pre-thinking bundles
  carry the right corpus instead of "everything ever".
