# Proposal: phase2_mcp_server_contract_fix

## Why

`cortex-mcp-server` connects successfully to Claude Code (the MCP panel shows `cortex ✓ Connected`) but no `mcp__cortex__*` tools appear in the deferred-tools registry. Verified on 2026-04-27 with multiple `ToolSearch` probes returning `No matching deferred tools found`. The handshake works (`initialize`); the `tools/list` response is silently dropped by the IDE.

Two contract violations against MCP 2024-11-05 explain the silent rejection:

1. **Tool names contain dots.** `cortex-mcp-server/src/tools.rs:236,332,466` declares `cortex.query`, `cortex.pre_thinking`, `cortex.status`. The MCP spec restricts tool names to `[a-zA-Z0-9_-]+`. The other working servers in this same Claude Code instance follow the convention: `rulebook_task_create`, `browser_click`, etc. Dots cause clients to reject the descriptor.

2. **Schema field uses snake_case.** `cortex-mcp-server/src/tools.rs:339,473` and `cortex-api/src/mcp.rs:54-55` emit `input_schema` / `output_schema`. The MCP spec uses `inputSchema` (camelCase). `outputSchema` is non-standard — most clients ignore the field. The IDE's tool-loader can't find the input schema, treats the tool as malformed, drops it.

Same anti-pattern as the recently-fixed `phase1_adapter_pre_thinking_contract_fix`: a sibling component re-declaring a wire shape that drifts from the canonical contract. `cortex-mcp-server` shipped with a custom-flavoured MCP descriptor instead of the spec's. The probe via stdio in 2026-04-27's audit confirmed the bytes the server emits — both bugs visible on the same line:

```json
{"tools":[{"name":"cortex.query","input_schema":{...}}]}
```

## What Changes

- Rename tool identifiers in `crates/cortex-mcp-server/src/tools.rs`:
  - `cortex.query` → `cortex_query`
  - `cortex.pre_thinking` → `cortex_pre_thinking`
  - `cortex.status` → `cortex_status`
- Replace every `input_schema` and `output_schema` JSON key with `inputSchema` / `outputSchema` (camelCase) in:
  - `crates/cortex-mcp-server/src/tools.rs` (3 descriptors + tests)
  - `crates/cortex-api/src/mcp.rs` (descriptor builder + tests)
- Decide on `outputSchema`: remove if not consumed (MCP spec only requires `inputSchema`); keep as `outputSchema` if downstream tooling expects it. Default plan: keep camelCase rename to preserve the existing surface but document that the field is non-standard.
- Update unit tests / asserts that compare against the old strings.
- Rebuild `cortex-mcp-server.exe` and replace the running copy at `~/.cargo/bin/`.

## Impact

- Affected specs: spec-18 (Claude Code plugin / MCP surface).
- Affected code:
  - `crates/cortex-mcp-server/src/tools.rs`
  - `crates/cortex-api/src/mcp.rs`
  - tests in both crates
- Breaking change: NO for end users (the MCP tools were never reachable). Internally yes — any caller that hard-coded `cortex.query` must use `cortex_query`. The `cortex-plugin/commands/cortex-query.md` slash command and any `cortex-pre-thinking` agent prompt that references the dotted name needs an audit pass.
- User benefit: `mcp__cortex__cortex_query`, `mcp__cortex__cortex_pre_thinking`, `mcp__cortex__cortex_status` actually become callable from the model, so Cortex retrieval can drive ad-hoc queries (decision lookup, similar problems, law check) that today only fire via the auto-injected `pre_change_context` hook.

## Source

2026-04-27 audit; MCP panel showed `cortex ✓ Connected` while `ToolSearch +cortex` consistently returned no matches. Source inspection of `tools.rs` and `mcp.rs` confirmed both contract violations.
