## 1. Live vector lane impl
- [x] 1.1 Add `vectorizer-sdk` dep (workspace) — pinned to the version `cortex-embedder-worker` uses
- [x] 1.2 New `cortex-api::vectorizer_lane::VectorizerLane` implementing `cortex_api::VectorLane`
- [x] 1.3 Server-side embedding via SDK `search_vectors(collection, query, limit, threshold)` — same surface the embedder uses for write traffic
- [x] 1.4 Auth flow mirrors embedder-worker: explicit JWT wins, else username/password runs `/auth/login` once at boot, else no-auth
- [x] 1.5 Connection-error path returns `LaneError::Transport` (404 / "not found" returns empty hits, mirroring keyword lane's lazy-collection handling)

## 2. Boot-time wiring
- [x] 2.1 Read `VECTORIZER_URL` / `CORTEX_VECTORIZER_URL` env at startup
- [x] 2.2 Probe with `health_check` SDK call; on success swap `MemoryVectorLane` for `VectorizerLane`
- [x] 2.3 On failure, log warn and keep `MemoryVectorLane` (preserve dev workflow)
- [x] 2.4 Boot wiring leaves `VectorRequest.collection` to the strategies layer (`repo_scoped(req, "code")` → `cortex-{slug}-code`); same alias the embedder upserts into

## 3. Score + source correctness
- [x] 3.1 Hits emerge with `extras["source"] = "vector"` and the SDK's similarity score in `[0,1]`
- [x] 3.2 RRF fusion now sees real similarity scores (the previous double scored 0 / positional)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation (spec-06 `## Read path` section: live lane, hit projection, failure handling, configuration)
- [x] 4.2 Write tests covering the new behavior (3 unit tests for project() + 6 wiremock integration tests: query forwarding, 404 collection, distinct queries, fail-open, trait swap)
- [x] 4.3 Run tests and confirm they pass — 91 cortex-api tests green
