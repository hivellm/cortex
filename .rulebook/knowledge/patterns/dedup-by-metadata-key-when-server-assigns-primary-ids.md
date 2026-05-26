# dedup-by-metadata-key-when-server-assigns-primary-ids

**Category**: integration
**Tags**: integration, vectorizer, dedup, idempotency, metadata, cortex-embedder

## Description

When a vector/document store reassigns primary ids server-side (ignoring the client-supplied `id` — as `hivehub/vectorizer:3.0.x` does on `POST /insert_texts`), stop fighting the server and adopt its UUID as canonical. Carry the client-side deterministic identifier as a metadata field instead.

**Recipe** (implemented in `cortex-embedder`, round 8):

1. **Generate a deterministic client-side key.** For every pre-upsert item, compute `dedup_key = ulid_from_hash(parent_id || ':' || ordinal || ':' || content_hash)` (or analogous stable derivation). Store it on the item as a field distinct from any "primary id" slot the client transport might have.

2. **Send the dedup key as metadata.** Include `metadata.dedup_key = <key>` in the upsert body. Most vector stores pass metadata through verbatim even when they reassign ids. Cross-check by inspecting a `list_vectors`-style response post-insert: if the metadata field survives, the pattern is viable. If the server strips metadata, this pattern doesn't apply.

3. **Pre-upsert scan.** Before every `upsert_chunks`/equivalent call, paginate `GET /collections/{c}/vectors?limit+offset` (or the store's equivalent list view), extract `payload.metadata.dedup_key` per entry, build a `BTreeSet<String>`, and filter chunks whose key is already present. Bound the scan at a sane page cap (e.g., `LIST_PAGE_HARD_LIMIT = 50 * page_size = 10_000`).

4. **Preserve the server's UUID for downstream joins.** Return a per-chunk `UpsertedChunk { dedup_key, server_id }` mapping from the upsert call; downstream consumers (graph writer linking vectors to graph nodes, query API materialising vector lookups) key off `server_id`.

5. **In-process guard.** Keep a small in-memory `processed: BTreeSet<event_id>` to collapse duplicate upstream deliveries, but do NOT rely on it for cross-run idempotency — that's the dedup key's job.

**When to use**: any integration where the store's primary-id contract doesn't match your idempotency requirements, especially when the store's REST/RPC surface is authoritative and you can't fork it. Also useful as a forward-compatible shim: if the store later grows key-based upsert, you can drop the scan and promote the dedup key to the primary id field without touching callers that read `UpsertedChunk`.

**When NOT to use**: (a) if your store honours client-supplied primary ids and supports key-based upsert, just use those — the extra metadata field is overhead; (b) if the store doesn't preserve metadata on insert (check before adopting); (c) for collections expected to exceed `LIST_PAGE_HARD_LIMIT × page_size` items where a full pre-scan would be prohibitive — switch to a server-side bulk-id lookup endpoint then.

**Cost**: one paginated list call per `embed_batch` per destination collection. For collections of 10k entries at `page_size=200`, that's 50 round-trips — acceptable for dev/medium scale, not acceptable for 1M+. Escalate to a dedicated index when throughput matters.

## Example

// crates/cortex-embedder/src/identity.rs
pub fn dedup_key(event_id: &str, ordinal: u32, content_hash: &str) -> String {
    // SHA-256 → first 16 bytes → ULID
}

// crates/cortex-embedder/src/vectorizer_client.rs
pub struct UpsertedChunk { pub dedup_key: String, pub server_id: String }
pub struct UpsertReport { pub written: u32, pub deduped: u32,
                          pub new_entries: Vec<UpsertedChunk> }

async fn exists_by_dedup_key(&self, collection, &[dedup_key]) -> BTreeSet<String> {
    let stored = self.list_stored_dedup_keys(collection).await?;
    dedup_keys.iter().filter(|k| stored.contains(*k)).collect()
}

## When to Use

Any client of a vector/document store whose server reassigns primary ids on insert AND preserves metadata verbatim — the pattern is transport-agnostic.

## When NOT to Use

Stores that honour client-supplied primary ids (just use those); stores that strip metadata; collections large enough that a pre-upsert list-scan is prohibitive.
