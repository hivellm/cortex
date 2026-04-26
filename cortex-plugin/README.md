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

## Install

### Prerequisites

- `cortex-api` running on `http://127.0.0.1:15011` (the default).
- `cortex-mcp-server` on `PATH`. From this repo:
  ```bash
  cargo install --path crates/cortex-mcp-server
  ```
  Or grab a release artifact and drop it in `~/.local/bin` / `%USERPROFILE%\.cargo\bin`.
- (Recommended) `cortex-adapter-claude` installed for capture (spec 10).

### As a marketplace (recommended)

```text
/plugin marketplace add hivellm/cortex
/plugin install cortex@hivellm-cortex
```

### Local development

```bash
claude --plugin-dir ./cortex-plugin
```

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
└── commands/
    ├── cortex-status.md
    ├── cortex-query.md
    ├── cortex-laws.md
    ├── cortex-decisions.md
    ├── cortex-pre-thinking.md
    └── cortex-audit.md
```

## See also

- Spec 18 — full design: `docs/specs/18-claude-code-plugin.md`
- Spec 10 — capture-side hooks adapter
- Spec 11 — query API (the MCP tool's backend)
- Spec 12 — pre-thinking pipeline
