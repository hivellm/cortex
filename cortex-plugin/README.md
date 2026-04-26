# Cortex — Claude Code plugin

Brings the Cortex retrieval + governance stack into Claude Code sessions.
Adds three MCP tools, three skills, three sub-agents, and six slash commands
backed by the local Cortex daemon (`cortex-api` + `cortex-adapter-claude`).

## What you get

| Surface | Use it for |
|---|---|
| `cortex.query` (MCP tool) | Hybrid retrieval (vector + keyword + graph) over everything Cortex captured. |
| `cortex.pre_thinking` (MCP tool) | Refresh the system-prompt context bundle: laws + decisions + similar turns + snippets. |
| `cortex.status` (MCP tool) | Daemon health: pid, queue depth, recent publisher errors, overflow WAL bytes. |
| `cortex-context` (skill) | "Pull fresh context for what I'm about to do" — invokes pre-thinking. |
| `cortex-audit` (skill) | "Audit turn `<id>`" — fetches the audit envelope. |
| `cortex-laws` (skill) | "What laws apply here?" — surfaces active laws in scope. |
| `cortex-historian` (sub-agent) | Decision-lookup specialist. Pull supersession chains + dates. |
| `cortex-lawkeeper` (sub-agent) | Compliance auditor. Checks an action against active laws. |
| `cortex-context-curator` (sub-agent) | Picks intent + scope + returns a focused bundle. |
| `/cortex-status` | Daemon health. |
| `/cortex-query <q>` | Free-text search across Cortex. |
| `/cortex-laws` | Active laws in scope. |
| `/cortex-decisions <topic>` | Decision lookup. |
| `/cortex-pre-thinking` | Manually trigger a pre-thinking bundle (debug). |
| `/cortex-audit <turn_id>` | Audit envelope for a past turn. |
| Hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, `Notification`) | Push every Claude Code session event into the local `cortex-adapter-claude` daemon for indexing. |

## Install

### Prerequisites

- `cortex-api` running on `http://127.0.0.1:15011` (the default).
- `cortex-mcp-server` and `cortex-adapter-claude` on `PATH`. From this repo:
  ```bash
  cargo install --path crates/cortex-mcp-server
  cargo install --path crates/cortex-adapter-claude-code
  ```
  Or grab release artifacts and drop them in `~/.local/bin` / `%USERPROFILE%\.cargo\bin`.
- The capture daemon socket: `cortex-adapter-claude install --no-hooks` once, then run the daemon (`cortex-adapter-claude daemon`).
  - `--no-hooks` is important: the plugin's `hooks/hooks.json` already registers the same shim catalogue, and running both paths without it would fire each event twice. The flag keeps `~/.claude/settings.json` byte-identical to its pre-install state.
- `bash` on `PATH` so the plugin's hook shims (`cortex-*.sh`) execute. On Windows the Claude Code harness already provides Git Bash.
- On Windows: `pwsh` 7+ on `PATH`. The `.sh` shim detects `$OSTYPE` (`msys*` / `cygwin*` / `win32*`) and re-execs the sibling `.ps1` via `pwsh` because the daemon binds a Windows named pipe (`\\.\pipe\cortex-adapter-claude`) that Git Bash's `nc` / `socat` can't reach.

### As a marketplace (recommended)

```text
/plugin marketplace add hivellm/cortex
/plugin install cortex@hivellm-cortex
```

### Local development

```bash
claude --plugin-dir ./cortex-plugin
```

### Local marketplace (development) — known cache pitfall

When the marketplace source is `directory` (the local-dev path), the
Claude Code installer copies **only** the manifest files
(`plugin.json`, `marketplace.json`) into
`~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/`. Hooks /
skills / agents / commands stay invisible to the loader because the
runtime reads from the cache, not from the source directory.

Workaround: after `claude plugin install cortex@hivellm-cortex` (or
after editing the plugin assets in the source tree), run

```bash
bash cortex-plugin/scripts/sync-cache.sh
```

The script copies the missing assets (hooks, hooks.json, skills,
agents, commands, README, .claude-plugin, .mcp.json) into the cache.
Restart Claude Code so the loader picks up the new files —
`~/.claude/settings.json` should then list `cortex-*` hook entries
under the `hooks` key.

### Migrating from the spec-10 standalone install

If you previously ran `cortex-adapter-claude install` (without `--no-hooks`) and are now switching to the plugin path:

```bash
cortex-adapter-claude uninstall          # restores ~/.claude/settings.json byte-identical to pre-install
claude plugin install cortex@hivellm-cortex
cortex-adapter-claude install --no-hooks # bring the daemon back up; leaves settings.json alone
```

The plugin's `hooks/hooks.json` is now the single source of hook firing.

## Configuration

The plugin's `.mcp.json` carries two env vars the MCP server reads:

| Var | Default | Purpose |
|---|---|---|
| `CORTEX_API_URL` | `http://127.0.0.1:15011` | `cortex-api` base URL |
| `CORTEX_ADAPTER_SOCK` | `~/.cortex/adapter-claude.sock` | Adapter UDS for `cortex.status` |

Override per-machine by editing `.mcp.json` in your local plugin install.

## Verifying the install

```bash
cortex-mcp-server validate ./cortex-plugin
```

Exits 0 on a clean tree. Non-zero if any required asset is missing or
malformed (a missing skill `SKILL.md`, a corrupt agent frontmatter, a
broken `.mcp.json`).

## Layout

```
cortex-plugin/
├── .claude-plugin/
│   ├── plugin.json
│   └── marketplace.json
├── README.md
├── .mcp.json
├── skills/
│   ├── cortex-context/SKILL.md
│   ├── cortex-audit/SKILL.md
│   └── cortex-laws/SKILL.md
├── agents/
│   ├── cortex-historian.md
│   ├── cortex-lawkeeper.md
│   └── cortex-context-curator.md
├── commands/
│   ├── cortex-status.md
│   ├── cortex-query.md
│   ├── cortex-laws.md
│   ├── cortex-decisions.md
│   ├── cortex-pre-thinking.md
│   └── cortex-audit.md
└── hooks/
    ├── hooks.json
    ├── cortex-session-start.{sh,ps1}
    ├── cortex-user-prompt.{sh,ps1}
    ├── cortex-pre-tool.{sh,ps1}
    ├── cortex-post-tool.{sh,ps1}
    ├── cortex-stop.{sh,ps1}
    ├── cortex-subagent-stop.{sh,ps1}
    └── cortex-notification.{sh,ps1}
```

## See also

- Spec 18 — full design: `docs/specs/18-claude-code-plugin.md`
- Spec 10 — capture-side hooks adapter
- Spec 11 — query API (the MCP tool's backend)
- Spec 12 — pre-thinking pipeline
