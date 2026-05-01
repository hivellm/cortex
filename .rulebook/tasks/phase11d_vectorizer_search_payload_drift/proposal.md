# Proposal: phase11d_vectorizer_search_payload_drift

## Why

Even after phase11a (the JWT auth fix), the cortex-api vector lane returns hits with empty `text` and missing `path`. Verified live on 2026-05-01 by curling the Vectorizer directly and comparing with the cortex-api MCP probe.

**Direct curl** against `POST /collections/cortex-cortex-code/search/text` returns 8 hits with rich payloads:

```json
{
  "id": "4fdf6e92-fd32-4e65-8985-89c5ba43be72",
  "score": 0.140,
  "vector": [0.019, ...],
  "payload": {
    "path": "crates/cortex-bootstrap/tests/runner.rs",
    "kind": "Artifact", "language": "rust", "repo": "Cortex",
    "symbol": "idempotent_replay_reuses_checkpoint_resume",
    "dedup_key": "4RTFH3SB3VJ8ZD4CJ7J9F1BYN2",
    "parent_event_id": "01KQAENWZ9HQ5G9656EYNA3Y3M",
    "byte_start": "3929", "byte_end": "5437",
    "chunk_content_hash": "e35f7f...",
    "parent_content_hash": "sha256:bbe080...",
    "severity": "info", "source": "code"
  }
}
```

**cortex-api** consumes the same hits via `vectorizer_sdk::VectorizerClient::search_vectors`, which deserialises into [`vectorizer_sdk::models::SearchResult`](https://docs.rs/vectorizer-sdk/3.0.3/vectorizer_sdk/models/struct.SearchResult.html):

```rust
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub content: Option<String>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}
```

The server-side wire fields are `payload` and `vector`. The SDK looks for `content` and `metadata`. Serde tolerantly deserialises and silently produces `SearchResult { content: None, metadata: None }` for every hit. [crates/cortex-api/src/vectorizer_lane.rs:461-548](crates/cortex-api/src/vectorizer_lane.rs#L461-L548) then reads `r.metadata.get("path")` etc. and projects `LaneHit { text: "", path: None, ... }` for every hit.

Symptom on every cortex_query call:
- `debug.lanes.vector_ms > 0` (lane runs, no 401 — phase11a working)
- `source-mix: {keyword: 5}` (every snippet that surfaces is from the keyword lane)
- The vector lane's hits get filtered out at the bundle renderer because their text is empty (phase10b §1: empty-text projection collapses to a header-only line and the renderer drops it from the bundle).

This is the **same family of anti-pattern** as the two previously-documented vectorizer-sdk drifts (auth, write path), but for the read path. The existing pattern is "bypass the SDK and POST direct via `reqwest` for the operations whose wire shape doesn't match" — the phase11d fix applies it to `search_vectors` too.

## What Changes

1. **Bypass the SDK's `search_vectors` in cortex-api.** Replace `client.search_vectors(...)` in `vectorizer_lane.rs::search` with a direct `reqwest::Client` call to `POST /collections/{c}/search/text` carrying the `Authorization: Bearer <jwt>` header (cached in the same place the SDK keeps it). Keep `probe_authenticated`, `refresh_token`, and the 401 retry path on the SDK — those endpoints' wire shapes do match.

2. **Define a local `WireSearchResponse` / `WireSearchHit` for the real shape.** Fields: `id: String, score: f32, vector: Option<Vec<f32>>, payload: HashMap<String, serde_json::Value>`. `vector` deserialised but immediately dropped (we don't need it for projection). `payload` becomes the source of truth for `project()`.

3. **Update `project()` to read from `wire.payload` instead of `r.metadata`.** Every key the projection uses (`path`, `kind`, `repo`, `severity`, `ts`, `topics`, `topic`, `body`, `summary`, `title`, `content_hash`) must move from the metadata path to the payload path. The `payload.payload.<key>` legacy fallback (phase6b) stays — older embedder builds nested under that key.

4. **Honour `body` / chunk text fallback.** The wire payload doesn't always carry the chunk text; it carries `byte_start` / `byte_end` and `parent_content_hash`. When `payload.body` is missing, fall through to `payload.summary` / `payload.title` exactly as today; if all three are missing, the hit's `text` stays empty and the renderer collapses it (the existing phase10b §1 behaviour). Don't synthesise text from path — that's the regression the phase10b §1 guard already catches.

5. **Keep the SDK as a fallback for non-search ops.** `list_collections` (used by `probe_authenticated`), `health_check`, `login`, `refresh_token` continue through the SDK. Only the read path's `search_vectors` swaps to direct HTTP.

6. **Don't move other ops.** This proposal does not touch the embedder's write path (already direct) or the orchestrator's collection-naming logic (that's phase11e). The diff stays inside `vectorizer_lane.rs`.

## Impact

- Affected code:
  - [crates/cortex-api/src/vectorizer_lane.rs](crates/cortex-api/src/vectorizer_lane.rs) — replace SDK `search_vectors` with direct HTTP, update `project()` to read `payload`.
- Affected tests:
  - [crates/cortex-api/tests/vectorizer_lane.rs](crates/cortex-api/tests/vectorizer_lane.rs) — existing `wiremock` mounts respond at `/collections/{c}/search/text` with `{results: [{id, score, content, metadata}]}`. The mocks must change to the real wire shape (`{id, score, vector, payload}`); the assertions on text/path/source stay.
- Breaking change: NO at the API surface — `cortex_query` callers don't change. Internal-only refactor.
- User benefit: vector lane snippets actually carry text and path. `cortex_query` results stop being keyword-only, RRF fusion works as designed, and the most-relevant document for a query (the source file rather than only its tests) starts surfacing in the top hits.

## Source

- Live curl against `cortex-vectorizer` (2026-05-01): server response shape captured verbatim above.
- [crates/cortex-api/src/vectorizer_lane.rs:461-548](crates/cortex-api/src/vectorizer_lane.rs#L461-L548) — `project()` implementation reading from `metadata`.
- [vectorizer-sdk 3.0.3 src/models.rs:192-202](https://docs.rs/vectorizer-sdk/3.0.3/) — `SearchResult` struct shape.
- [vectorizer-sdk 3.0.3 src/client/search.rs:21](https://docs.rs/vectorizer-sdk/3.0.3/) — `search_vectors` POSTs to `/collections/{c}/search/text` and parses with `serde_json::from_str::<SearchResponse>`.
- Existing anti-patterns in `rulebook_knowledge`: `vectorizer-sdk-3-0-drifts-from-hivehub-vectorizer-3-0-0-dev-image`, `vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side`. This is a new instance of the same family on the read path.
