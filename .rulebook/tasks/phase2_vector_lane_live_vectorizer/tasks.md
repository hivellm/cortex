## 1. Live vector lane impl
- [ ] 1.1 Add `vectorizer-sdk` dep (workspace) — pinned to the version `cortex-embedder-worker` uses
- [ ] 1.2 New `cortex-embedder::vector_lane::VectorizerLane { client, embedder, default_collection }` implementing `cortex_api::VectorLane`
- [ ] 1.3 KNN path: when the request carries a precomputed `query_vector`, call SDK directly
- [ ] 1.4 Text path: when only `query` text is present, embed via the embedder service before KNN (cache by query text hash with a short TTL)
- [ ] 1.5 Filter passthrough: support `filter` (kind, repo, since) by translating to the SDK's filter shape

## 2. Boot-time wiring
- [ ] 2.1 Read `VECTORIZER_URL` / `CORTEX_VECTORIZER_URL` env at startup
- [ ] 2.2 Probe with a `health` GET; on success swap `MemoryVectorLane` for `VectorizerLane`
- [ ] 2.3 On failure, log warn and keep `MemoryVectorLane` (preserve dev workflow)
- [ ] 2.4 Boot wiring populates `VectorRequest.collection` from `cortex-storage::collections::COLLECTIONS`

## 3. Score + source correctness
- [ ] 3.1 Hits emerge with `source = "vector"` and the SDK's similarity score in `[0,1]`
- [ ] 3.2 RRF fusion no longer relies on positional rank when a real similarity is present

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation (extend spec-06 with a `## Read path` section)
- [ ] 4.2 Write tests: unit tests with a wiremock Vectorizer fixture; integration test driving the orchestrator end-to-end
- [ ] 4.3 Run tests and confirm they pass
