# vectorizer-sdk 3.0.3 follow-up — 2 of 6 drifts resolved; 3, 4, 5, 6 still open server-side

**Category**: integration
**Tags**: vectorizer, sdk-drift, integration, cortex-embedder, followup, sdk-3-0-3

## Description

Follow-up to the `vectorizer-sdk-3-0-drifts-from-hivehub-vectorizer-3-0-0-dev-image` anti-pattern. SDK 3.0.3 (released after round 5 of the `cortex-embedder` work) fixed two of the six documented mismatches. The rest are **server-side bugs** that SDK 3.0.3 cannot fix alone; it only documents them in its rustdoc. Status as of round 7:

**RESOLVED in SDK 3.0.3** — remove the round-5 workarounds:
1. **Auth** — SDK now exposes `VectorizerClient::login(username, password) -> JwtToken` at `vectorizer_sdk::client::core::JwtToken`. The returned `access_token` feeds back into `ClientConfig::api_key`; the 3.0.3 HTTP transport sniffs the three-segment JWT shape and sends `Authorization: Bearer` automatically. Custom `reqwest`-based `login_token()` helper has been deleted from `cortex-embedder/tests/common/mod.rs`.
2. **Insert path** — SDK's `insert_texts(collection, Vec<BatchTextRequest>)` now posts to the correct `POST /insert_texts` with body `{ "collection": "...", "texts": [...] }`. `BatchResponse` is tolerant of both the pre-v3 (`successful_operations`/`failed_operations`) and v3 (`inserted`/`failed`) shapes via `serde(alias)`. The direct-`reqwest` `insert_one` in `LiveVectorizerClient` has been deleted.

**STILL OPEN** (server bugs, tracked in ADR 0001):
3. **get_vector fabricated response** — server returns 200 with a synthetic `[0.1, 0.1, …]` vector for any id, even missing ones. SDK 3.0.3's rustdoc on `get_vector` recommends `list_vectors` for miss detection. `LiveVectorizerClient::exists` now walks `GET /collections/{c}/vectors?limit=…&offset=…` (direct `reqwest`, since SDK 3.0.3 does not expose `list_vectors` yet) and intersects client ids from the `payload.chunk_id` field.
4. **Server-assigned UUIDs discard client id** — `insert_texts` response's per-result `client_id` now round-trips (SDK 3.0.3 `BatchResponse.results[i].client_id`), but the stored vector id is still the server UUID. Workaround: `chunk_to_batch_request` stores the client id in metadata under the `chunk_id` key so the list view can round-trip it.
5. **BM25-512 default dim** — unchanged. IT tests still use `dim=512` in `it_schema()`; `CollectionSchema::default()` stays 768 for production.
6. **No text-search endpoint** — `POST /collections/{c}/search_vectors` still 404s on the dev image; `/search` still wants a precomputed vector.

**Implementation footprint after 3.0.3 upgrade**:
- `LiveVectorizerClient::login` (new) wraps `sdk.login`.
- `LiveVectorizerClient::upsert_chunks` now a pure SDK call to `sdk.insert_texts`.
- `LiveVectorizerClient::list_stored_chunk_ids` replaces the per-id `vector_exists` + `is_fabricated_vector_response` heuristic.
- `reqwest::Client` still lives in the struct for the one pagination path; delete once SDK exposes `list_vectors`.

## Example

// NEW in round 7:
// crates/cortex-embedder/src/vectorizer_client.rs
pub async fn login(base_url, user, pw) -> Result<JwtToken, _> {
    let sdk = SdkClient::new(ClientConfig { base_url, .. })?;
    sdk.login(user, pw).await.map_err(sdk_error)
}
// upsert_chunks: loop of sdk.insert_texts(collection, Vec<BatchTextRequest>).
// exists: walks GET /collections/{c}/vectors?limit+offset, collects payload.chunk_id.

## When to Use

When operating against the `hivehub/vectorizer:3.0.x` server with `vectorizer-sdk` 3.0.3 or later. Supersedes the "use direct reqwest for login and insert" advice of the original entry.

## When NOT to Use

If the server image advances to a build that honours client ids (bug 4) and exposes a text-search endpoint (bug 6), delete the list-view workaround entirely and replace with SDK `get_vector`/`search_vectors` assertions.
