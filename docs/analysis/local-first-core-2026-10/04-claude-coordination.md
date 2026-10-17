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

## Lock resolution, 18:34 -03

- `.git/index.lock` was empty (0 bytes) and dated 17:29. It was unchanged
  for 65 minutes. The only `git.exe` running was short-lived and new. Claude
  removed the lock as stale.
- **Commit `8290023`.** Claude committed only its own paths, 33 files: §1.1–§1.6
  and §1.8–§1.9 plus this file. No `crates/cortex-local`, root
  `Cargo.toml`/`Cargo.lock` or backlog files were staged.
- **Gate.** §1.10, `clippy -D warnings` and `test --workspace --exclude
  cortex-local`, is running.
- **Next.** §1.7, the live Codex acceptance, after the operator trusts the
  plugin hooks in `/hooks`.

## Shared build config, 18:55 -03

Claude added `.cargo/config.toml` with
`[env] CORTEX_HOME = { value = "target/cortex-home-unset", relative = true }`.
The full suite had failed on this host:

- Since phase35 config discovery, `Config::load()` in `cortex-api` tests
  read the operator's installed `~/.cortex/config.toml`. Its daily profile
  and ACL settings caused 63 failures.
- The fix applies only to processes that cargo launches (`cargo test`,
  `cargo run`), and those now fall back to `./cortex.toml` or defaults.
- An explicit `CORTEX_HOME` still wins, because `[env]` does not override.
  Installed binaries are unaffected.

This also makes your `cortex-local` tests hermetic. If any of them relies on
the real `~/.cortex`, set `CORTEX_HOME` explicitly.

## Gate evidence, blockers and plugin hand-off, 2026-10-17 ~19:25 -03

**phase36_codex-adapter state.**

- **Done.** §1.1–§1.6 and §1.8–§1.10 are complete.
- **Commits.**
  - `8290023`: adapter, plugin 0.3.0 and spec 46.
  - `acdb73c`: the `.cargo` `[env]` `CORTEX_HOME` setting and the
    cortex-ops env lock.
  - `53638c8`: the hook stdio inheritance fix.
- **Gate.**
  - `clippy -D warnings` is clean on the workspace minus `cortex-local`.
  - Workspace tests: 4086 passed. The single failure was a pre-existing
    `CORTEX_HOME` env race, now fixed. `cortex-ops` passes 125/125 three
    times in a row.
  - OpenCode vitest 18/18, Python 10/10, `cortex-adapter-claude-code`
    148/148.
- **Live pre-check on this host.**
  - The installed 0.3.0 launcher ran through raw `cmd /C` with
    Codex-shaped payloads against the real runtime.
  - SessionStart returned memory with provenance in 0.18 s with the runtime
    up, and in 0.55 s when it had to start the runtime.
  - UserPromptSubmit returned 1,536 bytes in 0.19 s. Stop returned `{}`.
  - The hook log shows `client=codex`.
- **Bug found and fixed (`53638c8`).** `cortex-hook` spawned the runtime with
  handle inheritance on, so the daemons kept the hook's stdout pipe open, and
  Codex, which reads hook stdout to EOF, hung on any SessionStart that started
  the runtime. **Port this to local-first §1.5/§1.6:** any detached child that
  a hook launches on Windows must not inherit the hook's stdio. Clear
  `HANDLE_FLAG_INHERIT` on the std handles or use an explicit handle list.

**Blocker.** §1.7, the live Codex acceptance, needs the operator to trust the
`cortex@hivellm-cortex` 0.3.0 hooks in Codex `/hooks`. The operator has been
asked. The §2 tail stays unchecked until §1.7 closes (LAW-CORTEX-001).

**Ownership released.** Claude releases `packages/cortex-codex-plugin/**` to
Codex root for the local-first §1.6 packaging switch. Constraints to keep:

- Every change to plugin hooks or files changes Codex's trust hash. The
  operator must re-trust in `/hooks`, and §1.7 evidence gathered on 0.3.0
  will not carry over to a changed plugin.
- `commandWindows` runs under `%COMSPEC% /C` after textual `${PLUGIN_ROOT}`
  substitution. Keep the `.cmd` launcher pattern: PATH-only `where`, then the
  `%USERPROFILE%\.cortex\bin` fallback, then fail-open `{}`. Keep the
  package `.gitattributes`: CRLF for `.cmd`, LF for `sh`.
- Keep injected context within about 2,500 tokens (6 KiB clip). Stop and
  SubagentStop must reply JSON.
- `cortex update` refreshes the Codex plugin with
  `codex plugin add cortex@hivellm-cortex`. Bump `version` so the cache
  path changes.

Claude keeps ownership of the adapter crate and spec 46 until local-first
§1.6 supersedes them, and makes no core, Cargo or backlog edits.

## Test ownership hand-off and live-host alert, 2026-10-17 ~19:40 -03

**Granted.** Codex root now owns three files for the local-first §1.6 split.
Each one should point legacy tests at the retained legacy shims in
`crates/cortex-adapter-claude-code/hooks`, without skipping coverage:

- `crates/cortex-adapter-claude-code/tests/hook_delivery_it.rs`;
- `crates/cortex-adapter-claude-code/tests/daily_memory_lifecycle.rs`;
- the test in `src/legacy.rs` that reads the packaged `hooks.json`.

Claude will not edit these files, the `cortex-local` `plugin_wiring` tests or
`crates/cortex-mcp-server/tests/{hook_launcher,hook_drift}.rs` while you
reconcile. Adapter production code stays as committed.

**ALERT: the live host is already affected.**

- Claude Code on this host reads the plugin directly from the working tree
  (`claude plugin list`: `cortex@hivellm-cortex` 0.4.0, read from
  `E:\HiveLLM\Cortex\packages\cortex-claude-plugin`). Every new operator
  session loads the in-progress 0.4.0 package now.
- That package runs `cortex mcp` and `cortex hook --client claude-code`.
- The installed `~/.cortex/bin/cortex.exe` is the cortex-cli user binary
  (setup/install/doctor/update). It has no `mcp` or `hook` subcommand, and
  the workspace has since renamed that binary to `cortex-legacy`.
- Result: new sessions get no Cortex MCP tools, and hooks fail open with `{}`,
  so no memory is injected.
- **Needed from you.** Before or at your §1.6/§1.7, install the standalone
  `cortex` into `~/.cortex/bin`, or keep the live package wired to the
  installed binaries until then.
- **Also note.** The installed `cortex update` builds `--bin cortex` from the
  workspace. That now resolves to the cortex-local binary, while the update
  installs with the old five-binary list. Re-check `cortex update`
  before anyone runs it.
- Claude has not changed the host. The operator is being told.
