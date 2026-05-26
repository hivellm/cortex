## 1. cortex_audit MCP tool
- [x] 1.1 Confirm or mount `GET /v1/audit/{query_id}` on `cortex-api` returning `{caller, intent, scope, lanes: [{name, hits, latency_ms, samples}], cache_hit, fail_open, generated_at}`
- [x] 1.2 Add `cortex_audit { query_id }` tool in `crates/cortex-mcp-server/src/tools/audit.rs`
- [x] 1.3 Schema: required `query_id: string` (ULID), optional `include_samples: bool` (default false to keep payloads small)
- [x] 1.4 Returns the JSON envelope verbatim; agent-side rendering is the caller's job

## 2. cortex_capture_memory MCP tool
- [x] 2.1 Add `cortex_capture_memory { kind, body, repo, topic?, severity? }` in `crates/cortex-mcp-server/src/tools/capture.rs`
- [x] 2.2 Schema: `kind ∈ {memory, knowledge, learning}` (the three operator-curated kinds), `body` ≤ 8 KiB, `repo` lowercase per phase10d, optional `topic` from the canonical taxonomy, `severity ∈ {info, notable}`
- [x] 2.3 POSTs to `/v1/ingest` with the canonical envelope, returns `{event_id, content_hash, indexed_at}`
- [x] 2.4 Reject when body exceeds 8 KiB with a structured error so the agent retries with a shorter body

## 3. cortex_session_replay MCP tool
- [x] 3.1 Add `cortex_session_replay { session_id, max_turns?, include_tool_calls? }` in `crates/cortex-mcp-server/src/tools/replay.rs`
- [x] 3.2 Wraps `GET /v1/dashboard/conversations/{session_id}` plus an optional `?include=tool_calls` join
- [x] 3.3 Returns `{session_id, started_at, ended_at, turns: [{role, occurred_at, summary, tool_calls?}]}`
- [x] 3.4 Default `max_turns=20`; cap at 200 so the bundle stays under context budget

## 4. Wiring + auth
- [x] 4.1 Register the three tools through the existing `cortex-mcp-server` tool registry
- [x] 4.2 Reuse the auth + rate-limit middleware the existing tools (`cortex_query`, `cortex_status`, `cortex_pre_thinking`) flow through
- [x] 4.3 Update the MCP manifest (`.mcp.json` template) so the tools are discoverable

## 5. Tests
- [x] 5.1 Unit tests for each tool against an in-memory `cortex-api` test double (no live HTTP)
- [x] 5.2 Integration test in `crates/cortex-mcp-server/tests/end_to_end.rs` exercising the full MCP→HTTP round-trip for all three tools
- [x] 5.3 Regression: capture a memory, immediately query for it via `cortex_query intent=free_search`, assert it surfaces

## 6. Spec / docs
- [x] 6.1 NEW `docs/specs/20-mcp-tool-surface.md` enumerating every Cortex MCP tool with `(name, schema, endpoint, principle)`
- [x] 6.2 Update `docs/specs/11-query-api.md` §audit with the `/v1/audit/{query_id}` contract
- [x] 6.3 Update `docs/specs/12-pre-thinking-injection.md` §captura referencing `cortex_capture_memory`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
