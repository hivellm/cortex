# 17. ADR-017 — OpenCode adapter via TS plugin + shared Rust daemon

**Status**: proposed
**Date**: 2026-05-26
**Related Tasks**: phase11w_opencode-adapter

## Context

Cortex was bound to Claude Code as its sole agent host. The user runs analyses inside OpenCode where none of Cortex was reachable (no envelope capture, no pre-thinking injection, no MCP tools, no slash commands, no agent ports). OpenCode's plugin model is fundamentally different from Claude Code's hook subprocess model: plugins are long-running TS modules loaded into OpenCode's Bun runtime that subscribe to lifecycle events. Three of the five Cortex surfaces (MCP, custom commands, agents) port via config-only edits; hooks and pre-thinking injection require a plugin layer that talks to the existing adapter daemon.</context>
<parameter name="decision">Ship `@hivellm/cortex-opencode-plugin` (TypeScript, in `packages/cortex-opencode-plugin/`) that subscribes to OpenCode lifecycle events and POSTs HookFrame JSON to the existing `cortex-adapter-claude-code` daemon over a new HTTP listener (`IpcBinding::Http(addr)` reading `CORTEX_ADAPTER_HTTP_BIND`, default `127.0.0.1:17004`). The daemon's dispatcher / sync_paths / publisher / WAL are reused verbatim. Add `tool = "opencode"` to the envelope schema enum + matching `TOOL_OPENCODE` Rust const. Port `.claude/commands/` and `.claude/agents/` to `.opencode/commands/` and `.opencode/agents/` mechanically. Pre-thinking injection uses `tui.prompt.append` from inside the `message.updated` handler for the user prompt (Path A from the Phase 0 spike).

## Decision

_No decision recorded._

## Alternatives Considered

- Re-author the adapter as a separate Rust binary speaking OpenCode's plugin protocol — rejected because Bun is OpenCode's runtime and there is no Rust-side plugin host. The cost-benefit on a second binary is poor when the existing daemon's transport layer is the only thing that needs extending.
- Skip the plugin entirely and use OpenCode's `command`-style hooks (shell-out per event) — rejected because command hooks cannot inject pre-thinking context via `tui.prompt.append`; the plugin path is mandatory for the injection requirement.
- Rename `cortex-adapter-claude-code` to `cortex-adapter` with feature flags — deferred to a follow-up rename task; the additive HTTP listener is a smaller patch that does not break the existing crate name in CHANGELOG history.

## Consequences

Pros: zero Rust changes for envelope capture; daemon stays the single source of truth for the dispatcher, the publisher, the WAL, the redactor, the scope deriver, the pre-thinking client, and the law-check client. The HTTP listener is loopback-only by default so no new attack surface. Plugin failures fail-open (empty bundle, "ask" verdict) so the session never breaks. Cons: the TS plugin couples to Bun runtime + `@opencode-ai/plugin` API stability — a major-version bump on the plugin runtime may need a follow-up. The plugin runtime version is pinned in `package.json` as a peerDependency. The spike answers (event ordering, `tui.prompt.append` semantics, `permission.asked` deny capability, `session.idle` per-subagent firing) are derived from the public plugin contract and confirmed by the §10 operator-led end-to-end smoke; a behaviour drift surfaces as a WARN log via runtime feature-detection rather than a hard failure.
