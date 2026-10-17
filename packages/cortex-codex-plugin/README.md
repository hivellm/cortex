# Cortex: Codex plugin

The Codex counterpart of `packages/cortex-claude-plugin`. It provides the
daily Cortex MCP tools, lifecycle hooks and the `cortex-memory` skill, with no
paths, tokens or environment blocks in `~/.codex/config.toml`.

## Install

Install the binaries and the user configuration first, exactly as for Claude
Code: `cortex-ops install-binaries --apply` and `cortex-ops setup --apply`
(see `docs/operations/daily-memory-runtime.md`). Then:

```text
codex plugin marketplace add <path-or-git-url-of-this-repository>
```

The repository marketplace is `.agents/plugins/marketplace.json`. Install
`cortex` from the plugin browser, then open `/hooks` and trust the Cortex
hooks. Codex runs plugin hooks only after you trust them.

## Contents

| File | Purpose |
|---|---|
| `.codex-plugin/plugin.json` | Manifest; points at `mcp.json`. |
| `mcp.json` | `cortex-mcp-server serve` over stdio, resolved from `PATH`. |
| `hooks/hooks.json` | Lifecycle hooks. `command` runs `scripts/cortex-hook-launcher` under `sh`; `commandWindows` calls `cortex-hook` directly. |
| `skills/cortex-memory/` | Save and recall decisions and lessons. |

## Supported events

`SessionStart`, `UserPromptSubmit` and `PreToolUse` wait for the reply.
`PostToolUse` and `Stop` are sent fire-and-forget. Codex also has `PreCompact`
and `SessionEnd`, but `cortex-hook` does not handle them, so the plugin does
not register them. Codex has no `SubagentStop` or `Notification` events.
Delivery of saved memory through Codex hooks is recorded as verified only
where the acceptance evidence for phase35 says so. Explicit
`cortex_memory_read` calls over MCP are the verified Codex path.

`commandWindows` needs `cortex-hook` on `PATH`. When the runtime is not
running, `cortex-hook` starts it, the same way it does for Claude Code.
