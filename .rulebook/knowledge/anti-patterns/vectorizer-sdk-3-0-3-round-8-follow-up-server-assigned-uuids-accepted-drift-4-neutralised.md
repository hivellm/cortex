# vectorizer-sdk 3.0.3 round-8 follow-up — server-assigned UUIDs accepted, drift #4 neutralised

**Category**: integration
**Tags**: vectorizer, sdk-drift, integration, cortex-embedder, followup, sdk-3-0-3, round-8

## Description

Update to `vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side`. The round-8 canonical-id refactor neutralises drift item (4) from the embedder's perspective without requiring a server fix.

**Item (4) — "Server-assigned UUIDs discard client id": RESOLVED in cortex-embedder via the `dedup-by-metadata-key-when-server-assigns-primary-ids` pattern.**

The embedder no longer treats its deterministic client-side id as the primary id at all. `Chunk::chunk_id` was renamed to `Chunk::dedup_key`, the SDK's `BatchTextRequest.id` is still populated with it (because `BatchResultEntry::client_id` round-trips it), but the canonical chunk id is now the server-assigned UUID surfaced via `UpsertedChunk::server_id` / `EmbedReport::new_records`. Idempotency is enforced by pre-scanning the destination collection's `metadata.dedup_key` set rather than trusting the server to honour client ids on write.

The server bug itself remains open upstream — any future non-embedder client will hit the same behaviour — but cortex-embedder is no longer affected. `LiveVectorizerClient::exists_by_dedup_key` now returns precise answers against the live `hivehub/vectorizer:3.0.x` image, and the round-8 IT suite (`it_vectorizer` + `it_end_to_end`) asserts strict dedup semantics instead of the round-5 "defensive disjunction" pattern.

**Items (3), (5), (6) — unchanged**: still open server-side.

- (3) `get_vector` fabricated response: still worked around via list-view pagination in `list_stored_dedup_keys`.
- (5) BM25-512 default dim: still forces `it_schema()` to use `dim=512`.
- (6) No text-search endpoint: `it_end_to_end` still asserts server-side list count instead of retrieval.

**Implementation footprint of the round-8 change** (see `crates/cortex-embedder/src/vectorizer_client.rs`):

- `UpsertReport` now carries `new_entries: Vec<UpsertedChunk>` (the `dedup_key → server_id` mapping).
- Trait method renamed `exists → exists_by_dedup_key`.
- `Chunk::chunk_id → Chunk::dedup_key` (crate-wide rename, touches 10 files).
- `identity::chunk_id → identity::dedup_key` (same formula).
- Worker publishes an optional `server_ids` array on `cortex.events.embedded` so downstream consumers can join Nexus / query API back to the stored vectors.

This neutralisation is forward-compatible: when the server is fixed to honour client ids (or the SDK ships key-based upsert), the client can drop the pre-scan and promote `dedup_key` to the primary id slot without touching callers that consume `UpsertedChunk`.

## Example

// Round-8 canonical id model:
pub struct UpsertedChunk { pub dedup_key: String, pub server_id: String }
// The dedup_key drives idempotency (pre-upsert scan);
// server_id is the handle downstream consumers use.

## When to Use

When reasoning about which vectorizer drifts remain open for the embedder: consult this entry for the round-8 state. For clients other than cortex-embedder, the server bug still applies — see the original follow-up entry for the full list.

## When NOT to Use

If the server image advances to one that honours client ids, archive this entry and drop the dedup-by-metadata pattern from cortex-embedder.
