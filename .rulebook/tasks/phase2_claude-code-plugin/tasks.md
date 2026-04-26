## 1. Spec + workspace scaffold
- [x] 1.1 Draft `docs/specs/18-claude-code-plugin.md` (Goal, Scope in/out, Inputs/Outputs, Design, Acceptance criteria, Decisions, References) and add the row to `docs/specs/00-index.md`
- [x] 1.2 Workspace member `cortex-mcp-server` registered in `Cargo.toml`
- [x] 1.3 Cargo deps: `cortex-api` (path), `cortex-pre-thinking` (path), serde / serde_json / clap / tokio / tracing / async-trait / reqwest / thiserror; dev-deps `tempfile` + `wiremock`

## 2. Plugin directory at workspace root
- [x] 2.1 `cortex-plugin/.claude-plugin/plugin.json` manifest (name, description, version, author, homepage, repository, license)
- [x] 2.2 `cortex-plugin/.claude-plugin/marketplace.json` listing pointing at this repo's `cortex-plugin` path
- [x] 2.3 `cortex-plugin/.mcp.json` registers `cortex-mcp-server` (command, args, env)
- [x] 2.4 `cortex-plugin/README.md` covers install via marketplace + local dev + env vars + layout

## 3. Skills (`cortex-plugin/skills/`)
- [x] 3.1 `cortex-context/SKILL.md` — refreshes the system-prompt context bundle via `cortex.pre_thinking`
- [x] 3.2 `cortex-audit/SKILL.md` — pulls the audit envelope for a turn id via `cortex.query` `intent=free_search`
- [x] 3.3 `cortex-laws/SKILL.md` — surfaces active laws via `cortex.query` `intent=law_check`

## 4. Sub-agents (`cortex-plugin/agents/`)
- [x] 4.1 `cortex-historian.md` — specialised at `decision_lookup`; emphasises supersession chains + dates
- [x] 4.2 `cortex-lawkeeper.md` — runs `intent=law_check`; verdicts allow / warn / block
- [x] 4.3 `cortex-context-curator.md` — picks intent + scope and returns a focused bundle

## 5. Slash commands (`cortex-plugin/commands/`)
- [x] 5.1 `cortex-status.md` — `/cortex-status` invokes `cortex.status`
- [x] 5.2 `cortex-query.md` — `/cortex-query <text>` runs `cortex.query` `intent=free_search`
- [x] 5.3 `cortex-laws.md` — `/cortex-laws` runs `intent=law_check`
- [x] 5.4 `cortex-decisions.md` — `/cortex-decisions <topic>` runs `intent=decision_lookup`
- [x] 5.5 `cortex-pre-thinking.md` — manually triggers the pre-thinking bundle (debug aid)
- [x] 5.6 `cortex-audit.md` — `/cortex-audit <turn_id>` reads the `cortex.events.query_audit` stream

## 6. `cortex-mcp-server` Rust binary
- [x] 6.1 `src/rpc.rs` JSON-RPC 2.0 framing (Request / Response / Notification / RpcError, `MCP_PROTOCOL_VERSION = "2024-11-05"`)
- [x] 6.2 `src/server.rs` handshake + dispatch loop (`initialize`, `notifications/initialized`, `tools/list`, `tools/call`)
- [x] 6.3 `src/tools.rs` — `Tool` trait, registry, three concrete tools (`QueryTool`, `PreThinkingTool`, `StatusTool`)
- [x] 6.4 `src/transport_stdio.rs` newline-delimited JSON over stdin/stdout
- [x] 6.5 `src/validate.rs` lints `cortex-plugin/` (manifests, .mcp.json, skills/agents/commands frontmatter, README)
- [x] 6.6 `src/metrics.rs` per-tool counters (`invocations`, `errors`, `latency_sum_ms`) + handshake counter
- [x] 6.7 `src/main.rs` clap entry: `serve`, `validate <plugin-dir>`, `print-tool-descriptors`

## 7. Observability
- [x] 7.1 Per-tool counters: `cortex.plugin.tool.invocations{tool}`, `cortex.plugin.tool.latency_ms{tool}`, `cortex.plugin.tool.errors{tool}`
- [x] 7.2 Handshake counter: `cortex.plugin.session.handshakes`
- [x] 7.3 Structured tracing event per `notifications/initialized` and per dispatched tool invocation

## 8. Tail (mandatory)
- [x] 8.1 Update or create documentation — flip `docs/specs/18-claude-code-plugin.md` status to 🟢 + update the `docs/specs/00-index.md` row; ship `cortex-plugin/README.md` covering install / config / MCP wire protocol / slash-command catalogue / sub-agent prompts
- [x] 8.2 Write tests — lib unit tests for the JSON-RPC framing, server dispatch (handshake, tools/list, tools/call, malformed frame, notifications, missing name, unknown tool), tools (descriptor parity with `cortex_api::tool_descriptor`, soft-error envelopes, registry shape), stdio transport (newline framing, blank lines), validator (clean tree passes; missing manifest / mcp section / skill SKILL.md / agent frontmatter all fail). End-to-end tests over a wiremock'd `cortex-api` exercise `cortex.query` round-trip, `cortex.query` 4xx → spec-11 reason, `cortex.query` unreachable → `api_unreachable`, `cortex.status` with reachable + unreachable upstream, and `cortex.pre_thinking` returning the spec-12 bundle
- [x] 8.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p cortex-mcp-server` (32 tests pass), `cargo run -p cortex-mcp-server -- validate ./cortex-plugin` exits 0
