# vectorizer-sdk 3.0 drifts from hivehub/vectorizer:3.0.0 dev image

**Category**: integration
**Tags**: vectorizer, sdk-drift, integration, http-bypass, cortex-embedder

## Description

The bundled `vectorizer-sdk 3.0.0` crate targets endpoints and response shapes that the `hivehub/vectorizer:3.0.0` Docker image does not serve. Concrete mismatches discovered while wiring `cortex-embedder`:

1. **Auth**: SDK treats `api_key` as a static bearer; the dev image requires `POST /auth/login` with `{username, password}` to mint a short-lived JWT that must then ride as `Authorization: Bearer`.
2. **Insert path**: SDK's `insert_texts` posts to `POST /collections/{c}/documents` which 404s; the server accepts `POST /insert` with a per-text body `{collection, id, text, metadata}`.
3. **Client id not honoured**: `POST /insert` allocates a fresh UUID per write and ignores the caller's `id` field entirely, so server-side dedup via client ids is impossible.
4. **get_vector shape drift + fabricated responses**: SDK parses `{data: [...]}`; server emits `{vector: [...]}`. Worse, the server returns HTTP 200 with a uniform `[0.1, 0.1, …]` vector for *any* requested id in any collection — even ids that were never stored — so a naive 200-means-present `exists` probe over-reports.
5. **Default embedder is BM25-512, not 768**: `POST /insert` requires `collection.dimension == 512`; the SDK's and spec's default of 768 triggers `invalid_dimension` errors.
6. **No text-search endpoint**: SDK's `search_vectors` targets a path the dev image doesn't serve, and `/search` demands a precomputed dense vector that the BM25 fallback cannot synthesise from a text query.

**Recommendation**: bypass the SDK for `insert_chunks` and `vector_exists` via direct `reqwest` calls against the real endpoints; keep the SDK only for operations that do route correctly (`get_collection_info`, `create_collection`, `delete_collection`). Distinguish the fabricated `[0.1, 0.1, …]` response from real vectors by checking per-dimension variance. Revisit when `vectorizer-sdk 3.1+` aligns with server routes or when we pin a different server build.

## Example

// See crates/cortex-embedder/src/vectorizer_client.rs — `insert_one` and
// `vector_exists` bypass the SDK via reqwest. `is_fabricated_vector_response`
// filters the dev image's uniform-0.1 response so `exists` stays accurate.

## When to Use

When integrating any Cortex worker against the hivehub/vectorizer:3.0.0 dev image — the mismatches apply uniformly.

## When NOT to Use

Once vectorizer-sdk 3.1+ ships with aligned routes and response shapes, or if we target a pinned server build that honours the SDK contract.
