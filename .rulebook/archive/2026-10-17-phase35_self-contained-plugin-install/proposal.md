# Proposal: phase35_self-contained-plugin-install

## Why
Cortex activation writes host-specific absolute paths (pinned binaries under .cortex-state/daily-runtime/bin, CORTEX_CONFIG_FILE, pipe flags) into ~/.claude/settings.json, ~/.claude.json, ~/.codex/config.toml and, until f91181e, the versioned repository .mcp.json. The existing plugin package (packages/cortex-claude-plugin) is stale: its MCP targets the old full stack and its marketplace is registered at a nonexistent path, so it is disabled. Daemons do not come back after a reboot, which caused the July-October 2026 capture outage (analysis 05 F-030). akitaonrails/ai-memory solves the same problem with one PATH binary that owns its config and credential, plus a marketplace plugin that carries hooks, a plugin-scoped MCP entry and skills, so no client file holds a path or token. Cortex should follow the same principle.

## What Changes
Make Cortex installable as a self-contained Claude Code and Codex plugin that needs no project .mcp.json and no hand-written hook or MCP entries. Binaries discover their own config and credential under ~/.cortex; release binaries live on PATH; cortex-hook starts the daily runtime on demand when the adapter pipe is absent (app-managed lifecycle, no OS autostart, per ADR-039 and the AGENTS.override Codex rules); the repository root publishes a marketplace whose plugin bundles hooks, a plugin-scoped MCP entry and skills for the daily surface; a reversible migration removes the existing absolute-path entries. Reference design: akitaonrails/ai-memory plugins/claude-code and plugins/codex (pinned tag v3.1.0).

## Impact
- Affected specs: new `plugin-install` delta (this task); `.rulebook/specs/daily-memory/spec.md` lifecycle wording; `docs/specs/10-claude-code-adapter.md`.
- Affected code: `crates/cortex-config` (config and credential discovery), `crates/cortex-cli` (setup and install subcommands), `crates/cortex-adapter-claude-code` (`cortex-hook` on-demand start, duplicate guard), `crates/cortex-mcp-server` (env-free start), `packages/cortex-claude-plugin`, a new Codex plugin package, root `.claude-plugin/marketplace.json`, `scripts/activate_daily_memory_clients.py`, `bin/cortex-daily.ps1`.
- Breaking change: NO. Explicit `CORTEX_CONFIG_FILE` and environment variables keep working, and migration is opt-in and reversible.
- User benefit: install with two `claude plugin` commands, no host paths or tokens in any client or repository file, and the runtime comes back on the first session after a reboot without OS autostart.

## Decisions
- **Lifecycle:** app-managed start from `cortex-hook`. The HKCU Run key used by ai-memory is excluded, because the project rules forbid OS schedules and ADR-039 selected an app-managed lifecycle. Reopening it needs an explicit operator decision.
- **No new crate:** setup and install go into the existing `cortex-cli` binaries.
- **Reference:** akitaonrails/ai-memory, tag v3.1.0: `plugins/claude-code/{.claude-plugin/plugin.json,.mcp.json,hooks/hooks.json,scripts/ai-memory-hook}` and `plugins/codex/`.
