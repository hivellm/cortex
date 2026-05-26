# Proposal: phase2_static_classifier_summary_preserves_text

## Why

The 2026-04-27 reindex shows the static classifier is overwriting every artifact body with a meaningless metadata stub before it reaches Meilisearch.

Sample doc pulled from `cortex-docs` index:
```json
{
  "id": "01KQ7VRAV1N9VS5MBP1YQNREPT",
  "title": "scripts/bench-config.yml",
  "body": "static summary: 12467 chars",
  "summary": "static summary: 12467 chars",
  "path": "scripts/bench-config.yml",
  "topics": ["code", "yaml"],
  "repo": "Synap"
}
```

The `body` and `summary` fields are the literal string `"static summary: " + content_length`. The actual file body is gone. Result: every keyword search comes back with **0 hits** because the indexed body contains no real content tokens. Confirmed empirically across 5 distinct queries against the 10 000 indexed docs — none searchable.

The bug lives in the `StaticClassifier` (`crates/cortex-classifier/`). It produces a debug-shaped placeholder string instead of either (a) leaving `summary = None` so downstream consumers fall back on the original `text`, or (b) computing a real summary (first N chars / first sentence).

Source: 2026-04-27 reindex audit. Vectorizer cortex-docs (39 521 vectors) and Meilisearch cortex-docs (8 285 docs) both carry the destroyed body.

## What Changes

- `StaticClassifier` no longer stamps `summary = "static summary: <N> chars"`. Two options ranked by simplicity:
  1. **Drop the field.** `summary = None` for all static-classified events. Downstream readers (`cortex-fulltext-worker`, `cortex-embedder`) already fall back to the source `text` field when `summary` is missing.
  2. **Real summary.** Take the first non-empty line clipped at 240 chars.

  Pick option 1 unless the worker chain depends on a non-empty summary somewhere — option 2 only as a follow-up if the simpler fix breaks a test.

- `cortex-fulltext-worker` builds the Meilisearch document with `body = source_text` when `classifier.summary.is_none()` (or empty). Today it appears to copy `summary` verbatim into `body` regardless.

- Backfill: existing 10 000 cortex-docs entries are unsalvageable as-indexed. Drop and re-emit them via `cortex-bootstrap`. The retest probe runs after the rebuild and asserts a known query (`"HNSW recall benchmark"`) returns at least one hit pointing at `Vectorizer/benches/`.

## Impact

- Affected specs: spec-05 (classifier summary contract).
- Affected code:
  - `crates/cortex-classifier/src/static_classifier.rs` (or the impl that builds `ClassifierOutput.summary`)
  - `crates/cortex-fulltext/src/document.rs` (or wherever the Meili doc is constructed) — fall back to `text` when `summary` is empty
  - integration test asserting Meilisearch returns hits for known terms
- Breaking change: NO — `ClassifierOutput.summary` stays `Option<String>`; readers already handle `None`.
- User benefit: 10 000 indexed docs become searchable; Meilisearch keyword lane (phase2_keyword_lane_live_meilisearch) becomes useful.
