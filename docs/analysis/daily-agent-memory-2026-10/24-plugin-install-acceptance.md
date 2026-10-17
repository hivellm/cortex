# phase35 §1.13 — plugin install acceptance on this host

Date: 2026-10-17, about 18:55–19:01 UTC. Operator authorization was given
explicitly in the Claude session: "Authorize everything" for the binaries,
config and token, PATH, the plugin, stopping the runtime, and the migration.
Code under test is commit `6936eef`, with release binaries built from it.

## Host changes, in order

| Step | Result | Undo |
| --- | --- | --- |
| `cortex-ops install-binaries --from target/release --apply` | 4 binaries in `~/.cortex/bin` with `install-manifest.json` | delete `~/.cortex/bin` |
| `cortex-ops setup --from .cortex-state/daily-runtime/runtime.toml --apply` | `~/.cortex/config.toml` written; `~/.cortex/api-token` ACL is `BOLADO\Bolado:(F)` only; a second run reports `Unchanged` for both | delete both files |
| User `Path` (HKCU, `REG_EXPAND_SZ`) | `%USERPROFILE%\.cortex\bin` prepended. It now precedes `~/.cargo/bin`, which holds stale `cortex-*.exe` from 2026-04 to 2026-10-16 that had shadowed the install | previous value saved in the Claude session scratchpad (`pre-1.13/user-path.json`) |
| Claude marketplace | stale `hivellm-cortex` (`e:\HiveLLM\Cortex\cortex-plugin`, missing) removed; repository root added; `cortex@hivellm-cortex` 0.2.0 installed at user scope, enabled, both `userConfig` options left empty | `claude plugin uninstall`, then `marketplace remove` |
| Migration | dry-run manifest reviewed, then `--apply --expected-manifest`: 7 hooks in `settings.json`, `mcpServers.cortex` in `.claude.json`, 6 hooks in `.codex/hooks.json`, `mcp_servers.cortex` in `.codex/config.toml`. The marketplace row was already handled by the CLI | `--rollback ~/.cortex/migrations/clients-20261017T185812873931Z.json` |
| Pinned runtime | `bin/cortex-daily.ps1 stop` stopped api pid 97560 and adapter pid 110908; no Cortex runtime process remained | `cortex-daily.ps1 start …` (ops guide) |
| Codex plugin | marketplace `hivellm-cortex` added; `cortex@hivellm-cortex` 0.2.0 installed and enabled | `codex plugin remove` |

After the migration, no client file has a Cortex hook or MCP entry. Remaining
mentions are the plugin's own `enabledPlugins` and marketplace entries, a
permission rule and project paths. The repository `.mcp.json` has no Cortex
entry.

## Claude Code — fresh headless session

`claude -p` from the repository root, in an environment with this session's
`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_PLUGIN_ROOT` and
`CLAUDE_CONFIG_DIR` variables removed and `~/.cortex/bin` first on `PATH`.
Raw stream: `pre-1.13/accept-claude.jsonl` in the session scratchpad.

- **Runtime started by the plugin hook.**
  - `~/.cortex/lifecycle/last-spawn` = `ok` at 18:59:08.28.
  - `cortex-api` was ready at 08.63 and the adapter at 08.82.
  - Both processes run from `C:\Users\Bolado\.cortex\bin\`.
  - No start-failure notice was shown.
- **Seeded memory reached the model, with provenance.** The plugin's
  `SessionStart` hook returned `additionalContext` "## Local memory" with
  `adr039-daily-runtime` and `daily-value-priority`. Each carried its event
  ID, scope `repo:"Cortex"`, origin and source. `UserPromptSubmit` added
  `windows-claude-hook-pipe`. The model quoted all three titles and their
  provenance in its answer. The hook returned within its own bounded budget;
  a hook over budget emits `{}`.
- **Plugin MCP connected.**
  - `plugin:cortex:cortex` was `connected` and exposed 12 daily tools.
  - `cortex_status` returned `runtime_profile=daily` and
    `api_reachable=true`.
  - `daily_memory` was `ready`, with `delivery_consumption=not_observed`.
- **Doctor.** `cortex-ops doctor-install` with the new PATH order reported
  all four binaries `ok` (matches install manifest) and no double install.

## Codex — partial, by design

`codex exec --sandbox read-only` (codex-cli 0.162.0):

- The plugin MCP server `cortex` was listed and started, and the model saw
  `cortex_status`. The call was refused because headless `exec` uses approval
  policy `never`. An interactive session prompts for approval instead.
- The Cortex plugin hooks did not run. Codex runs plugin hooks only after the
  operator trusts them in `/hooks`, and this acceptance did not write trust
  state on the operator's behalf. The only `SessionStart` that ran belonged to
  another plugin.
- **Open, to check after trust.** codex-cli 0.162.0 exposes
  `CLAUDE_PLUGIN_ROOT`/`CLAUDE_PLUGIN_DATA` to hooks, while `PLUGIN_ROOT`
  appears only in its MCP schema. Whether `${PLUGIN_ROOT}` in
  `packages/cortex-codex-plugin/hooks/hooks.json` expands, and which shell
  runs `commandWindows`, must be observed in the first trusted session.

## Native memory

535/535 baseline files are unchanged. The scan shows one added file and one
changed file. Both are this Claude session's own auto-memory writes from
before the acceptance: `feedback_never_end_turn_while_waiting.md` and its
`MEMORY.md` index line. Cortex wrote no native file.

## Not changed

- The stale `~/.cargo/bin/cortex-*.exe` files remain. They are now behind
  `~/.cortex/bin` on PATH.
- Two `cortex-mcp-server.exe` processes from the old pinned directory still
  run. They belong to Claude and Codex sessions that started before the
  migration, and they end with those sessions.
- `.cortex-state/daily-runtime/` remains intact as the fallback.
