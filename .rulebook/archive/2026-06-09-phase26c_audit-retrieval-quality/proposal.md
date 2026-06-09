# Proposal: phase26c_audit-retrieval-quality

## Why

Live audit on 2026-06-09 (docs/analysis/cortex/12-live-audit-2026-06-09.md) identified three
bugs that directly degrade retrieval quality and the pre-thinking pipeline:

- **Bug #8**: Vector similarity scores on live queries are 0.07–0.13 (effectively random).
  Root cause: the classifier runs in Static mode, so no `summary` field is ever populated.
  The embedder indexes raw JSON payloads (bash commands, tool outputs) instead of
  human-readable descriptions. Pre-thinking bundles surface raw JSON as context to the model.
- **Bug #9**: Pre-thinking p95 latency has grown from 104ms to 1,043ms over the last 20
  data points — 10× degradation, already past the 800ms circuit-breaker cap. No bundle
  caching exists: every pre-thinking call is a cache miss regardless of whether the same
  query shape was assembled seconds ago.
- **Bug #10**: All 170 decisions in the system carry `status: "proposed"` permanently.
  ADR-001 is explicitly marked "SUPERSEDED" in its text, but the Meilisearch document still
  shows "proposed". The bootstrap promotes decisions with a hardcoded default status and
  never re-parses the `**Status**:` line from the ADR file on incremental runs.

Together these bugs mean: (a) the vector lane is near-useless for natural-language queries,
(b) pre-thinking is degrading toward constant timeout, and (c) the decision registry does
not reflect ADR lifecycle reality.

## What Changes

### Bug #8 — Classifier summaries + re-embed stale events
- Decide and document the intended classifier mode (Static-with-summaries vs LLM) and
  configure it consistently across `.env` and docker-compose.
- Ensure Static mode generates at minimum a `summary` field derived from the event `kind`,
  `path`, and key payload fields (no LLM required — a deterministic template is enough).
- After the summary field is populated for new events, trigger a backfill of existing
  indexed events that have `summary: null`: re-classify via Static and re-embed.
- Metric to watch: vector query scores for "event classification system" against cortex
  repo should rise above 0.30 after re-embed.

### Bug #9 — Pre-thinking bundle cache
- `crates/cortex-pre-thinking/src/assembler.rs` (or equivalent): add an in-process LRU
  cache keyed on `sha256(query_text + scope + intent)` with a 60-second TTL.
- Cache hit must return in <5ms (memory read). Cache miss runs the full assembly.
- Expose `cache_hit_total` and `cache_miss_total` counters on the health endpoint.
- Target: p95 < 200ms under normal load (repeated or similar queries within a session).

### Bug #10 — Decision status re-parsed on incremental bootstrap
- `crates/cortex-cli/src/bootstrap/promoter.rs`: when promoting ADR files, parse the
  `**Status**: <value>` line from the markdown body and map it to the `status` field.
  Accepted values: `proposed`, `accepted`, `superseded`, `deprecated`.
- On incremental bootstrap, update the Meilisearch document if the status has changed
  since last promotion (keyed on content_hash or file mtime).

## Impact
- Affected specs: spec 05 (classifier), spec 06 (embedder), spec 09 (bootstrap), spec 12 (pre-thinking)
- Affected code: cortex-workers (classifier, embedder), cortex-pre-thinking, cortex-cli (bootstrap promoter)
- Breaking change: NO — status field update is a data quality fix; cache is transparent
- User benefit: semantic search returns relevant results; pre-thinking stays fast as index grows; ADR lifecycle visible in dashboard
