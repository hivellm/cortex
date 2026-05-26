## 1. Wire-shape types
- [x] 1.1 Define `WireSearchResponse { results: Vec<WireSearchHit> }` and `WireSearchHit { id: String, score: f32, vector: Option<Vec<f32>>, payload: serde_json::Map<String, serde_json::Value> }` in `vectorizer_lane.rs` (private)
- [x] 1.2 `serde(default)` on every field so a missing key never fails deserialisation

## 2. Direct HTTP call replacing the SDK
- [x] 2.1 Add a `reqwest::Client` to `VectorizerLane` (build with the same 10 s timeout the SDK uses)
- [x] 2.2 Replace `client.search_vectors(...)` in `VectorLane::search` with `reqwest` POST to `{base_url}/collections/{c}/search/text` body `{ "query": ..., "limit": ... }`
- [x] 2.3 Auth header: pull the cached JWT from the SDK client (or duplicate the cached value in `VectorizerLane` so the reqwest call carries `Authorization: Bearer <jwt>`) — implemented as `bearer: Arc<RwLock<Option<String>>>` mirrored by `with_login` / `with_initial_jwt_for_test` / `refresh_token`
- [x] 2.4 Keep `probe_authenticated`, `refresh_token`, `health_check`, `login` on the SDK — those wire shapes already match
- [x] 2.5 Preserve the existing 401 → refresh → retry path verbatim (just route the retry through the new reqwest call) — refactored as the `DirectSearchOutcome` enum dispatched from `VectorLane::search`

## 3. Update `project()` to read `payload`
- [x] 3.1 Function signature changes from `(SearchResult, &VectorRequest) -> LaneHit` to `(WireSearchHit, &VectorRequest) -> LaneHit`
- [x] 3.2 Replace `metadata.get(key)` with `payload.get(key)` for every key (`path`, `kind`, `repo`, `severity`, `ts`, `topics`, `topic`, `summary`, `title`, `body`, `content_hash`, `chunk_id`, `dedup_key`, `parent_event_id`, `language`)
- [x] 3.3 Keep the legacy nested fallback (`payload.payload.<key>`) for older embedder builds — applied to both spec-11 contract keys and the `body`/`summary`/`title` text-bearing chain
- [x] 3.4 Keep the phase10b §1 path-as-text guard (collapse text when it equals the path)

## 4. Tests
- [x] 4.1 Update every `wiremock` `Mock::given(method("POST")).and(path("/collections/.../search/text"))` to respond with the real wire shape `{id, score, vector, payload}` instead of the SDK-style `{id, score, content, metadata}`
- [x] 4.2 Add a regression test asserting `LaneHit.text` is non-empty when `payload.body` is non-empty (`live_lane_passes_query_text_through_to_vectorizer_search`)
- [x] 4.3 Add a regression test asserting `LaneHit.path` matches `payload.path` (same test)
- [x] 4.4 Add a regression test asserting the legacy nested `payload.payload.path` still resolves (`projects_legacy_nested_payload_for_text_and_contract_keys` + `vectorizer_falls_back_to_nested_payload_when_top_level_lacks_the_key`)
- [x] 4.5 Confirm the existing `live_lane_passes_query_text_through_to_vectorizer_search` test still passes (just the response shape changes)
- [x] 4.6 Bonus: regression test pinning the failure mode when the upstream re-emits the legacy SDK shape (`live_lane_rejects_legacy_sdk_shape_as_empty_hits`) and when payload is omitted entirely (`live_lane_drops_text_when_payload_omitted`)

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — module-level doc comment in `vectorizer_lane.rs` explains the drift and the fix; phase11a's `docs/operations/vectorizer-auth.md` already references the lane structure
- [x] 5.2 Write tests covering the new behavior
- [x] 5.3 Run tests and confirm they pass (274 unit + 12 integration tests, 0 failures; e2e validated via live docker stack — vector hits surface in MCP cortex_query with non-empty paths)
