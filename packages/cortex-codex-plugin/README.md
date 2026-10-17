# Cortex: Codex plugin

The Codex counterpart of `packages/cortex-claude-plugin`. It provides the
daily Cortex MCP tools, lifecycle hooks and the `cortex-memory` skill, with no
paths, tokens or environment blocks in `~/.codex/config.toml`. The contract is
[spec 46](../../docs/specs/46-codex-adapter.md).

## Install

Install the binaries and the user configuration first, exactly as for Claude
Code: `cortex install --from target/release --apply` and `cortex setup --apply`
(see `docs/operations/daily-memory-runtime.md`). The hooks need a
`cortex-hook` that understands `--client` (phase36 or later). An older binary
rejects the flag and answers `{}`, so Codex gets no memory until you reinstall.
Then:

```text
codex plugin marketplace add <path-or-git-url-of-this-repository>
```

The repository marketplace is `.agents/plugins/marketplace.json`. Install
`cortex` from the plugin browser.

### Trust the hooks

Codex runs plugin hooks only after you trust them. Open `/hooks` and trust the
Cortex hooks. The trust hash covers the hook declarations and the plugin
content, so **trust them again after every plugin update**. Codex caches each
installed version under `~/.codex/plugins/cache/<marketplace>/<plugin>/<version>/`,
which is why every plugin change bumps `version` in `.codex-plugin/plugin.json`.

`codex --dangerously-bypass-hook-trust` skips the trust check for a single
run. Use it only as a one-off operator check, never as the normal setup.

### Approve the MCP tools

Codex asks before each call to a plugin MCP tool. To approve the Cortex tools
up front, add this to `~/.codex/config.toml`:

```toml
[plugins."cortex@hivellm-cortex".mcp_servers.cortex]
default_tools_approval_mode = "approve"
```

## Contents

| File | Purpose |
|---|---|
| `.codex-plugin/plugin.json` | Manifest (version `0.3.0`); points at `mcp.json`. |
| `mcp.json` | `cortex-mcp-server serve` over stdio, resolved from `PATH`. |
| `hooks/hooks.json` | Lifecycle hooks. `command` runs `scripts/cortex-hook-launcher` under `sh`; `commandWindows` runs `scripts/cortex-hook-launcher.cmd`. |
| `scripts/cortex-hook-launcher` | POSIX launcher: finds `cortex-hook` on `PATH`, then in `~/.cortex/bin` and `~/.cargo/bin`. |
| `scripts/cortex-hook-launcher.cmd` | Windows launcher: finds `cortex-hook.exe` on `PATH` (never in the session's working directory), then in `%USERPROFILE%\.cortex\bin`. |
| `skills/cortex-memory/` | Save and recall decisions and lessons. |
| `.gitattributes` | Keeps `LF` for the sh launcher and `CRLF` for the `.cmd` launcher. |

## Supported events

Both launchers pass `--client codex`, so `cortex-hook` labels the session as
Codex. Each hook has a 5 second timeout. `cortex-hook` bounds its own I/O at
1.5 seconds, and every failure is fail-open.

| Codex event | Mode | What Cortex does | Reply |
|---|---|---|---|
| `SessionStart` | waits | Saved session memory and active work go into `hookSpecificOutput.additionalContext`. | Context, or `{}` |
| `UserPromptSubmit` | waits | Memory relevant to the prompt goes into `hookSpecificOutput.additionalContext`. | Context, or `{}` |
| `PostToolUse` | `--fire-forget` | Records the tool call (`tool_name`, `tool_input`, `tool_response`; `apply_patch` paths). | `{}` |
| `Stop` | `--fire-forget` | Records the turn's `last_assistant_message`. | `{}` |
| `SubagentStop` | `--fire-forget` | Records the sub-agent's `agent_type` and `last_assistant_message`. | `{}` |

Codex shows about 2,500 tokens of hook context to the model, so Cortex clips
Codex context to 6 KiB at a line boundary and ends it with a marker that
points at `cortex_memory_read`. `PreToolUse` is not registered: Cortex answers
it with `{}`, so it only added latency to every tool call. `PreCompact`,
`PostCompact`, `PermissionRequest`, `SubagentStart`, `Interrupt` and
`SessionEnd` are not used.

In the daily profile the runtime does not publish captured events, so the
live value is memory delivery on `SessionStart` and `UserPromptSubmit`. The
capture events run, return `{}`, and only reach storage in the full profile.

## Windows

Codex runs `commandWindows` as `%COMSPEC% /C "<command>"` after substituting
`${PLUGIN_ROOT}` into the text. The command is
`"${PLUGIN_ROOT}\scripts\cortex-hook-launcher.cmd" <Event>`. `cmd /C` strips
only the outer pair of quotes, so a plugin path with spaces still works. The
launcher passes stdin through to `cortex-hook.exe` unchanged. Without a binary,
or for an unknown event, it drains stdin, prints `{}` and exits 0.
