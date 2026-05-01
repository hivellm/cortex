## 1. Wire-shape types
- [ ] 1.1 Define `WireSearchResponse { results: Vec<WireSearchHit> }` and `WireSearchHit { id: String, score: f32, vector: Option<Vec<f32>>, payload: serde_json::Map<String, serde_json::Value> }` in `vectorizer_lane.rs` (private)
- [ ] 1.2 `serde(default)` on every field so a missing key never fails deserialisation

## 2. Direct HTTP call replacing the SDK
- [ ] 2.1 Add a `reqwest::Client` to `VectorizerLane` (build with the same 10 s timeout the SDK uses)
- [ ] 2.2 Replace `client.search_vectors(...)` in `VectorLane::search` with `reqwest` POST to `{base_url}/collections/{c}/search/text` body `{ "query": ..., "limit": ... }`
- [ ] 2.3 Auth header: pull the cached JWT from the SDK client (or duplicate the cached value in `VectorizerLane` so the reqwest call carries `Authorization: Bearer <jwt>`)
- [ ] 2.4 Keep `probe_authenticated`, `refresh_token`, `health_check`, `login` on the SDK — those wire shapes already match
- [ ] 2.5 Preserve the existing 401 → refresh → retry path verbatim (just route the retry through the new reqwest call)

## 3. Update `project()` to read `payload`
- [ ] 3.1 Function signature changes from `(SearchResult, &VectorRequest) -> LaneHit` to `(WireSearchHit, &VectorRequest) -> LaneHit`
- [ ] 3.2 Replace `metadata.get(key)` with `payload.get(key)` for every key (`path`, `kind`, `repo`, `severity`, `ts`, `topics`, `topic`, `summary`, `title`, `body`, `content_hash`, `chunk_id`, `dedup_key`, `parent_event_id`, `language`)
- [ ] 3.3 Keep the legacy nested fallback (`payload.payload.<key>`) for older embedder builds
- [ ] 3.4 Keep the phase10b §1 path-as-text guard (collapse text when it equals the path)

## 4. Tests
- [ ] 4.1 Update every `wiremock` `Mock::given(method("POST")).and(path("/collections/.../search/text"))` to respond with the real wire shape `{id, score, vector, payload}` instead of the SDK-style `{id, score, content, metadata}`
- [ ] 4.2 Add a regression test asserting `LaneHit.text` is non-empty when `payload.body` is non-empty
- [ ] 4.3 Add a regression test asserting `LaneHit.path` matches `payload.path`
- [ ] 4.4 Add a regression test asserting the legacy nested `payload.payload.path` still resolves
- [ ] 4.5 Confirm the existing `live_lane_passes_query_text_through_to_vectorizer_search` test still passes (just the response shape changes)

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
