# MCP server tool descriptors must match the spec contract — names without dots, schema fields in camelCase
**Source**: manual
**Date**: 2026-04-27
**Related Task**: phase2_mcp_server_contract_fix
**Tags**: mcp, cortex, spec-18, contract-bug, claude-code
Symptom: Claude Code's MCP panel showed `cortex ✓ Connected` for `cortex-mcp-server`, but no `mcp__cortex__*` tools surfaced to the model. Multiple `ToolSearch` probes (broad queries, exact `select:` lookups, every plausible name prefix) returned `No matching deferred tools found`.

Root cause: two contract violations against MCP 2024-11-05, both in the `tools/list` descriptor:

1. Tool names contained dots: `cortex.query`, `cortex.pre_thinking`, `cortex.status`. The MCP spec restricts names to `[a-zA-Z0-9_-]+`. The handshake (`initialize`) succeeds, but Claude Code silently drops tool descriptors that violate this rule — the connection appears healthy in the UI while the tool list is empty.

2. Schema field used snake_case `input_schema` / `output_schema`. The MCP spec uses camelCase `inputSchema`. Same silent-drop semantics: the loader can't find `inputSchema`, treats the descriptor as malformed, discards it.

Probe via stdio confirmed both bugs on a single line of the JSON-RPC response: `{"tools":[{"name":"cortex.query","input_schema":{...}}]}`. Same anti-pattern as `phase1_adapter_pre_thinking_contract_fix` (adapter sent `additional_context` snake_case where Claude Code expected `hookSpecificOutput.additionalContext`) — the project keeps re-inventing wire shapes that drift from the canonical spec.

Lesson: any code that emits MCP descriptors must either round-trip through a schema-validated builder or be locked by a unit test that asserts the camelCase keys are present and the snake_case keys are absent. Tests that only check "name field exists" miss this class of bug because the bytes look plausible.

How verified: edited the 5 source files (`cortex-mcp-server/src/{tools,metrics,server,lib}.rs`, `cortex-api/src/mcp.rs`); rebuilt with `cargo build --release -p cortex-mcp-server`; stopped 3 stale `cortex-mcp-server.exe` processes (PIDs 30940/66204/80428); copied the new binary into `~/.cargo/bin/`; ran a fresh stdio probe — `tools/list` now returns `cortex_query`, `cortex_pre_thinking`, `cortex_status` with `inputSchema` (camelCase). Plugin source and cache both updated to reference the new names. 52 + 33 unit tests green.

Pending: user `/clear` to recreate the deferred-tools registry inside Claude Code so `mcp__cortex__*` finally becomes callable in-session.