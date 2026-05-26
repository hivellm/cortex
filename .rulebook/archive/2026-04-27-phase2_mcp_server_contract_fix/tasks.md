## 1. Tool name rename
- [x] 1.1 Rename `cortex.query` to `cortex_query` in `crates/cortex-mcp-server/src/tools.rs` (TOOL_NAME constants + descriptors)
- [x] 1.2 Rename `cortex.pre_thinking` to `cortex_pre_thinking`
- [x] 1.3 Rename `cortex.status` to `cortex_status`
- [x] 1.4 Update metrics dispatch tables in `metrics.rs` that key by tool name
- [x] 1.5 Update unit-test assertions that compare against the old strings

## 2. Schema field rename to camelCase
- [x] 2.1 In `crates/cortex-mcp-server/src/tools.rs`, every `"input_schema"` JSON key becomes `"inputSchema"`
- [x] 2.2 In `crates/cortex-mcp-server/src/tools.rs`, every `"output_schema"` becomes `"outputSchema"`
- [x] 2.3 In `crates/cortex-api/src/mcp.rs`, mirror the same rename
- [x] 2.4 Update tests that index into `d["input_schema"]` / `d["output_schema"]`

## 3. Build + rollout
- [x] 3.1 `cargo check -p cortex-mcp-server -p cortex-api` clean
- [x] 3.2 `cargo test -p cortex-mcp-server -p cortex-api` green
- [x] 3.3 `cargo build --release -p cortex-mcp-server`
- [x] 3.4 Stop running `cortex-mcp-server.exe` instances spawned by the IDE
- [x] 3.5 Replace `~/.cargo/bin/cortex-mcp-server.exe` with the freshly built binary
- [ ] 3.6 User runs `/clear` or restart so Claude Code respawns the server

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation (extend spec-18 with the MCP descriptor contract)
- [x] 4.2 Write tests covering the new behavior
- [x] 4.3 Run tests and confirm they pass
