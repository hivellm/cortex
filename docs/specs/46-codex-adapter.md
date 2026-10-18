# 46 — Codex adapter (plugin + shared daemon)

> **Status:** 🟡 Implemented, live acceptance pending (phase36 §1.7) · **Owner:** Core team · **Depends on:** 10, 12, 18, 23

> **Historical scope:** This specification describes the shared-daemon adapter.
> The default standalone plugin now invokes `cortex hook` and `cortex mcp`
> directly; it does not require that daemon. Standalone installed acceptance
> passed after the Windows PowerShell launcher correction and per-hook trust
> renewal. See the [current runtime guide](../operations/local-first-memory.md)
> and [acceptance evidence](../analysis/local-first-core-2026-10/05-implementation-evidence.md).

## Goal

Give Codex CLI sessions Cortex memory and capture them under their own
identity. Codex CLI 0.162 has Claude-compatible lifecycle hooks with
`additionalContext` injection and a plugin system, so Codex reuses the
shared `cortex-adapter-claude` daemon and the `cortex-hook` shim. This spec
supersedes the Codex sections of [spec 17](17-additional-adapters.md), which
described a `beforeCommand`/`onPrompt` API that Codex does not have.

## Scope

**In:**

- `packages/cortex-codex-plugin` (`.codex-plugin/plugin.json`, `mcp.json`,
  `hooks/hooks.json`, a POSIX and a Windows launcher, the `cortex-memory`
  skill), listed in `.agents/plugins/marketplace.json`.
- Client identity on the hook frame (`client: "codex"`), set by
  `cortex-hook --client codex`.
- Codex payload mapping into canonical envelopes, and Codex-safe replies.

**Out:**

- A Codex-specific daemon or crate.
- `PreToolUse` enforcement. The legacy law check is not implemented, and
  Cortex answers `PreToolUse` with `{}`.
- `PreCompact`, `PostCompact`, `PermissionRequest`, `SubagentStart`,
  `Interrupt` and `SessionEnd`.
- Reading Codex transcripts. `transcript_path` is nullable and its format
  is not a stable contract.

## Inputs / Outputs

### Events

Codex runs each hook with the event JSON on stdin. Common fields:
`session_id`, `transcript_path` (nullable), `cwd`, `hook_event_name`,
`model`. Turn-scoped events add `turn_id`, and several add
`permission_mode`.

| Codex hook | Mode | Payload fields Cortex reads | Envelope `kind` | Reply |
|---|---|---|---|---|
| `SessionStart` | sync | `session_id`, `cwd`, `source` | none | `hookSpecificOutput {hookEventName: "SessionStart", additionalContext}` with session memory and active work, or `{}` |
| `UserPromptSubmit` | sync | `prompt`, `turn_id`, `model`, `permission_mode` | `turn` (`user_message`) | `hookSpecificOutput {hookEventName: "UserPromptSubmit", additionalContext}` with prompt memory, or `{}` |
| `PostToolUse` | `--fire-forget` | `tool_name`, `tool_use_id`, `tool_input`, `tool_response`, `turn_id` | `tool_call` | `{}` |
| `Stop` | `--fire-forget` | `last_assistant_message`, `turn_id` | `turn` (`assistant_message`) | `{}` |
| `SubagentStop` | `--fire-forget` | `agent_id`, `agent_type`, `last_assistant_message` | `agent_call` | `{}` |

Codex requires JSON on exit 0 for `Stop` and `SubagentStop`. `cortex-hook`
always prints one JSON object and exits 0, and with `--fire-forget` it
prints `{}`. Cortex never emits `ask`, `continue: false`, `suppressOutput`
or the legacy `approve`, which fail a Codex hook. `systemMessage` appears
only on `SessionStart`/`UserPromptSubmit`, once per session, when local
memory is unavailable. Codex shows it as a UI warning.

### Payload mapping (`client = codex`)

- **`tool_response`.** Codex documents no shape. An object is parsed like
  Claude's (`stdout`, `stderr`, `exit_code`, `truncated`, `cas_ref`,
  `size`). A string becomes `output.stdout`. Other JSON is serialized into
  `output.stdout`. `null` means no output. `outcome` is `error` only for a
  non-zero `exit_code`.
- **`tool_input`.** An object is kept as is. Any other value is kept as
  `{"input": <value>}`, because the canonical `input` is an object.
- **`touched[]`.** Only for `apply_patch`. The patch text (a bare string,
  or `command` / `patch` / `input` in the input object) is scanned for
  `*** Add File:` (`file_create`), `*** Update File:` (`file_write`),
  `*** Delete File:` (`file_delete`) and `*** Move to:` (`file_create`).
  Duplicates are dropped. Shell and MCP tools record no paths, the same rule
  as Claude's `Bash`.
