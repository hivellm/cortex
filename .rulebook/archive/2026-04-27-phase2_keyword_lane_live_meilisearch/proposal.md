# Proposal: phase2_keyword_lane_live_meilisearch

## Why

The `cortex-api` orchestrator runs against `MemoryKeywordLane` ([crates/cortex-api/src/main.rs:41](../../../crates/cortex-api/src/main.rs#L41)), which is a test double. Its `search` method ignores `req.query` entirely — see [crates/cortex-api/src/lanes.rs:218-232](../../../crates/cortex-api/src/lanes.rs#L218-L232) — and returns whatever the seeder loaded. The `archive_loader` doc-comment confirms: *"The MemoryKeywordLane returns all seeded hits regardless of `query`, so the same hit set surfaces under every alias."*

Empirical proof, captured 2026-04-27 against the running daemon:

| Query | Top-5 returned |
|---|---|
| `pre-thinking pipeline` | canonical envelope smoke / V3 ping / watcher pinguim / smoke após hooks.json / manual probe pos restart |
| `asdfqwerty` | identical |
| `rust` | identical |
| `LAW-007` | identical |

A "search" that returns the same five rows for every input is not search. The pre-thinking bundle therefore looks identical for every prompt — confirmed by three consecutive `UserPromptSubmit` hooks in this very session, which each delivered the same five smoke-test envelopes.

A `cortex-fulltext-worker` daemon is already running (PID 71852 confirmed via tasklist on 2026-04-27) and `cortex-fulltext` crate exists. The wiring from the worker's index to the `cortex-api` orchestrator is the missing piece.

## What Changes

- New crate (or expanded `cortex-fulltext`) exposes a `MeiliKeywordLane` implementing the `KeywordLane` trait by calling Meilisearch's `/multi-search` endpoint against the indexes seeded by `cortex-fulltext-worker` (per spec-08).
- `cortex-api/src/main.rs` builds the lane from `MEILI_URL` (or `CORTEX_MEILI_URL`), with `meilisearch_sdk::Client`. When the env is missing or the server is unreachable at boot, fall back to `MemoryKeywordLane` and warn — never crash.
- The lane translates the orchestrator's `KeywordRequest { index, query, filter, k }` to the Meili `/indexes/{index}/search` body with `q`, `filter`, `limit=k`, `attributesToHighlight`, and consumes the `hits[].score` / Meili `_rankingScore` to populate `LaneHit.score`.
- The lane scores and source labels become real (not the `1/(60+rank)` positional artifacts surfaced today). The `source` field on `LaneHit` correctly reflects `"keyword"` instead of the misleading `"vector"` label that today appears on keyword-derived hits.
- `archive_loader.rs`'s seeding-into-MemoryKeywordLane stays as a fallback for offline dev, but the live lane is the default when Meili is up.

## Impact

- Affected specs: spec-08 (fulltext indexer — query-time read path is missing), spec-11 (lane wiring).
- Affected code:
  - new: `crates/cortex-fulltext/src/keyword_lane.rs` (or a new `cortex-fulltext-client` crate)
  - `crates/cortex-api/src/main.rs` (boot wiring, env discovery, fallback)
  - `crates/cortex-api/src/lanes.rs` (no trait change; only a new `impl KeywordLane`)
  - tests in `cortex-fulltext` against a `wiremock`-or-Meili-instance fixture
- Breaking change: NO (the trait is unchanged; the wiring is additive)
- User benefit: keyword search actually filters by query. Pre-thinking bundles surface different content for different prompts.

## Source

2026-04-27 audit; the doc-comment in `archive_loader.rs:78-80` is itself a confession of the test-double behaviour shipping in production.
