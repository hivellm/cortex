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
- [x] §1.1 Backfill the `analyses` index summaries — done via the §1.3 re-emit; `cortex-cortex-analyses` re-embedded (2816→967 after dedup). Summaries are the static-classifier deterministic template. NOTE (new finding): the static `summary` is raw-JSON-ish (`"artifact in cortex: {json}"`), so it does not produce clean NL — the real retrieval-quality lever is NL summary quality (LLM or better static projection); tracked in the phase26f follow-up.
- [x] §1.3 One-time dedupe transition: dropped the bootstrap-refillable cortex collections (`cortex-cortex-{docs,code,analyses,knowledge,learnings}`), rebuilt+redeployed the stable-id embedder (git 9d480ba), then re-emitted via `cortex-bootstrap . --kinds docs,code,analyses,knowledge,learnings --force` (2235 events, 13446 chunks, 0 vectorizer errors). Massive dedupe, no content lost (docs/specs confirmed present, 20 chunks/spec file): docs 23315→4006, code 142675→10586, analyses 2816→967, knowledge 174→83, learnings 165→111 (~169k→~15.7k total, ~90% bloat removed). Turns NOT touched (session-derived, not bootstrap-refillable; needs archive replay) → phase26f follow-up.
- [x] §1.4 Re-measured (evidence-based; 0.50 target revised per user "realistic target"): the dedupe removed ~90% duplicate vectors (precision win) but did NOT raise the top-1 score, because the embedding model (nomic-embed-text 384d) caps at ~0.42–0.45 raw cosine for the generic query "event classification system" on clean markdown prose — 0.50 via `/v1/query` (fused ~0.17–0.24) is unreachable by purging. Headline metric reset to: dedupe verified (counts above) + raw-cosine ceiling documented. The score lever is NL summary quality (phase26f follow-up), not dedupe.

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
