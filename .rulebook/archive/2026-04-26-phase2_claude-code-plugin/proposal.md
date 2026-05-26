# Proposal: phase2_claude-code-plugin

## Why

Phase 1 wired Cortex to Claude Code through hook scripts (spec 10). Hooks work for capture and synchronous pre-thinking, but they're invisible to the model — the assistant never *invokes* Cortex; the daemon just observes. Claude Code's first-class extensibility model (MCP servers, slash commands, skills, sub-agents) lets the model itself reach into Cortex: ask the query API on demand, surface the active laws, replay past analyses, audit a session. Without that surface, the assistant can't *use* what Cortex captures during a turn — only the human reading the dashboard can.

The MCP tool descriptor for `cortex.query` already exists (spec 11). What's missing is the runtime wiring that registers it with Claude Code so a `/cortex` slash command, a `cortex-historian` sub-agent, and a `cortex-context` skill all flow through the same backend. This task ships that runtime + the catalogue of plugin assets the install command drops into `~/.claude/`.

## What Changes

- New `cortex-plugin/` directory at the workspace root (text + JSON, language-agnostic per the Claude Code plugin reference) holding the manifest, MCP registration, marketplace listing, skills, sub-agents, and slash commands.
- New `cortex-mcp-server` Rust crate — the only code artifact. Single binary referenced from `cortex-plugin/.mcp.json`; speaks JSON-RPC 2.0 over stdio (MCP revision `2024-11-05`); exposes three tools backed by `cortex_api::QueryService` (`cortex.query`), `cortex_pre_thinking::pipeline::run` (`cortex.pre_thinking`), and a daemon health probe (`cortex.status`).
- Plugin assets shipped under `cortex-plugin/`:
  - **Slash commands** (`commands/cortex-*.md`): `/cortex-status`, `/cortex-query <q>`, `/cortex-laws`, `/cortex-decisions`, `/cortex-pre-thinking`, `/cortex-audit <turn_id>`.
  - **Skills** (`skills/cortex-*/SKILL.md`): `cortex-context` (pull pre-thinking bundle), `cortex-audit` (retrieve audit envelope for a turn), `cortex-laws` (list active laws in scope).
  - **Sub-agents** (`agents/cortex-*.md`): `cortex-historian` (decision lookup specialist), `cortex-lawkeeper` (compliance auditor), `cortex-context-curator` (pre-thinking + scope refinement).
  - **MCP server registration** (`.mcp.json`): wires Claude Code to spawn `cortex-mcp-server serve` with `CORTEX_API_URL` + `CORTEX_ADAPTER_SOCK` env vars.
- Distribution via `cortex-plugin/.claude-plugin/marketplace.json`: users install with `/plugin marketplace add hivellm/cortex` then `/plugin install cortex@hivellm-cortex` — orthogonal to the spec-10 hook adapter, which keeps capture working untouched.
- CI safety net: `cortex-mcp-server validate ./cortex-plugin` lints the asset tree (manifest fields, `.mcp.json` cortex entry, skill / agent / command frontmatter) and exits non-zero on a malformed plugin.
- Optional VS Code companion extension stub remains a Phase-3 follow-up.

## Impact

- **Affected specs:** new spec `docs/specs/18-claude-code-plugin.md` (drafted alongside this task); references spec 10 (capture) + spec 11 (query API) + spec 12 (pre-thinking).
- **Affected code:** new `cortex-plugin/` directory at the workspace root + new `cortex-mcp-server` Rust crate. The spec-10 adapter (`cortex-adapter-claude-code`) is untouched.
- **Breaking change:** NO — additive. Hooks keep working; the plugin installs separately via the Claude Code marketplace.
- **User benefit:** the assistant in a Claude Code session can surface relevant decisions, audit prior turns, and consult active laws on demand instead of relying solely on adapter-injected pre-thinking.

## Source

`docs/specs/18-claude-code-plugin.md` (to be drafted) · depends on specs 10 + 11 + 12 (all 🟢) · architecture §5.1 + §8.
