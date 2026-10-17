# Cortex: Claude Code plugin

Daily Cortex memory for Claude Code. The plugin bundles the lifecycle hooks, a
plugin-scoped MCP server and the skills, so no client file needs a Cortex path,
token or environment block.

## What you get

| Surface | What it does |
|---|---|
| Hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, `Notification`) | Hand each event to the installed `cortex-hook`. The hook starts the local runtime when it is not running and returns saved memory context to the session. |
| MCP server `cortex` | Runs `cortex-mcp-server serve` from `PATH` with the daily tool surface: `cortex_memory_read`, `cortex_capture_memory`, `cortex_pre_thinking`, `cortex_query`, `cortex_status`, `cortex_audit`, `cortex_active_work`, `cortex_feedback_signals`, `cortex_feedback_record`, `cortex_acl_whoami`, `cortex_acl_grant`, `cortex_capabilities`. |
| Skills | `cortex-memory` (save or recall decisions), `cortex-context`, `cortex-audit`, `cortex-laws`. |
| Agents and commands | `cortex-historian`, `cortex-lawkeeper`, `cortex-context-curator`; `/cortex-status`, `/cortex-query`, `/cortex-laws`, `/cortex-decisions`, `/cortex-pre-thinking`, `/cortex-audit`. All of them use `cortex_query`, `cortex_pre_thinking` or `cortex_status`, which the daily runtime serves. |

## Install

1. Build and install the binaries into `~/.cortex/bin`, then put that directory
   on `PATH`:

   ```powershell
   cargo build --release --bin cortex-api --bin cortex-adapter-claude --bin cortex-hook --bin cortex-mcp-server --bin cortex
   target\release\cortex install --from target\release          # dry run
   target\release\cortex install --from target\release --apply
   ```

2. Write the user configuration and credential under `~/.cortex`. Run it as a
   dry run first; `--from` migrates an existing runtime TOML and moves its
   `dashboard.api_token` into `~/.cortex/api-token`:

   ```powershell
   cortex setup --from .cortex-state\daily-runtime\runtime.toml
   cortex setup --from .cortex-state\daily-runtime\runtime.toml --apply
   ```

3. Add the marketplace and install the plugin:

   ```text
   claude plugin marketplace add <path-or-git-url-of-this-repository>
   claude plugin install cortex@hivellm-cortex
   ```

   The marketplace lives at the repository root, in `.claude-plugin/marketplace.json`. To pin a
   release, add the marketplace at a tag (`https://github.com/hivellm/cortex.git#v<version>`).
   The plugin source is a path inside that checkout, so the tag pins it too.

4. Start a new session. The first `SessionStart` starts the runtime when it
   is not already running. Nothing is registered with the OS.

`cortex doctor` reports a binary that is missing from `PATH`,
shadowed by another `PATH` entry, or stale. It also reports a double install,
where the plugin is enabled while legacy Cortex hooks remain in
`~/.claude/settings.json`.

## Options

| Option | Default | Meaning |
|---|---|---|
| `bin_dir` | empty | Absolute directory holding `cortex-hook`, needed only when it is neither on `PATH` nor in `~/.cortex/bin`. A relative value is ignored, so a checkout cannot supply the binary. |
| `api_url` | empty | Cortex API URL. When empty, `dashboard.api_url` from `~/.cortex/config.toml` applies. |

## How the hooks behave

- Each `hooks/cortex-*.sh` script finds `cortex-hook` in this order: the
  absolute override (`CORTEX_HOOK_BIN`, then the `bin_dir` option), `PATH`,
  `~/.cortex/bin`, and finally the legacy `~/.cortex` location. Without a binary
  the script prints `{}` and exits 0.
- `SessionStart`, `UserPromptSubmit` and `PreToolUse` wait for the reply. The
  other events are sent fire-and-forget.
- When the runtime is not running, `cortex-hook` starts it and retries within
  its 1.5 s budget. A notice appears only when the start fails, at most once per
  session. `CORTEX_HOOK_AUTOSTART=0` turns automatic start off.
- If legacy Cortex hooks are still present in `~/.claude/settings.json`, the
  plugin skips those events so each one is captured once. Remove the legacy
  entries with `scripts/activate_daily_memory_clients.py`, which runs as a dry
  run unless you pass `--apply` and keeps backups for `--rollback`.

The same scripts are installed by `cortex-adapter-claude install`. A drift test
keeps the two copies byte-identical.

## Verifying the package

```bash
cortex-mcp-server validate ./packages/cortex-claude-plugin
```

## See also

- `docs/operations/daily-memory-runtime.md`: runtime, install and migration.
- `.rulebook/decisions/040-*`: why the runtime is app-managed, with no OS autostart.
- `packages/cortex-codex-plugin/`: the Codex counterpart.
