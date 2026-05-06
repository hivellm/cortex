# 01 — Surface comparison: Claude Code vs OpenCode

Side-by-side reference. Source: `https://opencode.ai/docs/{config,plugins,mcp-servers,sdk,agents,commands}` (fetched 2026-05-06).

---

## Configuration files

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Project config | `.claude/settings.json` | `opencode.json` (or `.jsonc`) at project root |
| User config | `~/.claude/settings.json` | `~/.config/opencode/opencode.json` |
| Project extension dir | `.claude/{commands,agents,hooks,rules,skills}/` | `.opencode/{commands,agents,plugins}/` |
| User extension dir | `~/.claude/{commands,agents,…}/` | `~/.config/opencode/{commands,agents,plugins}/` |
| Merge precedence | Project overrides user | Multi-layer merge: managed → MDM → project → `.opencode/` → inline → custom path → global → remote (later wins for conflicts) |

---

## MCP servers

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Config file | `.mcp.json` (project) or `~/.claude/settings.json` `mcpServers` | `opencode.json` `mcp` key |
| stdio transport | `{ "command": "...", "args": [...], "env": {...} }` | `{ "type": "local", "command": [...], "environment": {...}, "enabled": true }` |
| HTTP/SSE transport | `{ "type": "sse", "url": "..." }` | `{ "type": "remote", "url": "...", "headers": { "X": "{env:VAR}" }, "enabled": true }` |
| Per-tool gating | Not native; via tools allowlist in settings | `tools` section with glob patterns + per-agent overrides |
| Auth (remote) | n/a | Optional OAuth on remote type |

**Cortex impact**: `cortex-mcp-server` (stdio) ships as-is. Just change
config file format.

### Equivalence example

```jsonc
// Claude Code (.mcp.json today)
{
  "mcpServers": {
    "cortex": {
      "type": "stdio",
      "command": "cortex-mcp-server",
      "args": ["serve"],
      "env": {
        "CORTEX_API_URL": "http://127.0.0.1:17000",
        "CORTEX_ADAPTER_SOCK": "~/.cortex/adapter-claude.sock"
      }
    }
  }
}
```

```jsonc
// OpenCode (opencode.json)
{
  "mcp": {
    "cortex": {
      "type": "local",
      "command": ["cortex-mcp-server", "serve"],
      "enabled": true,
      "environment": {
        "CORTEX_API_URL": "http://127.0.0.1:17000",
        "CORTEX_ADAPTER_SOCK": "~/.cortex/adapter-opencode.sock"
      }
    }
  }
}
```

---

## Hooks / Lifecycle events

### Claude Code hooks (shell-out model)

| Event | Trigger | stdin | stdout |
|-------|---------|-------|--------|
| `SessionStart` | Session boot or after `/clear` | `{}` | optional `{ additionalContext: "..." }` |
| `UserPromptSubmit` | User submits a prompt | `{ prompt, session_id, cwd }` | optional `{ additionalContext: "..." }` to inject context |
| `PreToolUse` | Before tool invocation | `{ tool_name, tool_input, … }` | `{}` (or `{ decision: "block", reason: "..." }` to deny) |
| `PostToolUse` | After tool invocation | `{ tool_name, tool_input, tool_response, … }` | `{}` |
| `SubagentStop` | Subagent finishes | `{ subagent_type, output, … }` | `{}` |
| `Stop` | End of assistant turn | `{ messages, … }` | `{}` |
| `Notification` | Permission prompts, idle warnings | varies | `{}` |

Configured under `hooks` key in `.claude/settings.json`. Schema is
camelCase. Each hook spawns a fresh subprocess.

### OpenCode lifecycle events (plugin model)

Plugin events are TypeScript callbacks inside a long-running module.
Documented event names:

| Category | Events |
|----------|--------|
| Command | `command.executed` |
| File | `file.edited`, `file.watcher.updated` |
| Installation | `installation.updated` |
| LSP | `lsp.client.diagnostics`, `lsp.updated` |
| Message | `message.part.removed`, `message.part.updated`, `message.removed`, `message.updated` |
| Permission | `permission.asked`, `permission.replied` |
| Server | `server.connected` |
| Session | `session.created`, `session.compacted`, `session.deleted`, `session.diff`, `session.error`, `session.idle`, `session.status`, `session.updated` |
| Tool | `tool.execute.before`, `tool.execute.after` |
| Shell | `shell.env` |
| TUI | `tui.prompt.append`, `tui.command.execute`, `tui.toast.show` |
| Todo | `todo.updated` |

### Mapping Claude → OpenCode

| Claude hook | OpenCode equivalent |
|-------------|---------------------|
| `SessionStart` | `session.created` |
| `UserPromptSubmit` | `message.updated` (with role=user) — or intercept via `tool.execute.before` of the model invocation, depending on internals |
| `PreToolUse` | `tool.execute.before` |
| `PostToolUse` | `tool.execute.after` |
| `SubagentStop` | partially `session.idle` — needs verification |
| `Stop` | `session.idle` (turn-end) |
| `Notification` | `permission.asked` / `tui.toast.show` |

