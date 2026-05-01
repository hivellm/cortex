## 1. Confirm the gap
- [ ] 1.1 Trace the `/v1/query` handler for `intent = free_search` and identify whether it runs through `cortex_pre_thinking::pipeline::run` / `formatter.rs` clipping
- [ ] 1.2 Reproduce the overflow locally (a `free_search` call with a high-recall query) and capture the response size

## 2. Route free_search through the section-cap budget
- [ ] 2.1 Make the `free_search` branch invoke the same formatter the other intents use, with a default `budget_bytes = 24 * 1024`
- [ ] 2.2 Per-snippet text clipped to ~1 KiB inside the bundle (reuse the existing clip helper)
- [ ] 2.3 Hit-count cap: when more hits than the byte budget allows, truncate and append `<!-- N more results clipped -->`

## 3. Expose `budget_bytes` on the MCP `cortex_query` tool
- [ ] 3.1 Add optional `budget_bytes` (integer, default 32768) to the tool's input schema in `crates/cortex-mcp-server/src/tools.rs`
- [ ] 3.2 Plumb it into the `/v1/query` body
- [ ] 3.3 `/v1/query` honours `budget_bytes` for every intent (not just `free_search`)

## 4. MCP server-side overflow guard
- [ ] 4.1 In the MCP `cortex_query` adapter, after building the response, count its serialised byte length
- [ ] 4.2 When it exceeds the configured cap (or the transport's hard limit), return a structured `BudgetExceeded { hits_returned, total_hits, suggested_budget_bytes }` instead of letting the transport reject the call

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
