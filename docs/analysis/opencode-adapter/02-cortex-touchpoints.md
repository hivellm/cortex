# 02 — Cortex touchpoints in Claude Code today

What this project actually uses inside a Claude Code session, and
where each piece comes from. This is the surface that needs to be
ported.

---

## Inventory

| Touchpoint | Source | Purpose |
|------------|--------|---------|
| `.mcp.json` | repo root | Registers `cortex-mcp-server` (stdio) + `rulebook` MCP servers |
| `.claude/settings.json` | repo root | Hooks (4 surfaces), env, multi-agent flag |
| `.claude/hooks/*.{sh,ps1}` | 7 scripts | Local enforcement (no-deferred, no-shortcuts, mcp-for-tasks, terse mode, handoff, compact reinject) |
| `.claude/commands/*.md` | 14 files | Slash commands wrapping `rulebook_*` MCP tools |
| `.claude/agents/*.md` | various | Specialized subagents (`researcher`, `implementer`, `tester`, `architect`, `code-reviewer`, `cortex:*`, …) |
| `.claude/skills/` | various | Workflow automations (terse, handoff, loop, schedule) |
| `.claude/rules/*.md` | 12 files | Behavioral rules loaded on session start |
| `cortex-adapter-claude-code` binary | `crates/cortex-adapter-claude-code/` | Local daemon: receives hook events over Unix socket, publishes envelopes to Synap, runs sync-path pre-thinking and law-check |
| `cortex-mcp-server` binary | `crates/cortex-mcp-server/` | MCP server exposing `cortex_query`, `cortex_status`, `cortex_pre_thinking` to the model |
| `cortex-api` HTTP server | `crates/cortex-api/` | Backend: `/v1/query`, `/v1/laws/check`, `/v1/events/batch`, dashboard |
| `cortex-pre-thinking` library | `crates/cortex-pre-thinking/` | Bundle assembly (laws + decisions + similar turns + snippets) |

---

## Wire flow today (Claude Code)

```
┌─────────────────────────┐
│ Claude Code session     │
│                         │
│  hook event ───┐        │
│                ▼        │
│   .claude/hooks/X.sh    │
│                │        │
└────────────────┼────────┘
                 │ JSON over Unix socket / named pipe
                 ▼
┌────────────────────────────────────────────┐
│ cortex-adapter-claude-code (Rust daemon)  │
│  • dispatcher: HookKind → Envelope         │
│  • publisher: Synap /v1/events/batch       │
│  • sync_paths: pre-thinking + law-check    │
│  • redaction, scope derivation, WAL        │
└──────┬──────────────────┬───────────────────┘
       │                  │
       │ HTTP             │ HTTP
       ▼                  ▼
┌────────────┐    ┌──────────────────┐
│ Synap bus  │    │ cortex-api       │
│            │    │  /v1/query       │
│ events.raw │    │  /v1/laws/check  │
└────────────┘    └──────────────────┘

╔═══════════════════════════════════════════╗
║ Separate path (model-facing tools):       ║
║                                           ║
║  Claude Code MCP client                   ║
║       │ stdio                             ║
║       ▼                                   ║
║  cortex-mcp-server                        ║
║       │ HTTP                              ║
║       ▼                                   ║
║  cortex-api (same instance)               ║
╚═══════════════════════════════════════════╝
```

Two channels:

1. **Hook channel** (capture + injection): synchronous, per-event,
   shell-out to bash → IPC to daemon. The daemon publishes envelopes
   asynchronously (fire-and-forget, with WAL fallback) and returns
   the pre-thinking bundle synchronously when the hook needs to
   inject context.
2. **MCP channel** (tools the model invokes): standard MCP. The
   model calls `cortex_query` / `cortex_status` / `cortex_pre_thinking`
   via the MCP server, which proxies to `cortex-api`.

---

## What each hook does today

| Hook | What the adapter does |
|------|----------------------|
| `UserPromptSubmit` | Build `Turn` envelope (user side); run pre-thinking; return `additionalContext` |
| `PreToolUse` | Build `ToolCall` envelope (input); run law-check; return verdict (allow/block) |
| `PostToolUse` | Update `ToolCall` with output; route `agent_call` separately |
| `SubagentStop` | Close `AgentCall` envelope |
| `Stop` | Build `Turn` envelope (assistant side) |
| `SessionStart` | Telemetry only; no envelope |
| `Notification` | Telemetry only |

The seven shell scripts in `.claude/hooks/` are project-level
**enforcement** hooks (no-deferred, no-shortcuts, etc.) — those are
distinct from the adapter's own `crates/cortex-adapter-claude-code/hooks/`
shims. Both exist; both port differently.

---

## What needs to be ported (mapped to OpenCode surface)

| Touchpoint | OpenCode mapping | Effort |
|------------|------------------|--------|
| `.mcp.json` | `opencode.json` `mcp` key | trivial — re-emit |
| `.claude/hooks/*.sh` (project enforcement) | OpenCode `command` hooks (if supported) or plugin event handlers | medium — rewrite the no-shortcuts / no-deferred / mcp-for-tasks logic in TS |
| `crates/cortex-adapter-claude-code/hooks/*` (envelope shims) | TS plugin in `.opencode/plugins/` | medium — replace bash shell-out with plugin event listeners |
| `cortex-adapter-claude-code` daemon | Same binary; add HTTP listener; refactor IPC layer to be transport-agnostic | small — already has `serve` binding abstraction |
| `cortex-mcp-server` | unchanged | none |
| `.claude/commands/*.md` | `.opencode/commands/*.md` with `template:` frontmatter | small — sed-like transform |
| `.claude/agents/*.md` | `.opencode/agents/*.md` with permissions block | medium — rewrite frontmatter + classify primary vs subagent |
| `.claude/skills/` | drop or re-author as commands | varies — most are CC-specific |
| `.claude/rules/*.md` | OpenCode `instructions` key (if it accepts a rules dir) or inline-merge into agent prompts | small — research; AGENTS.md already covers most |

---

## What does NOT need to change

- `cortex-api` and `cortex-pre-thinking` — backend is caller-agnostic.
  The `tool` field on envelopes (currently `claude-code`) just needs to
  accept `opencode` as a new value (one-line change in
  `crates/cortex-core/schemas/envelope.schema.json`).
- `cortex-mcp-server` — MCP is a standard. Both clients speak it.
- Synap event bus, Vectorizer, Nexus, Meili, SQLite — all downstream
  of the adapter; no changes.

---

## Schema delta

Single field change in the envelope schema:

```diff
 // crates/cortex-core/schemas/envelope.schema.json
 "tool": {
   "enum": [
     "claude-code",
+    "opencode",
     ...
   ]
 }
```

And one Rust constant:

```diff
 // crates/cortex-adapter-claude-code/src/events.rs (or moved to a shared crate)
-pub const TOOL_CLAUDE_CODE: &str = "claude-code";
+pub const TOOL_CLAUDE_CODE: &str = "claude-code";
+pub const TOOL_OPENCODE: &str = "opencode";
```

If the adapter is split into a shared `cortex-adapter-core` crate,
both adapters share the dispatcher / publisher / WAL / redaction
logic and only differ in their hook frontends.