---

## Pre-thinking injection (the hard part)

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Mechanism | Return `{ "additionalContext": "..." }` from `UserPromptSubmit` hook stdout | No `additionalContext` field. Options: (a) emit `tui.prompt.append` from the plugin; (b) intercept `message.updated` and rewrite via SDK; (c) send a separate priming message via `client.session.prompt(...)` |
| Latency budget | Hook runs synchronously before model call (~1500 ms cap in our adapter) | Plugin events run inside Bun runtime — likely synchronous on the same event loop, but verify whether the hook awaits before model call |
| Failure mode | Hook stdout empty or non-JSON → Claude Code ignores it (fail-open) | Plugin throwing → unclear; needs verification. Default: catch all, swallow, fail-open |

**Decision needed**: which OpenCode injection mechanism feels native.
The current top candidate is `tui.prompt.append` because the docs list
it as a TUI integration explicitly designed for plugins. Fallback:
plain `client.session.prompt` with the bundle as a system-prefixed
text part.

---

## Custom commands

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Location | `.claude/commands/*.md` or `~/.claude/commands/*.md` | `.opencode/commands/*.md` or `~/.config/opencode/commands/*.md` |
| Naming | filename = command (`/foo` ← `foo.md`) | same |
| Frontmatter | Loose / mostly free-form | Required `template:`; optional `description`, `agent`, `subtask`, `model` |
| Body | The prompt / instructions | The body is unused if `template` is set; otherwise the body is the prompt |
| Args | `$ARGUMENTS` | `$ARGUMENTS` or `$1`, `$2`, … (positional) |
| Shell injection | n/a | `` !`<command>` `` runs shell, output inlined |
| File reference | n/a | `@filename` includes file content |
| Override built-ins | partial | Yes — can override `/init`, `/undo`, `/redo` |

**Cortex impact**: re-author the existing `.claude/commands/*.md`
into `.opencode/commands/*.md` with explicit frontmatter. Most are
trivial — set `template:` to the existing body content.

---

## Agents

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Location | `.claude/agents/*.md` | `.opencode/agents/*.md` or `agent` key in `opencode.json` |
| Categories | Single class | "Primary" (Tab-cycle main agents) vs "Subagents" (invoked via `@name` mention) |
| Built-ins | `general-purpose`, `Explore`, `Plan`, … | `Build`, `Plan` (primary); `General`, `Explore` (subagent) |
| Frontmatter | model, description, allowed tools | model, temperature, top_p, max_steps, prompt (`{file:./prompt.md}` ref), permissions (`allow`/`ask`/`deny` per tool category, glob support) |
| Permission glob | n/a | e.g. `"git *": "ask"`, `"rm *": "deny"` |
| Multiple primaries | n/a | Tab-cycle through them |

**Cortex impact**: the project's specialized agents
(`code-reviewer`, `researcher`, `implementer`, `tester`,
`architect`, …) port over by translating frontmatter and possibly
splitting "main" vs "subagent" classification. Most are subagents.

---

## Plugins (OpenCode-only)

```typescript
import type { Plugin } from "@opencode-ai/plugin"

export const MyPlugin: Plugin = async ({
  project, client, $, directory, worktree
}) => {
  return {
    "tool.execute.before": async (input) => { /* ... */ },
    "session.created": async (event) => { /* ... */ },
    // ... any of the lifecycle event names above
  }
}
```

Context handles:
- `project` — project metadata
- `client` — full OpenCode SDK client (HTTP-backed)
- `$` — Bun shell API
- `directory` — cwd
- `worktree` — git worktree

Plugins live in `.opencode/plugins/*.ts` (auto-loaded) or are
declared as npm packages in `opencode.json` `plugin` array.

Custom tools can also be authored in plugins via the `tool()` helper
with Zod schemas — useful if Cortex wants to expose tools that
operate on the live OpenCode session state without going through MCP.

---

## SDK

| Concern | Claude Code | OpenCode |
|---------|-------------|----------|
| Language | n/a (CLI only) | TypeScript (`@opencode-ai/sdk`) |
| Embedded | n/a | `createOpencode()` spawns server + client |
| Remote | n/a | `createOpencodeClient({ baseUrl: "http://localhost:4096" })` over HTTP |
| Capabilities | n/a | sessions, messages, files, events SSE stream, TUI control, structured output (JSON schema) |
| External use | n/a | Yes — third-party tools can drive an OpenCode instance |

**Implication for Cortex**: a future "Cortex Studio" UI could embed
OpenCode and surface envelopes / pre-thinking bundles as a sidebar
without going through any plugin — straight SDK calls. Out of scope
for the initial adapter.

---

## Skills

Claude Code has `.claude/skills/` (Anthropic SDK feature). OpenCode has
no equivalent. The closest match is custom commands. The Cortex
project currently ships several skills (`update-config`,
`fewer-permission-prompts`, `loop`, `schedule`, …) — most of these are
Claude Code-specific. Either drop in OpenCode or re-author the few
that make sense (e.g. `/loop` → OpenCode command).
