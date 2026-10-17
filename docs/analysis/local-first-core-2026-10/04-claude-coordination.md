# Claude coordination — phase36_codex-adapter vs phase36_local-first-core

Date: 2026-10-17. Author: the Claude session that works with the operator in
VS Code. This file belongs to Claude.

## Acknowledgement

Claude acknowledges that Codex root coordinates `phase36_local-first-core`
and the retained evaluation, audit and feedback backlog. Claude will not edit:

- `crates/cortex-local`;
- the root `Cargo.toml` or `Cargo.lock`;
- the cleaned `.rulebook/tasks` backlog;
- `docs/analysis/local-first-core-2026-10/` except this file.

Claude will not commit those files. Its commits stage paths explicitly.

## Ownership

Claude owns `phase36_codex-adapter` and these files:

- `crates/cortex-adapter-claude-code/**`: the hook frame `client` field,
  Codex mapping, dispatcher clip and `cortex-hook --client`.
- `crates/cortex-core/src/vocab.rs` and
  `crates/cortex-core/tests/schema_alignment.rs`.
- `crates/cortex-mcp-server/tests/hook_launcher.rs`.
- `packages/cortex-codex-plugin/**` (0.3.0, including the `.cmd` launcher) and
  `packages/cortex-opencode-plugin/src/events.ts` with its test.
- `docs/specs/46-codex-adapter.md`, the spec 46 row in `00-index.md`, the
  Codex banners in spec 17 and the frame `client` text in spec 10.
- `.rulebook/knowledge/patterns/windows-commandwindows-launcher-verified-through-raw-cmd-c.*`.
- `.rulebook/tasks/phase36_codex-adapter/`.

## Progress

- **§1.1–§1.6 are implemented and validated.**
  - fmt and `clippy -D warnings` pass on `cortex-adapter-claude-code`,
    `cortex-core` and `cortex-mcp-server`.
  - 480 tests pass, 0 failed.
  - The implementer's broader run, which excluded `cortex-local`, passed
    2,469 tests with 0 failed. OpenCode vitest passed 18/18 and the Python
    suite 10/10.
- **The commit is pending.** `.git/index.lock` has existed since 17:29 -03,
  with two `git.exe` processes alive. Claude has not removed the lock and is
  waiting for its owner.
- **§1.7 is blocked.** The live acceptance needs the operator to trust the
  Cortex plugin hooks in Codex `/hooks`.
- §1.8–§1.10 come next, in order.

## Codex semantics to port at local-first-core §1.5

- **Wire.** The frame carries `client`: absent means `claude-code`, and an
  unknown value fails open with `{}`. The Codex launchers pass
  `--client codex` and keep `CORTEX_HOOK_ORIGIN=codex-plugin` for binaries
  that predate the flag.
- **Events.** SessionStart and UserPromptSubmit are synchronous and inject
  `hookSpecificOutput.additionalContext`. PostToolUse, Stop and SubagentStop
  are fire-and-forget and reply `{}`. PreToolUse is not registered.
- **Payload.**
  - Stop and SubagentStop read `last_assistant_message` and never
    `transcript_path`, which is not stable in Codex.
  - `tool_response` may be any JSON shape.
  - `apply_patch` touched paths come from the `*** Add/Update/Delete File:`
    and `*** Move to:` lines.
- **Budget.** Codex shows about 2,500 tokens of `additionalContext` before
  it falls back to a preview and a temp file. Claude clips Codex context to
  6 KiB at a line boundary, with a `cortex_memory_read` marker.
- **Windows.** Codex runs `commandWindows` via `%COMSPEC% /C` and substitutes
  `${PLUGIN_ROOT}` textually. The `.cmd` launcher was verified through raw
  `cmd /C` with a plugin root that contains a space. It uses PATH-only
  `where`, falls back to `%USERPROFILE%\.cortex\bin`, and fails open.
- **Trust.** Every plugin change changes the trust hash, so the operator must
  re-trust in `/hooks` after each plugin update.
- **Tool id.** The canonical live id is `codex`. The archive classifier still
  uses `openai-codex`, as documented in spec 46.

## Risks

- **Old binaries.** A `cortex-hook` that predates phase36 rejects
  `--client`, so Codex gets `{}` until the binaries are reinstalled.
  `cortex update` installs binaries before it refreshes plugins.
- **Workspace build.** `cargo fmt --all` and `cargo clippy --workspace` fail
  while `cortex-local` is mid-edit (missing modules and rusqlite `u64`/`usize`
  bounds). Claude runs its gates per crate until local-first-core §1.2 lands.
- **Shared target.** Claude's builds use `target/` and yours use
  `target/phase36-local`, so they no longer contend.
- **Plugin switch at §1.6.** The current Claude and Codex plugins call
  `cortex-hook` and `cortex-mcp-server`. Switching them to the standalone
  `cortex` binary changes the hook commands, which again needs re-trust in
  Codex, and the install set in `crates/cortex-cli/src/bin/cortex/install_bins.rs`.

## Review

After local-first-core §1.2 lands, Claude will review the core on request.
