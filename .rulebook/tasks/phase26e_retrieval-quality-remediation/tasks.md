## §1. Gap A — stable vector id (replace-in-place), clean re-embed (Bug #8 finish)

> Research finding (phase26e): the Vectorizer `insert_texts` UPSERTS by the
> client-supplied id, and the vector id == the embedder's `dedup_key`. Because
> `dedup_key = f(event_id, ordinal, chunk_content_hash)` folds the content hash
> into the id, a content change (e.g. phase0 raw-JSON→NL projection) produces a
> NEW id and orphans the old vector — that is the additive bloat (turns: 33k
> vectors for ~8k events). There is no delete-by-filter and the list scan is
> capped at 10k, so a per-event scan-delete does not scale. The fix is a
> **content-independent stable id** so a re-embed replaces in place.

- [x] §1.2 Embedder: introduced `vector_id(event_id, ordinal)` (content-independent) used as the `insert_texts` id; kept content-hash `dedup_key` in payload + exists pre-check for dedupe-on-unchanged. Content change now re-upserts the SAME id (replace), no orphan. MemoryVectorizerClient re-keyed by vector_id to model upsert-by-id. Tests: vector_id determinism/ordinal/content-independence/≠dedup_key (identity.rs), chunk_to_batch_request_uses_vector_id_as_id_and_keeps_dedup_key_in_metadata, reembed_with_changed_content_replaces_in_place. embedder 59/59, pruner 18/18, clippy clean. (Built before §1.1/§1.3 — exemption #1: the re-embed must run through the stable-id path or it re-orphans.)
- [ ] §1.1 Backfill the `analyses` index summaries (phase26d found 22% coverage; turns/code/docs already 100%) — satisfied by the §1.3 bootstrap --force re-run, which re-runs the static classifier (summaries) over all cortex events.
- [ ] §1.3 One-time transition: drop the stale cortex vector collections (`cortex-cortex-{turns,code,docs,analyses,knowledge}`) then re-embed via `cortex-bootstrap --force --only cortex` so the collections refill with clean NL vectors under stable ids.
- [ ] §1.4 Re-measure: `/v1/query` repo=cortex query="event classification system" top vector hit must clear 0.50 (phase26d measured 0.238 with additive vectors).

## §2. Gap B — observable bundle-cache hit rate

- [ ] §2.1 Publish adapter `cache_hit_total` / `cache_miss_total` into the `cortex-adapter` subsystem `extras` (or a dedicated adapter health endpoint).
- [ ] §2.2 Verify two identical pre-thinking queries within the 60s TTL increment the live `cache_hit_total`.

## §3. Gap C — dedicated pre-thinking latency metric

- [ ] §3.1 Record bundle-assembly latency separately from envelope `duration_ms`.
- [ ] §3.2 Surface it as its own dashboard series (or repoint `pre_thinking_p95_ms` to the real source); confirm < 200ms for repeated same-scope/intent queries.

## §4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §4.1 Update or create documentation covering the implementation
- [ ] §4.2 Write tests covering the new behavior
- [ ] §4.3 Run tests and confirm they pass
