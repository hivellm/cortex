## 1. cortex-api proxy module (`crates/cortex-api/src/search_proxy.rs`)

- [x] 1.1 `crates/cortex-api/src/search_proxy.rs` ships the three handlers and the request/response wire types (`KeywordSearchRequest/Response`, `VectorSearchRequest/Response`, `GraphQueryRequest/Response` + `GraphNode` + `GraphEdge`). All types serde-derived; the module reaches the live clients via direct `reqwest` POSTs (mirrors the `VectorizerLane` SDK-bypass pattern) plus `nexus_sdk::NexusClient` from `ApiState`.
- [x] 1.2 `handle_vector_search` accepts exactly one of `query_vector` (raw f32) / `query_text` per phase11v's wire schema; v1 declines `query_text` with HTTP 400 + reason `not_implemented` until the embedder bin exposes a server-side embed accessor. POSTs to `{vectorizer}/collections/{name}/search/text`, surfaces upstream-latency, applies `score_threshold`. URL resolution walks `CORTEX_VECTORIZER_URL` → `CORTEX_EMBEDDER_VECTORIZER_URL` → `VECTORIZER_URL`.
- [x] 1.3 `handle_keyword_search` forwards `{ index, q, limit, filter, sort, attributes_to_retrieve }` to `{meili}/indexes/{uid}/search`. Raw Meili documents pass through. Unknown index → HTTP 404 + reason `index_not_found`. Bearer pulls from `CORTEX_FULLTEXT_MEILI_KEY` / `_API_KEY` / `MEILI_MASTER_KEY` in priority order.
- [x] 1.4 `handle_graph_query` dispatches on `mode`. `neighbors` walks 1..=5 hops with optional `edge_kinds` filter. `cypher` is gated on `CORTEX_GRAPH_CYPHER_ENABLED=1`; otherwise HTTP 403 + reason `cypher_disabled`. Both modes return `{ mode, nodes, edges }` with deduplication on `node_id` and `(from, to, kind)` respectively.
- [x] 1.5 `ok_capped()` clamps every successful response at `MCP_RESPONSE_HARD_CAP = 30 KB`; overflow returns HTTP 413 + `{ error: "budget_exceeded", payload_bytes, transport_cap_bytes, suggested_limit }`. Pinned by `vector_clamp_k_caps_at_max` and the cap-aware test cases.

## 2. cortex-api router wiring (`crates/cortex-api/src/http.rs`)

- [x] 2.1 `crates/cortex-api/src/http.rs` mounts `POST /v1/search/keyword`, `/v1/search/vector`, `/v1/search/graph` on the same router as `/v1/query`. Auth posture matches: no key required by default; `CORTEX_DASHBOARD_AUTH=1` only gates the dashboard sub-router.
- [x] 2.2 Direct env resolution inside `search_proxy` (instead of accessor methods on `ApiState`) keeps the new module self-contained while reusing the existing `nexus` handle on `ApiState`. The duplicated env walk costs ~20 lines and avoids invasive refactors of the lane factories.

## 3. MCP tool wrappers (`crates/cortex-mcp-server/src/tools.rs`)

- [x] 3.1 `KeywordSearchTool` registered (`name() = "cortex_keyword_search"`). Descriptor advertises `index` (required), `q`, `limit` (≤ 100), `filter`, `sort`, `attributes_to_retrieve`. `call()` POSTs through the shared `proxy_search` helper.
- [x] 3.2 `VectorSearchTool` registered (`name() = "cortex_vector_search"`). Descriptor advertises `collection` + `query_vector` (required), `k` (≤ 200), `score_threshold`.
- [x] 3.3 `GraphQueryTool` registered (`name() = "cortex_graph_query"`). Descriptor advertises the two-mode discriminator with depth cap and the cypher-gate semantics.
- [x] 3.4 `ToolRegistry::default_set()` returns 10 tools (was 7). Pinned by `tools::tests::registry_returns_ten_tools_with_unique_names` and `server::tests::tools_list_returns_ten_descriptors`.

## 4. Spec + docs

- [x] 4.1 New [`docs/specs/22-fine-grained-search.md`](../../../docs/specs/22-fine-grained-search.md) documents all three endpoints with full request / response shapes, the 14-reason error taxonomy, the cypher-gate posture, and the 30 KB hard-cap contract. Cross-references spec 11 (fused query) and spec 18 (MCP tool surface).
- [x] 4.2 `CHANGELOG.md` `[Unreleased]` § Added carries a phase11v entry covering §1-§4 plus the cross-tree drift fix that phase 11x's `.ps1` retirement introduced (synced `cortex-plugin/hooks/` to the trimmed canonical tree so `plugin_hook_shims_match_adapter_canonical_sources` is green again).

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 5.1 Update or create documentation covering the implementation: spec 22 created (`docs/specs/22-fine-grained-search.md`) enumerating wire reasons + status codes per failure path; CHANGELOG `[Unreleased]` § Added carries the phase11v entry; ADR-009 records the cypher-gate posture.
- [x] 5.2 Write tests covering the new behavior: 9 unit tests in `cortex-api::search_proxy::tests` (request serde defaults × 3, k/limit clamps × 3, ok_capped budget cap × 2, cypher_enabled env read × 1); 9 wiremock tests in `cortex-mcp-server::tools::tests` covering each of the three new tools (happy path × 3, soft-error mapping × 3 — `index_not_found` / `bad_input` / `cypher_disabled`, transport / cypher / budget edge × 3); registry-size-10 assertions pinned at `cortex-mcp-server::tools::tests::registry_returns_ten_tools_with_unique_names` and `cortex-mcp-server::server::tools_list_returns_ten_descriptors`. Hook-drift drift guard re-greened by syncing `cortex-plugin/hooks/`.
- [x] 5.3 Run tests and confirm they pass: `cargo check --workspace` clean; `cargo test -p cortex-api --lib search_proxy` 9/9 green; `cargo test -p cortex-mcp-server` 60+/60+ green (no failures). Pre-existing `cortex-cli` and `cortex-workers` clippy warnings remain — out of scope for phase11v.
- [x] 5.4 Learning captured indirectly via the phase 11x learning entry (`Hook latency on Windows is bound by pwsh cold-start, not the daemon`), which documents the `cortex-plugin/hooks/` ↔ `cortex-adapter-claude-code/hooks/` drift guard that this task touched.
- [x] 5.5 ADR landed: [`.rulebook/decisions/009-cypher-gate-disabled-by-default-on-mcp-graph-surface.md`](../../decisions/009-cypher-gate-disabled-by-default-on-mcp-graph-surface.md) (status `proposed`, 2026-05-07). Captures the decision (env-gate `CORTEX_GRAPH_CYPHER_ENABLED`), the rejected alternatives (ungated, drop-cypher, statement allowlist, bearer-bound gate), and the bearer-bound migration earmark for the post-dashboard-auth ADR. The decision shape was already locked by §1.4 + spec 22; landing the ADR now eliminates the open follow-up.
