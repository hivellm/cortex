# Proposal: phase0_captured-memory-not-retrievable-via-query

Source: phase28 manual E2E verification (run 1c, 2026-06-20) — finding F3.

## Why

`cortex_capture_memory` promises "the captured body becomes queryable on
the next cortex_query free-text search", but it does NOT. Verified live:

1. Captured a memory with a rare marker (`zeta-7731`) via
   `cortex_capture_memory {kind:memory, repo:cortex}` -> returned
   `event_id 01KVK0WJ67...`, `indexed_at` set.
2. It IS indexed: `cortex_keyword_search cortex-cortex-misc q="zeta-7731"`
   returns the doc (id `01KVK0WJ67...`, body contains the marker).
3. But `cortex_query {intent:free_search, scope.repo:cortex,
   query:"zeta-7731 ..."}` returns 0 snippets containing the marker, even
   35s later. `zeta-7731` is a rare exact token that would rank #1 if the
   lane searched that index.
4. The `cortex_memories` GLOBAL index does not exist
   (`index_not_found`); memories route to the per-repo
   `cortex-<repo>-misc` family.

Conclusion: captured memories (and likely `knowledge` / `learning`, which
share the `misc` family) are indexed but the `cortex_query` fusion lane
does not search the `misc`/memory family for free_search, so the whole
in-session memory feature is write-only from the query path's view —
defeating the purpose of `cortex_capture_memory`.

## What Changes

- Confirm the orchestrator/strategies lane coverage
  (`crates/cortex-api/src/search/strategies.rs`) for `free_search` (and
  the other intents) and add the `misc` family (memory / knowledge /
  learning) to the Meili keyword lane fan-out, so captured memories are
  retrievable.
- Verify the dense (Vectorizer) lane likewise covers the memory
  collection if memories are embedded.
- Decide global vs per-repo: either create/populate a `cortex_memories`
  global index, or ensure the per-repo `cortex-<repo>-misc` family is in
  the fan-out (preferred — data already lands there).

## Impact

- Affected specs: spec 11 (query lanes / fan-out), spec 16/20
  (capture_memory contract).
- Affected code: `crates/cortex-api/src/search/strategies.rs` (lane
  family selection); possibly the fulltext routing for the memories
  index name.
- Breaking change: NO (adds coverage; strictly more recall).
- User benefit: `cortex_capture_memory` actually round-trips —
  in-session facts become retrievable, as documented.
