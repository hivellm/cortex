# Proposal: phase11w_opencode-adapter

Source: `docs/analysis/opencode-adapter/`

## Why

Cortex is currently bound to Claude Code as its sole agent host
(`crates/cortex-adapter-claude-code`). The user has begun running
analyses inside OpenCode (`opencode.ai`), where today none of Cortex
is reachable: no envelope capture, no pre-thinking injection, no MCP
tools surfaced inside the session, no slash commands, no agent
ports. Every minute spent in OpenCode is a minute Cortex doesn't
learn from.

The analysis at `docs/analysis/opencode-adapter/` documents that the
port is **structural**, not a rewrite: `cortex-mcp-server`,
`cortex-pre-thinking`, and `cortex-api` are caller-agnostic. Three of
the five surfaces (MCP, custom commands, agents) are config-only
ports. Hooks and pre-thinking injection require a new TS/JS plugin
(`@hivellm/cortex-opencode-plugin`) talking to the existing
`cortex-adapter-claude-code` daemon over a new HTTP listener — same
binary, same dispatcher, same publisher, new transport.

This task ships parity at every surface that exists today in Claude
Code so the user can switch hosts without losing institutional
memory or governance.

## What Changes

- Add a new `tool` enum value `"opencode"` to the envelope schema and
  to `cortex-adapter-claude-code` exports.
- Refactor the adapter daemon's IPC layer so the binding is
  transport-agnostic: keep Unix socket / named pipe; add an HTTP
  `POST /hook` listener for plugin posts. New env var
  `CORTEX_ADAPTER_HTTP_BIND` (default `127.0.0.1:17004`).
- Author a new TypeScript plugin package
  `packages/cortex-opencode-plugin/` (target: `@hivellm/cortex-opencode-plugin`
  npm) implementing the OpenCode `Plugin` interface
  (`@opencode-ai/plugin`). Plugin subscribes to the lifecycle events
  that map to today's hooks (`session.created`, `message.updated`,
  `tool.execute.{before,after}`, `permission.asked`, `session.idle`)
  and posts envelopes to the daemon's HTTP listener. Plugin also
  emits `tui.prompt.append` (or SDK fallback) for pre-thinking
  injection.
- Port `.claude/commands/*.md` → `.opencode/commands/*.md` rewriting
  frontmatter to OpenCode's required schema (`template:`,
  `description`, `agent`, `subtask`, `model`).
- Port `.claude/agents/*.md` → `.opencode/agents/*.md` rewriting
  frontmatter (model, temperature, top_p, max_steps, prompt,
  permission with `allow`/`ask`/`deny` glob patterns). Classify each
  as primary vs subagent.
- Generate `opencode.json` registering both `cortex` and `rulebook`
  MCP servers under the `mcp` key plus the new plugin under the
  `plugin` array.
- Add a Phase-0 spike doc capturing the answers to the 4 open
  questions from `docs/analysis/opencode-adapter/README.md` (event
  ordering, `tui.prompt.append` semantics, `permission.asked` deny
  capability, `session.idle` per-subagent firing).
- Author install/uninstall script `scripts/install-opencode.{sh,ps1}`.
- Tests: unit tests for event mapping, integration test for HTTP
  transport on the daemon, end-to-end smoke that publishes envelopes
  via the plugin to a fake Synap.
- Docs: `docs/specs/20-opencode-adapter.md` (new spec), update root
  README + `crates/cortex-adapter-claude-code/README.md`.
- ADR-016 — OpenCode adapter via TS plugin + shared Rust daemon.

## Impact

- **Affected specs**: `docs/specs/20-opencode-adapter.md` (new),
  `docs/specs/10-claude-code-adapter.md` (update tool-enum reference),
  envelope schema (`crates/cortex-core/schemas/envelope.schema.json`).
- **Affected code**:
  - `crates/cortex-core/schemas/envelope.schema.json` (one-line
    enum addition)
  - `crates/cortex-adapter-claude-code/src/{ipc.rs,events.rs,lib.rs}`
    (HTTP listener + new TOOL_OPENCODE constant)
  - `packages/cortex-opencode-plugin/` (new TS package)
  - `.opencode/{commands,agents,plugins}/` (config + ports)
  - `opencode.json` (new project config)
  - `scripts/install-opencode.{sh,ps1}` (new)
- **Breaking change**: NO. Additive across the board. Claude Code
  path unchanged.
- **User benefit**: Cortex envelope capture + pre-thinking injection
  + MCP tools + commands + agents work inside OpenCode the same way
  they work inside Claude Code. The user can switch hosts without
  losing institutional memory.