- **`Stop`.** `assistant_message` is `last_assistant_message`. An empty or
  `null` value publishes the turn boundary with `assistant_message: null`.
- **`SubagentStop`.** `agent_type` comes from the payload, and `description`
  defaults to `agent call`. `result` is
  `{"last_assistant_message": <text>}`, because the schema requires an
  object.
- **Correlation.** The Codex plugin registers no `PreToolUse`, so a
  `PostToolUse` without a pre-call slot is normal. It is `orphan` only when
  no turn is open.
- **`context.extras.codex`.** Holds the shared correlation tags (`turn_id`
  `cx-turn-<ulid>`, `tool_call_id` `cx-tc-<ulid>`, `tool_use_id`,
  `orphan`) and the Codex-native fields that are present: `codex_turn_id`
  (the payload `turn_id`), `model`, `source`, `permission_mode` and
  `agent_id`.
- **Envelope.** `tool = "codex"` and `context.ide = "codex"`. `model` comes
  from the payload only; `CLAUDE_MODEL` is never consulted.

### Context budget

Codex shows about 2,500 tokens of `additionalContext` to the model. Longer
context becomes a preview plus a temp file. Memory bundles
(`max_bundle_kb`, 32 KiB by default) carry ULIDs, hashes and paths that
tokenize at roughly 2.5 to 3 bytes per token. For `client = codex`, the
dispatcher therefore clips `SessionStart` and `UserPromptSubmit` context to
**6 KiB** (`CODEX_CONTEXT_BUDGET`). It cuts at the last whole line and ends
with `<!-- cortex: memory clipped to fit the Codex hook context limit;
cortex_memory_read returns the rest -->`. Claude Code and OpenCode keep the
`max_bundle_kb` budget.

## Design

### Client identity

The frame's optional `client` field is specified in
[spec 10](10-claude-code-adapter.md) §Hook ↔ daemon protocol.
`cortex-hook --client codex` sets it. Without the flag,
`CORTEX_HOOK_ORIGIN=codex-plugin` (or `adapter.hook_origin`) selects
`codex`. That covers a 0.2.0 plugin running a newer binary. For
`client = codex`, `cortex-hook` ignores `CLAUDE_SESSION_ID`, so a Codex
started from a Claude Code terminal never adopts the parent's session. The
legacy-settings duplicate guard (`hook_origin = "plugin"`) also does not
apply. An unknown `--client` value fails open with `{}` before any I/O.

**Canonical tool id.** Live capture uses `codex`, the value in the
envelope schema enum and in `cortex_core::vocab::TOOL_IDS`. The Claude
archive classifier's dotted kinds (`turn.openai-codex`, …, in
`cortex-workers` `classifier_worker/kinds.rs`) are an archive-only
vocabulary that maps onto the canonical `kind`. They are not renamed, and
they never appear as an envelope `tool`.

### Plugin

`hooks/hooks.json` registers the five events in the table. Each hook has a
5 second `timeout`; `cortex-hook` bounds its own I/O at 1.5 s.

- `command` (Unix): `sh "${PLUGIN_ROOT}/scripts/cortex-hook-launcher" <Event>`.
- `commandWindows`: `"${PLUGIN_ROOT}\scripts\cortex-hook-launcher.cmd" <Event>`.

Both launchers share one contract. They allow only the five events (case
sensitive), find the binary, add `--fire-forget` for `PostToolUse`, `Stop`
and `SubagentStop`, pass `--client codex`, export
`CORTEX_HOOK_ORIGIN=codex-plugin`, and pass stdin through to `cortex-hook`
unchanged. When there is no binary or the event is unknown, they drain
stdin, print `{}` and exit 0. They print nothing besides `cortex-hook`'s
reply.

- **Unix discovery.** `command -v cortex-hook`, then
  `~/.cortex/bin/cortex-hook[.exe]`, then `~/.cargo/bin/cortex-hook`.
- **Windows discovery.** `where $PATH:cortex-hook.exe`, which searches
  `PATH` only and never the session's working directory, then
  `%USERPROFILE%\.cortex\bin\cortex-hook.exe`.

`--client` needs a `cortex-hook` from phase36 or later. An older binary
rejects the flag and answers `{}`.

**MCP.** The manifest points `mcpServers` at `./mcp.json`, which holds
`{"mcpServers":{"cortex":{"type":"stdio","command":"cortex-mcp-server","args":["serve"]}}}`.
That is the documented form, and the installed 0.2.0 cache copy was
observed to list and start this server.

**Versioning.** Codex caches each installed version under
`~/.codex/plugins/cache/<marketplace>/<plugin>/<version>/`. Every plugin
change bumps `version` (now `0.3.0`). The package `.gitattributes` pins
`LF` for the sh launcher and `CRLF` for the `.cmd`.

