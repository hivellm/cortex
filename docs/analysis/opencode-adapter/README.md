# OpenCode adapter for Cortex — analysis

> **Goal**: make Cortex work inside OpenCode the way it already works
> inside Claude Code (envelope capture + pre-thinking injection + MCP
> tools + custom commands + agents).
>
> **Verdict**: feasible without any cortex-core changes. Three of the
> five surfaces (MCP, commands, agents) are config-only ports.
> Hooks/pre-thinking require a new TS/JS plugin
> (`@hivellm/cortex-opencode-plugin`) that talks to the existing
> `cortex-adapter-claude-code` daemon over its HTTP/socket surface.
> Estimated effort: **4-6 days**.

---

## Documents

| File | Content |
|------|---------|
| [01-surface-comparison.md](./01-surface-comparison.md) | Side-by-side: Claude Code vs OpenCode extension surfaces (hooks, MCP, commands, agents, plugins, SDK). |
| [02-cortex-touchpoints.md](./02-cortex-touchpoints.md) | What Cortex currently uses in Claude Code (hook scripts, adapter daemon, MCP server, commands, agents, skills). |
| [03-implementation-plan.md](./03-implementation-plan.md) | Phased plan: plugin + daemon refactor + config files + tests. |

---

## TL;DR — what changes vs Claude Code

| Surface | Claude Code | OpenCode | Action |
|---------|-------------|----------|--------|
| **MCP server** | `.mcp.json` with stdio transport | `opencode.json` `mcp.<name>.type=local` | Re-emit config; no code change |
| **Hooks** (capture + inject) | Bash/PowerShell scripts shelling out to local daemon socket | TS/JS plugin in `.opencode/plugins/` subscribing to lifecycle events | New plugin package |
| **Custom commands** | `.claude/commands/*.md` (loose schema) | `.opencode/commands/*.md` (frontmatter: `template`, `agent`, `subtask`, `model`) | Port + rewrite frontmatter |
| **Agents** | `.claude/agents/*.md` (loose schema) | `.opencode/agents/*.md` or JSON `agent` key (frontmatter: model, temperature, permissions, prompt) | Port + rewrite frontmatter |
| **Skills** | `.claude/skills/` (Anthropic-specific) | No equivalent | Re-author as commands or drop |
| **Pre-thinking injection** | Hook returns `additionalContext` field on UserPromptSubmit stdout | Plugin emits `tui.prompt.append` event or hooks `message.part.updated` | Plugin handles it |
| **Adapter daemon** | `cortex-adapter-claude-code` Rust binary, Unix-socket / named-pipe | Same daemon, called over HTTP `127.0.0.1` instead of socket | Add HTTP listener; no new binary |

---

## Why a TS plugin (not bash hooks)

OpenCode's plugin model is fundamentally different from Claude Code's:

- **Claude Code** spawns a fresh subprocess per hook event, passes JSON
  on stdin, reads JSON on stdout. Every hook is independent. Claude
  Code's `additionalContext` field on `UserPromptSubmit` lets a hook
  inject pre-thinking context.
- **OpenCode** loads plugins as long-running TS modules in its Bun
  runtime (`@opencode-ai/plugin`). Plugins subscribe to events
  (`tool.execute.before`, `session.created`, `message.updated`,
  `tui.prompt.append`, …) inside the same process. There is no
  `additionalContext` field — context injection has to flow through
  `tui.prompt.append` or by mutating the message via SDK.

OpenCode does support `command`-style hooks too (run shell commands on
events), but the plugin path is richer and is what the docs prefer for
non-trivial integrations. Cortex needs:

1. Reading session state (session_id, cwd, prompt) → plugin context has
   `project`, `directory`, `worktree`, `client`.
2. Posting envelopes to Synap — easy from a plugin (just `fetch()`).
3. Injecting pre-thinking context before the model sees the prompt —
   requires the `tui.prompt.append` API or `tool.execute.before`
   interception.
4. Running synchronously enough that the bundle lands before the LLM
   call.

A bash-only solution can capture telemetry but cannot inject context.
The plugin is mandatory.

---

## What is reused unchanged

- **`cortex-mcp-server`** — already speaks MCP over stdio. OpenCode
  configures it the same way Claude Code does, just under the
  `opencode.json` `mcp` key instead of `.mcp.json`.
- **`cortex-adapter-claude-code` daemon** — the IPC server, envelope
  publisher, WAL, redaction, scope derivation, pre-thinking client,
  and law-check client are all caller-agnostic. Only the wire format
  on the socket needs to accept OpenCode's hook payload shape.
  Renaming the crate to `cortex-adapter` (with `claude-code` /
  `opencode` features) is the cleanest long-term, but a same-binary +
  new HTTP listener is a smaller patch.
- **`cortex-pre-thinking`** — bundle assembly is unchanged.
- **`cortex-api`** `/v1/query` and `/v1/laws/check` — unchanged.

---

## Open questions (call out before implementing)

1. **Does OpenCode's plugin runtime block on a long event handler?**
   If `tool.execute.before` returning a Promise blocks the tool call
   until resolved, pre-thinking can run inline (good). If it doesn't,
   we need a different injection point. Evidence from docs is
   ambiguous — verify with a smoke test before committing to the
   plugin design.
2. **`tui.prompt.append` semantics**: does it append to the current
   user prompt (so the LLM sees it as part of the user's message) or
   does it append to a buffer that's sent next turn? This determines
   whether pre-thinking injection feels native or feels like a stale
   sidebar.
3. **Permission flow**: Claude Code has hook-driven `PreToolUse` deny.
   OpenCode has `permission.asked` / `permission.replied` events. Does
   replying to `permission.asked` from a plugin actually deny the tool
   call, or only record the decision? This affects whether Cortex's
   law-check can block tools the way it does today.
4. **Sub-agent equivalence**: Claude Code's `SubagentStop` hook lets
   us close `AgentCall` envelopes. OpenCode's `session.idle` /
   `session.updated` may or may not fire per sub-agent — needs
   verification.

---

## Recommended attack order

1. **Phase 1 — MCP + commands + agents (1 day, config-only)**: get
   `cortex_query`, `cortex_status`, `cortex_pre_thinking` callable
   from inside an OpenCode session. Port the highest-value commands
   and agents.
2. **Phase 2 — Plugin skeleton + envelope capture (2 days)**: TS
   plugin subscribes to `session.created`, `message.updated`,
   `tool.execute.before`, `tool.execute.after`, `session.idle`.
   Publishes envelopes to the daemon. No injection yet.
3. **Phase 3 — Pre-thinking injection (1-2 days)**: resolve open
   question 2 above; wire `tui.prompt.append` (or fallback) to deliver
   the bundle.
4. **Phase 4 — Law-check + permission gate (1 day)**: wire
   `permission.asked` to the existing `/v1/laws/check` flow if
   semantics allow; otherwise ship advisory-only.
5. **Phase 5 — Hardening (1 day)**: feature parity test matrix,
   docs, install script.

Detailed task breakdown lives in
[03-implementation-plan.md](./03-implementation-plan.md).
