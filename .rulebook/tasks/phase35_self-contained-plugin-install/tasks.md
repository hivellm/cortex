## 1. Implementation
- [ ] 1.1 Config self-discovery: when CORTEX_CONFIG_FILE is unset, cortex-config loads the runtime config from ~/.cortex/config.toml (CORTEX_HOME overrides the directory). Document and test precedence CLI > env > CORTEX_CONFIG_FILE > user config > ./cortex.toml, keeping existing repo cortex.toml bootstrap behavior unchanged
- [ ] 1.2 Credential self-discovery: mcp-server, adapter and hook read the daily API token from a file under ~/.cortex with user-only ACL when CORTEX_API_TOKEN is unset; no client config ever carries a token; the token never appears in logs, Debug output or error bodies
- [ ] 1.3 Setup command as a new subcommand of the existing cortex-cli binaries (no new crate): writes ~/.cortex/config.toml and the credential, migrates values from .cortex-state/daily-runtime/runtime.toml, idempotent, dry-run by default, backups before any write
- [ ] 1.4 Release install of cortex-api, cortex-adapter-claude, cortex-hook and cortex-mcp-server to a stable user bin directory on PATH, with Windows locked-executable handling (rename-aside then copy), SHA-256 verification against the build, and a doctor check that flags a stale or shadowed PATH binary
- [ ] 1.5 App-managed lifecycle: on SessionStart and on the first synchronous event, when the adapter pipe is absent, cortex-hook spawns the daily runtime detached under a single-instance lock with a spawn backoff, never exceeding the hook budget; the systemMessage notice is shown only when the spawn fails; no OS autostart, Run key, scheduled task or Startup entry
- [ ] 1.6 Tests for 3.1: pipe absent then spawned, concurrent hooks racing the lock, spawn failure notice once per session, backoff honored, daemon already running untouched
- [ ] 1.7 Repository-root .claude-plugin/marketplace.json publishing packages/cortex-claude-plugin pinned to a tag; plugin.json declares userConfig for an absolute-only binary directory override and the API URL, with empty defaults that defer to ~/.cortex/config.toml
- [ ] 1.8 Plugin hooks.json covers every captured lifecycle event through a launcher that locates cortex-hook on PATH or known user directories, honors only an absolute override, works under Git Bash on Windows, keeps SessionStart and UserPromptSubmit synchronous, and fails open
- [ ] 1.9 Plugin-scoped MCP entry runs cortex-mcp-server serve with no env block and no path; trim plugin commands, agents and skills to the daily-profile tool surface so nothing references disabled routes
- [ ] 1.10 Duplicate guard: when legacy Cortex hooks remain in ~/.claude/settings.json, the plugin path skips events they already deliver, and doctor reports the double install
- [ ] 1.11 Codex plugin (.codex-plugin/plugin.json, mcp.json, hooks.json with commandWindows) mirroring 4.2-4.4; record honestly which Codex hook events are supported and that Codex requires hook trust
- [ ] 1.12 Reversible migration: after the plugin is installed and verified, remove Cortex hook and MCP entries from ~/.claude/settings.json, ~/.claude.json and ~/.codex/config.toml, and the stale hivellm-cortex marketplace registration; dry-run manifest, hash-verified backups, rollback command; retire scripts/activate_daily_memory_clients.py or reduce it to the migration
- [ ] 1.13 Acceptance on this host: no Cortex entry in any client config or repository .mcp.json; daemons stopped to simulate a reboot; a fresh Claude Code session starts the runtime through the plugin hook, receives a seeded active memory with provenance within the configured budget, and the plugin MCP tools connect; the same checks for Codex where supported; 535 native memory files unchanged
- [ ] 1.14 Update or create documentation covering the implementation (README install section, docs/operations/daily-memory-runtime.md, plugin READMEs, ADR for the lifecycle choice)
- [ ] 1.15 Write tests covering the new behavior (config and credential discovery, setup idempotency, installer, lifecycle spawn, launcher binary discovery, duplicate guard, migration and rollback)
- [ ] 1.16 Run tests and confirm they pass (fmt, check, clippy -D warnings, full workspace suite)

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
