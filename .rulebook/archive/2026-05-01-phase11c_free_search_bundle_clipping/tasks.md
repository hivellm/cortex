## 1. Confirm the gap
- [x] 1.1 Traced — `/v1/query` handler `handle_query` (`crates/cortex-api/src/http.rs:558`) → `service.handle_with_headers` → `Orchestrator::run`; no byte-budget clipping ran on any intent (the cortex-pre-thinking clipper sits downstream). Spec dispatch in `crates/cortex-api/src/strategies.rs:90`. `free_search` strategy at `crates/cortex-api/src/strategies.rs:383-407`.
- [x] 1.2 Reproduced via in-memory service-level test instead of a live daemon call: `free_search_response_clipped_to_caller_budget_bytes` in `crates/cortex-api/src/service.rs` builds a 30-snippet × 800-byte fat response and asserts the post-clip serialised JSON fits under a caller-supplied 4 KiB budget. The proposal §Why captures the original 180,567-char production transcript from 2026-04-30 as the canonical evidence.

## 2. Route free_search through the section-cap budget
- [x] 2.1 New module `crates/cortex-api/src/budget.rs` with `clip_response_to_budget(&mut QueryResponse, budget_bytes: usize) -> ClipReport`. Wired into `service::handle_resolved` (after redaction + notices, before cache write) so every intent — including `free_search` — gets clipped. Default budget `DEFAULT_BUDGET_BYTES = 32 * 1024`.
- [x] 2.2 Per-snippet `text` clipped to `SNIPPET_TEXT_CAP = 1024` (UTF-8 boundary safe). Per-decision rationale clipped to 512 B. Per-similar-turn summary clipped to 384 B.
- [x] 2.3 Tail-drop loop pops entries (graph_neighbors → similar_turns → violations → decisions → snippets) and counts each removal in the `ClipReport` attached to `response.clipped`. The structured report replaces the spec's "marker line" — JSON callers branch on the counts; markdown renderers (cortex-pre-thinking formatter) translate the counts into "<!-- N more results clipped -->" if needed.

## 3. Expose `budget_bytes` on the MCP `cortex_query` tool
- [x] 3.1 Added optional `budget_bytes` to `crates/cortex-api/src/mcp.rs::input_schema` (default `32768`, min `1024`, max `262144`).
- [x] 3.2 Plumbing: `QueryRequest.budget_bytes: Option<usize>` (`crates/cortex-api/src/types.rs:104-110`). MCP server adapter uses `serde_json::from_value::<QueryRequest>` so the field rides through verbatim.
- [x] 3.3 `/v1/query` honours `budget_bytes` for every intent — the clipper sits in `service.rs::handle_resolved` and runs unconditionally after the orchestrator builds the response.

## 4. MCP server-side overflow guard
- [x] 4.1 In `crates/cortex-mcp-server/src/tools.rs::QueryTool::call`, after `serde_json::to_value(&parsed)` we measure the serialised payload bytes.
- [x] 4.2 When `payload_bytes > MCP_RESPONSE_HARD_CAP` (= 48 KiB, sized above the daemon-side 32 KiB default plus margin for the JSON-RPC envelope), the adapter returns `ToolResult::soft_error(reasons::BUDGET_EXCEEDED, …, { "total_hits", "payload_bytes", "transport_cap_bytes", "suggested_budget_bytes" })`. Verified by `tools_call_query_returns_budget_exceeded_when_response_is_too_large` in `crates/cortex-mcp-server/tests/end_to_end.rs`.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` request schema gains `budget_bytes` (with the per-section trim ladder explained) and the response example shows the new `clipped` field.
- [x] 5.2 Write tests covering the new behavior — 8 budget-module unit tests, 1 service-level pipeline test (`free_search_response_clipped_to_caller_budget_bytes`), 2 MCP schema/serde tests (`input_schema_exposes_budget_bytes_for_phase11c`, `budget_bytes_round_trips_through_query_request_serde`), 1 MCP end-to-end overflow test (`tools_call_query_returns_budget_exceeded_when_response_is_too_large`).
- [x] 5.3 Run tests and confirm they pass — cortex-api 299/299 lib + 30/30 http IT, cortex-pre-thinking 9/9 pipeline IT, cortex-mcp-server 13/13 end_to_end IT — all green.
