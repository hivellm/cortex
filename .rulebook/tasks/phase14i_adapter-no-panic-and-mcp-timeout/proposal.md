# Proposal: phase14i_adapter-no-panic-and-mcp-timeout

Source: `docs/analysis/rework/glm5.1/findings.md` (adapter panics HIGH, MCP timeout MEDIUM).

## Why

Two correctness gaps in the adapter + MCP layers:

1. `cortex-adapter-claude-code` carries `unwrap()` / `expect()` calls in production paths. Any envelope-construction edge case crashes the adapter daemon, which then takes the entire user session's hook capture down.
2. `cortex-mcp-server` falls back silently on tool-call timeout. The MCP host never sees a structured error; the model just gets a generic "no result" — indistinguishable from "tool returned empty".

Both are small, localised fixes that materially improve operator visibility.

## What Changes

- Replace every `unwrap()` / `expect()` in `cortex-adapter-claude-code/src/{dispatcher.rs,events.rs,ipc.rs}` with proper `Result` propagation. The dispatcher returns a structured error envelope that the adapter logs without crashing.
- In `cortex-mcp-server/src/tools.rs`, wrap each tool call with a timeout that, on expiry, returns `ToolError::Timeout { elapsed_ms, tool, request_id }` instead of falling through to a no-op.
- Both surfaces gain integration tests proving the failure mode is no-longer-silent.

## Impact

- Affected specs: `docs/specs/10-claude-code-adapter.md` § Error handling; `docs/specs/18-mcp-server.md` § Timeout contract.
- Affected code: `crates/cortex-adapter-claude-code/src/{dispatcher.rs,events.rs,ipc.rs}`, `crates/cortex-mcp-server/src/tools.rs`.
- Breaking change: NO at the wire format; structured errors replace silent no-ops.
- User benefit: adapter no longer crashes on edge cases; MCP timeouts are visible to the model + operator.
