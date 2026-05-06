## 1. Adapter unwrap audit
- [ ] 1.1 `rg "\.unwrap\(\)|\.expect\(" crates/cortex-adapter-claude-code/src/` — list every production-path call site.
- [ ] 1.2 Replace each with `?` propagation or explicit `match` returning `AdapterError`.
- [ ] 1.3 New `AdapterError` enum covering: `MalformedHook`, `MissingField`, `IpcWriteFailed`, `EnvelopeBuildFailed`.
- [ ] 1.4 Dispatcher catches every `AdapterError`, logs `tracing::error!` with full context, and continues serving the next hook (no process crash).
- [ ] 1.5 Fuzz test: 100 random hook payloads, none should crash the dispatcher.

## 2. MCP timeout structured error
- [ ] 2.1 Wrap every tool's `call()` body with `tokio::time::timeout(MCP_TOOL_TIMEOUT, ...)`.
- [ ] 2.2 On timeout, return `ToolError::Timeout { elapsed_ms, tool, request_id }`. The MCP transport surfaces it as a structured error to the host.
- [ ] 2.3 Default `MCP_TOOL_TIMEOUT = 5_000` ms; per-tool overrides via `cortex_config::MCPConfig`.
- [ ] 2.4 Integration test: stub a tool that sleeps 10s, assert the MCP response carries `error.code = "tool_timeout"` within 5.5s.

## 3. Tail (mandatory)
- [ ] 3.1 Update `docs/specs/10-claude-code-adapter.md` + `docs/specs/18-mcp-server.md` + `CHANGELOG.md`.
- [ ] 3.2 Tests: §1.5 + §2.4 + grep CI gate forbidding new `unwrap()` in adapter prod paths.
- [ ] 3.3 `cargo check --workspace && cargo clippy -p cortex-adapter-claude-code -p cortex-mcp-server -- -D warnings && cargo test -p cortex-adapter-claude-code -p cortex-mcp-server` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