### Windows execution

Codex 0.162.0-alpha.17.2 runs `commandWindows` as `pwsh.exe -NoProfile -Command "<command>"` (see the correction below).

> **Correction (2026-10-17, revised).** The installed codex-cli 0.162.0-alpha.17.2
> runs `commandWindows` through `pwsh.exe -NoProfile -Command`, not
> `%COMSPEC% /C`. This was captured from real child-process command lines.
> A quoted script path followed by an argument is a PowerShell
> `ParserError` (exit 1), so the hook fails before the launcher runs.
> Commands must be valid in PowerShell and cmd alike, for example
> `cmd /d /s /c ""<launcher>.cmd" <Event>"`. The earlier claim that
> `${PLUGIN_ROOT}` is not substituted is withdrawn as unverified.
> Evidence is in `docs/analysis/local-first-core-2026-10/04-claude-coordination.md`.
> The cmd-based description below is the intended *inner* form only.

The full command line is
`cmd.exe /C ""<root>\scripts\cortex-hook-launcher.cmd" <Event>"`. It has
more than two quotes, so cmd strips only the first and last one, and the
quoted script path survives a `<root>` that contains spaces. Forward
slashes in `<root>` also work. `crates/cortex-mcp-server/tests/hook_launcher.rs`
(`windows_cmd_launcher`) runs every `commandWindows` this way, with a raw
command line, against a fake `cortex-hook.exe`.

Unsupported: `%` or `^` in the plugin path. cmd expands them inside the
`/C` string.

### Trust and approval

- **Hook trust.** Plugin hooks run only after the operator trusts them in
  Codex `/hooks`. The trust hash covers the hook declarations and the plugin
  content, so every plugin update needs trusting again.
  `codex --dangerously-bypass-hook-trust` is a one-off operator check only.
- **MCP approval.** Codex asks before each plugin MCP tool call. Operators
  can approve the Cortex tools in `~/.codex/config.toml`:

  ```toml
  [plugins."cortex@hivellm-cortex".mcp_servers.cortex]
  default_tools_approval_mode = "approve"
  ```

  Cortex documents this and never writes it.

### Daily profile

The daily profile runs the daemon with `DisabledPublisher`, so captured
envelopes are not published. The mapping above takes effect in the full
profile. The daily live value is memory injection on `SessionStart` and
`UserPromptSubmit`, plus turn identity for the memory read. The capture
hooks still run and reply `{}`.

## Acceptance criteria

- [x] A frame carries `client`. A missing value means `claude-code`, an
  unknown value replies `{}`, and `tool`, `context.ide`, the extras key and
  the tag prefixes follow the client. OpenCode frames are labelled
  `opencode`.
- [x] Codex fixtures for every registered event map as above:
  `tool_response` as an object, a string or other JSON; `apply_patch`
  paths; `Stop` from `last_assistant_message` with a null
  `transcript_path`; `SubagentStop`.
- [x] A Codex `UserPromptSubmit` gets `hookEventName` plus context clipped
  to 6 KiB. Codex `PostToolUse`/`Stop`/`SubagentStop` get `{}`.
- [x] The real `cortex-hook --client codex` delivers a frame with
  `client: "codex"` and ignores `CLAUDE_SESSION_ID`.
- [x] The sh and `.cmd` launchers pick `PATH` before the user bin
  directory, pass `--client codex` and `--fire-forget` per event, forward
  stdin, and fail open with `{}`. Every `commandWindows` runs under
  `cmd /C` with a spaced plugin root.
- [ ] Live, in Codex on an operator host: after trust, a fresh session
  receives Cortex memory with provenance on `SessionStart` and
  `UserPromptSubmit`, `PostToolUse`/`Stop` run without errors, and
  `cortex_memory_read` works once approved (phase36 §1.7).

## Open questions

1. **`tool_response` shapes.** Codex does not document them. Once live
   payloads are observed, decide whether shell output deserves structured
   parsing (exit code, wall time).
2. **Old binaries.** A 0.3.0 plugin with a pre-phase36 `cortex-hook` gets
   `{}` from every hook. `cortex doctor` could flag a binary that lacks
   `--client`.

## References

- [Spec 10](10-claude-code-adapter.md): shared daemon, frame protocol, `client` field.
- [Spec 12](12-pre-thinking-injection.md): memory bundles.
- [Spec 17](17-additional-adapters.md): superseded Codex sections.
- [Spec 23](23-opencode-adapter.md): OpenCode, which also reuses the daemon.
- Codex hooks: https://learn.chatgpt.com/docs/hooks. Codex plugins: https://developers.openai.com/codex/plugins/build. Source: `openai/codex` `codex-rs/hooks`.
- Rulebook task `phase36_codex-adapter`.
