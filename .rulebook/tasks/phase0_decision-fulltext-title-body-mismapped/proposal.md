# Proposal: phase0_decision-fulltext-title-body-mismapped

Source: phase28 manual E2E verification (run 1, 2026-06-20) — finding F2.

## Why

A SUBSET of `cortex_decisions` Meili documents are malformed: `title` =
the document ULID instead of the decision title, and no `decision_title`
field. Refined diagnosis from two probe paths (phase28 run 1):

- `cortex_keyword_search cortex_decisions q="" attributes=[id,title,
  decision_title]` shows the `01KQNYF4J*` batch (6/6 sampled) with
  `title == id` and NO `decision_title` — malformed.
- The SAME tool with `q="vectorizer"` returns doc `01KQNYMYKH...` with the
  CORRECT `title` ("1. Bypass vectorizer-sdk ...") + `decision_title` +
  clean `body`.
- So the index is a **mix**: ~one ingest batch (`01KQNYF4J*`) is malformed
  while newer docs are correct. 51 decision docs total.

This is NOT the `decision_search` handler (it returns raw Meili hits
verbatim) and NOT the whole corpus — it is stale malformed docs from an
earlier/buggy decision-ingest run that were never reindexed. The current
builder CAN produce correct docs (proven by `01KQNYMYKH`). Net effect:
keyword search misses those decisions by title and pollutes BM25 with the
`title==id` rows.

## What Changes

- Confirm the current decision document builder
  (`crates/cortex-workers/src/fulltext/builders.rs` + routing) sets
  `title`/`decision_title`/`body` correctly (it appears to — verify and
  pin with a unit test asserting `title != id` and a populated
  `decision_title`).
- Identify how the `01KQNYF4J*` batch was produced with `title==id` (older
  builder revision, or a different ingest path that bypassed title
  extraction) — fix that path if it still exists.
- **Reindex** `cortex_decisions` (re-emit the decision envelopes through
  the current builder) so the stale malformed docs are corrected; document
  the reindex command.
- Add a doctor/coverage check that flags decision docs where `title == id`
  so malformed batches are caught automatically in future.

## Impact

- Affected specs: spec 08 (fulltext indexer document schema), spec 27
  (decisions).
- Affected code: `crates/cortex-workers/src/fulltext/builders.rs` (+ the
  decision routing/ingest path); reindex tooling; optional doctor check.
- Breaking change: NO (index content fix; re-emit corrects docs).
- User benefit: keyword search on decisions matches real titles; no
  `title==id` rows polluting the decision lane.
