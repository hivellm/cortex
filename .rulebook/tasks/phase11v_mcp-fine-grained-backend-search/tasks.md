## 1. cortex-api proxy module (`crates/cortex-api/src/search_proxy.rs`)

- [ ] 1.1 New module file declaring `pub async fn handle_vector_search`, `pub async fn handle_keyword_search`, `pub async fn handle_graph_query` axum handlers + the request/response wire types (`VectorSearchRequest`, `VectorSearchResponse`, `VectorHit`, etc.). All types serde-derived; reuse `cortex_workers::embedder::vectorizer_client::VectorizerClient`, `cortex_workers::fulltext::meili_client::MeiliClient`, and `nexus_sdk::NexusClient` from the existing `ApiState`. 6 unit tests (serde round-trip per type, defaults applied when fields absent).
- [ ] 1.2 `handle_vector_search` accepts either `query_text` (embed via the same model the embedder worker uses) OR `query_vector` (raw f32 vec). Rejects both-or-neither with 400 + reason `bad_input`. Calls `vectorizer.search_vectors(collection, vector, k, filters)` and folds the per-payload metadata (`event_id`, `repo`, `kind`, `occurred_at`) onto each hit via the existing payload schema. Returns `VectorSearchResponse { collection, hits, embed_latency_ms?, search_latency_ms }`. 4 unit tests with a stub VectorizerClient (happy path, both-modes-set rejected, neither-mode-set rejected, score_threshold filter applied).
- [ ] 1.3 `handle_keyword_search` accepts `{ index, q, limit?, filter?, sort?, attributes_to_retrieve? }`. Forwards verbatim to `MeiliClient::search`. Returns the raw `{ hits, processing_time_ms, estimated_total_hits }` block plus the `index` echo. Rejects unknown index with 404 + reason `index_not_found`. 4 unit tests via a stub MeiliClient (happy path, unknown-index 404, filter passthrough, limit honored).
- [ ] 1.4 `handle_graph_query` dispatches on `mode`. `neighbors` mode: BFS from `node_id` to `depth` (default 1, hard-cap 5). `cypher` mode gated on `CORTEX_GRAPH_CYPHER_ENABLED=1`; rejected otherwise with 403 + reason `cypher_disabled`. Both modes return `{ mode, nodes, edges }`. 5 unit tests (neighbors depth-1, neighbors depth>cap rejected, cypher disabled→403, cypher enabled happy path, edge_kinds filter respected).
- [ ] 1.5 Hard-cap response payload at the same `MCP_RESPONSE_HARD_CAP` `cortex_query` honours; on overflow return a soft-error JSON envelope `{ error: "budget_exceeded", payload_bytes, transport_cap_bytes, suggested_limit }`. 3 unit tests pin the cap behaviour for each handler.

## 2. cortex-api router wiring (`crates/cortex-api/src/http.rs`)

- [ ] 2.1 Mount three new routes: `POST /v1/search/vector`, `POST /v1/search/keyword`, `POST /v1/search/graph`. Same auth posture as `/v1/query` (no key required by default; `CORTEX_DASHBOARD_AUTH=1` gates the dashboard sub-router only). 3 router-build tests assert the routes resolve.
- [ ] 2.2 `ApiState` already carries the three client handles via the lane factories. Surface them through `pub fn vectorizer_client() / meili_client() / nexus_client()` accessors so `search_proxy` does not duplicate the env-resolution code. 1 round-trip test pins each accessor.

## 3. MCP tool wrappers (`crates/cortex-mcp-server/src/tools.rs`)

- [ ] 3.1 `VectorSearchTool` — `name() = "cortex_vector_search"`. `descriptor()` returns JSON-schema with `collection` (required, enum of known collections + free-form), `query_text` / `query_vector` (one-of), `k` (default 10, max 100), `repo`, `kind`, `score_threshold`. `call()` POSTs to `<api_url>/v1/search/vector`. Soft-error mapping mirrors `QueryTool`. 3 unit tests via wiremock (happy path, 400 routed to `ToolError::invalid_input`, 500 routed to soft-error).
- [ ] 3.2 `KeywordSearchTool` — `name() = "cortex_keyword_search"`. Descriptor surface mirrors §1.3. 3 unit tests via wiremock.
- [ ] 3.3 `GraphQueryTool` — `name() = "cortex_graph_query"`. Descriptor surface mirrors §1.4 (two modes, mode discriminator). 4 unit tests via wiremock (neighbors happy path, cypher disabled passthrough, cypher enabled happy path, depth-cap passthrough).
- [ ] 3.4 `ToolRegistry::default_set()` extended with the three new tools (size 7 -> 10). `tools/list` round-trip test bumped from 7 to 10. `transport_stdio` integration test count bumped accordingly.

## 4. Spec + docs

- [ ] 4.1 New `docs/specs/22-fine-grained-search.md` documenting all three endpoints + tool descriptors + the cypher gate + the hard-cap behaviour. Source-of-truth for the JSON-schema descriptors; link from spec 18 (MCP tool surface) and spec 11 (query) so the relationship is explicit.
- [ ] 4.2 `docs/specs/20-mcp-tool-surface.md` registry table extended with three new rows (read; payload caps; soft-error reasons). CHANGELOG `[Unreleased] § Added` entry covers §1-§4.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 5.1 Update or create documentation covering the implementation — §4.1 + §4.2 above are the doc deliverables.
- [ ] 5.2 Write tests covering the new behavior — 6 + 4 + 4 + 5 + 3 cortex-api unit tests (22) + 3 + 3 + 4 cortex-mcp-server tool tests (10) + 1 transport_stdio expansion + 1 round-trip test per accessor (3) = 36 new tests.
- [ ] 5.3 Run tests and confirm they pass — `cargo check --workspace` clean. `cargo clippy -p cortex-api -p cortex-mcp-server --all-targets -- -D warnings` zero new warnings on touched files. `cargo test -p cortex-api --lib search_proxy` + `cargo test -p cortex-mcp-server --lib` green. Live smoke against the running stack: `curl -X POST http://127.0.0.1:17000/v1/search/keyword -d '{"index":"cortex-cortex-consolidations","q":"","limit":3}'` returns the 3 freshly-consolidated session rows.
- [ ] 5.4 Capture at least one learning — likely candidate: how the cypher gate landed (env-var vs explicit handler-side allowlist tradeoff); record the choice + why for future graph-query expansions.
- [ ] 5.5 ADR follow-up: if the cypher mode survives to production with non-trivial usage, lift the gate decision into a new ADR (suggested ADR-008 "Raw Cypher exposure on the MCP surface — env-gated read-only by default").
