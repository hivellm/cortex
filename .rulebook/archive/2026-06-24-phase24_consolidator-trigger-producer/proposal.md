# phase24 — Consolidator trigger producer (live wiring)

## Why

The consolidator daemon (`cortex-consolidator`) is running and healthy
but has **dispatched=0** for the entire stack lifetime — it consumes the
Synap stream `cortex.consolidator.triggers` and nothing publishes to it.
Confirmed via container logs (`status: exited cleanly — dispatched=0
failed=0 idle_polls=640361`) and a code sweep: the only publisher of
`TRIGGER_STREAM` is `scripts/smoke-consolidator-daemon.sh`. The bin's own
module doc states "Live producer wiring against Synap + Vectorizer +
Nexus lands alongside phase11j §3 routing; until then the run-* / nightly
subcommands…" — so the live trigger producer was always deferred.

Consequence on retrieval quality (measured 2026-06-06 while validating
the dense/FastEmbed update):

- Meili indexes `cortex_consolidations` and `cortex_topic_cards` do not
  exist (HTTP 404) — they are only created on first write, and no
  consolidation/topic-card has ever been produced.
- `cortex_similar_sessions` returns `{hits:[], total:0}` for every query
  (it vector-searches `cortex-<repo>-consolidations`).
- `cortex_topic_search` returns `{hits:[], estimated_total_hits:0}`.
- The entire `cortex_consolidations_*` MCP tool family is dead.

So 3+ MCP retrieval surfaces are non-functional purely because no trigger
producer exists. The consolidator code, grains (session / topic /
decision-trace), and Meili/Vectorizer sinks are all implemented and
unit-tested — only the event producer that fires triggers is missing.

## What Changes

Add a live trigger producer so the consolidator actually runs:

1. A producer that watches the ingestion/event stream and publishes a
   `cortex.consolidator.triggers` envelope when each grain's condition
   fires (session-end, topic event-count threshold, decision-landed).
2. Wire the producer into the running stack — classifier-worker already
   consumes the event stream and is the natural host.
3. Backfill the empty indexes from existing history (reuse `run-session`
   / `run-topic` / `nightly`), gated behind the existing `estimate` cost
   preview so live Anthropic spend is authorised.
4. Verify the three dead MCP surfaces return data:
   `cortex_similar_sessions`, `cortex_topic_search`,
   `cortex_consolidations_recent`.

## Impact
- Affected specs: docs/specs/27-consolidation.md
- Affected code: crates/cortex-workers/src/consolidator/,
  crates/cortex-workers/src/bin/cortex-consolidator.rs,
  crates/cortex-workers/src/classifier_worker/ (producer host),
  docker-compose.yml
- Breaking change: NO
- User benefit: re-activates similar_sessions / topic_search /
  consolidations_* retrieval surfaces that are currently dead.

## Source

Discovered during phase22 post-backend-update validation (dense/FastEmbed
retrieval audit, 2026-06-06).
