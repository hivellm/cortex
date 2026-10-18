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

## Claude review of the standalone install, 2026-10-17 ~20:00 -03

**Verified on the host.**

- **Binary.** `~/.cortex/bin/cortex.exe` has SHA-256 `795585ea…` and
  `build-info` runtime=standalone, release=true, required_services=[].
- **Claude Code.** `cortex@hivellm-cortex-local` 0.4.0 is enabled and reads
  from `~/.cortex/plugins/cortex-standalone/claude`, not from the checkout.
  The old `cortex@hivellm-cortex` is disabled.
- **Codex.** Same setup. The old 0.3.0 plugin is disabled.
- **Fresh headless `claude -p` session.**
  - `plugin:cortex:cortex` connected, with 11 tools.
  - `cortex_status` answered without HTTP.
  - The SessionStart hooks exited 0 and replied `{}`.
  - The store grew from 2 to 4 records, so hook capture works.

**Concerns, in priority order.**

1. **Recall regressed for the operator right now.** The standalone store holds
   0 memories. Before the switch, every session received 2–3 cited memories
   (adr039-daily-runtime, daily-value-priority, windows-claude-hook-pipe)
   from the legacy daily-memory store. Until your §1.7 imports them, every
   new Claude Code and Codex session runs without Cortex memory. That is the
   project's success criterion ("agents recall accepted decisions"). Please
   import the accepted daily-memory records first, before bulk history, and
   confirm with a fresh session that they are injected with provenance.
2. **Codex trust.** The new Codex plugin needs a fresh `/hooks` trust from
   the operator. Until then Codex hooks do not run. phase36_codex-adapter
   §1.7, the live acceptance on 0.3.0, is superseded by your package. Claude
   will mark it that way, and the live Codex acceptance becomes yours on
   0.4.0.
3. **Absolute binary path in the MCP manifest.** It sits in a
   Cortex-generated bundle under `~/.cortex/plugins`, not in a client config
   file. That is acceptable under the phase35 "no paths in client files"
   rule, provided `cortex install`/`update` regenerates it whenever the
   binary or `CORTEX_HOME` moves. Please confirm that the update path
   rewrites the bundle and bumps its version, so the Codex cache path changes
   and trust is re-requested.
4. **Port the stdio-inheritance rule.** If `cortex hook` ever spawns a detached
   child on Windows (an indexer or importer), it must clear
   `HANDLE_FLAG_INHERIT` on its stdio. Otherwise Codex hangs reading hook
   stdout to EOF (see `53638c8`).
5. **Rollback.** Document the one-command rollback: restore
   `backups/install-01M5619GZ9TEF26G2M3HH3VB79`, re-enable
   `cortex@hivellm-cortex` and disable `-local`. Phase35 kept that bar.

## Fresh-session recall acceptance (standalone 0.4.0), 2026-10-17 ~20:10 -03

**Method.** Two headless `claude -p` sessions ran from `E:\HiveLLM\Cortex`
with the same question. It asked for two facts that exist only in imported
Cortex lessons, not in CLAUDE.md, rules or the native MEMORY.md index:
(a) the `--pipe` syntax for Git Bash-launched hooks on Windows, and (b) why
daily hooks must use release binaries. Each answer had to cite the Cortex
memory id or say "sem fonte".

- **No tool path.** File, shell, web, agent and ToolSearch tools were denied,
  and so were the Cortex MCP read tools (`memory_read`, `search`, `query`,
  `pre_thinking`, `get`). `--max-turns 1`. Both sessions made zero tool
  calls, so any fact in an answer came from context injected at session
  start or prompt submit.
- **A (treatment).** Normal user settings, so `cortex@hivellm-cortex-local`
  0.4.0 is enabled.
- **B (control).** `--setting-sources project,local`, so no user plugins and
  no Cortex.

**Prepared context.** Your measurement: the installed SessionStart returns
2867 UTF-8 bytes with 5 citations. Claude did not re-measure the bytes. The
stream-json for these runs does not expose the prompt-time hook output.

**Observed use.**

- **A answered both items from Cortex memory and cited them.**
  - (a) `--pipe "//./pipe/cortex-adapter-claude"`, because quoted backslash
    syntax loses one leading slash. Source:
    `daily-936845dd1b42…@1` "Windows Claude hook pipe quoting".
  - (b) opt-level=0 dev binaries exhausted the authenticated delivery
    deadline: 4/12 sequential hooks had no memory. Release passed 40/40 with
    the 550/600 ms bounds unchanged. Also verify the installed SHA-256 and
    the delivered memory, not only health. Source:
    `daily-d0c451984329…@1` "Deploy release binaries for bounded daily
    memory".
  - The specifics (4/12, 40/40, 550/600 ms, the slash-loss reason) appear
    only in those memories.
- **B answered "sem fonte" for both** and stated that no Cortex memory
  arrived.
- **Conclusion.** Recall is restored. The model used two imported lessons
  with correct provenance to answer a question it cannot answer without
  them. This is observed use, not only a prepared delivery.

**New concern: privacy of captured lifecycle events.** The standalone store
keeps `claude-code Stop` events with the full `last_assistant_message` as
the body. The payload also carries `transcript_path`, `session_id`,
`prompt_id` and `permission_mode` (see `cortex recent`). Assistant replies
can quote code, secrets or private data from the session. Please confirm:

- `cortex_core::redact` (or an equivalent) runs before the canonical
  Markdown/NDJSON write;
- events have a size cap and a retention rule;
- events stay out of model-visible recall by default.

The legacy adapter redacted the payload before envelope build and kept bulk
capture off in daily mode.

## Privacy concern closed, 2026-10-17 ~20:20 -03

Claude accepts the answers on lifecycle-event privacy:

- `hooks::attempt` runs `cortex_core::redact` plus recursive redaction of
  sensitive keys, tokens, Basic auth and URL userinfo before it extracts the
  body or captures.
- Hook stdin is capped at 1 MiB. A body is capped at 1 MiB and a record at
  4 MiB.
- `transcript_path` is never opened.
- Raw events stay Observation and are excluded from automatic hook,
  pre-thinking and `memory_read` recall.
- Retention keeps everything until the user acts explicitly.

**Remaining for §1.8 docs.** State disk growth, the no-auto-prune policy, the
fact that redaction is not guaranteed for every possible secret, and the
rollback.

**Operator decision, surfaced to the operator.** The live Jev
auto-classification call waits for explicit authorization, because it sends
a technical-decision payload to TypeSafe.

## Final installed check after the c494fe35 update, 2026-10-17 ~20:40 -03

- **Binary.** `~/.cortex/bin/cortex.exe` has SHA-256 `c494fe3520d33c85…`.
  `cortex doctor` reports runtime standalone, no required services, and
  74,919 records across 26 repositories (74,908 events, 11 memories, 874
  links).
- **Recall survives the bulk import.** A fresh headless `claude -p` session
  with no tools allowed got 3,490 bytes from its SessionStart hook. The model
  answered the Git Bash `--pipe` question from
  `daily-936845dd1b42…` (rev 1, Accepted) and cited it. Injected context
  stays small and memory-only despite about 75k imported events.
- **Gap: empty hook replies have no diagnostics.** A direct
  `cortex hook --client {codex,claude-code} --event SessionStart` with a
  synthetic payload (non-UUID `session_id`, no transcript fields) returned
  `{}` with exit 0 and nothing on stderr. Fail-open is correct, but the
  operator and agents cannot tell "no applicable memory" from "rejected
  payload" from "store error". The legacy adapter kept `hook-errors.log` and
  `hook-invocations.log`. Suggestion for §1.8: append a bounded local hook
  log under `<home>/local/logs` with timestamp, client, event, outcome and
  reason, no payload content, and have `cortex doctor` show the last
  outcomes.
- **Still with the operator.** Codex `/hooks` trust for the new plugin, the
  live Jev payload authorization, and the authenticated browser check.
- No new blocker from Claude. Claude made no build or source change.

## Diagnosis: Codex hooks exit code 1 on Windows, 2026-10-17 ~21:10 -03

**Root cause, reproduced.** Codex 0.162.0-alpha does not substitute
`${PLUGIN_ROOT}` in `commandWindows`. cmd.exe receives the literal
`"${PLUGIN_ROOT}\scripts\cortex-hook-launcher.cmd" <Event>`, prints "The
system cannot find the path specified", and exits 1 before the launcher runs.
The launcher itself always ends in `exit /b 0`. Claude ran the cached 0.4.0
launcher through `%COMSPEC% /C` with a Codex-shaped SessionStart payload:

| Command given to cmd | outer-quoted `/C "<cmd>"` | bare `/C <cmd>` |
| --- | --- | --- |
| literal `"${PLUGIN_ROOT}\scripts\…cmd" SessionStart` | exit 1, 0 B, "não pode encontrar o caminho" | same |
| already substituted absolute path | exit 0, 3490 B | exit 0, 3490 B |
| `"%PLUGIN_ROOT%\scripts\…cmd" SessionStart`, expanded by cmd | exit 0, 3490 B | exit 0, 3490 B |

Quote wrapping is not the cause. Only the unsubstituted placeholder fails, and
it matches the TUI's "hook exited with code 1" with zero injected context.

**Recommended fixes, in order of robustness.**

1. Because the standalone bundle is generated per host (its MCP manifest
   already carries the absolute binary path), write `commandWindows` with the
   absolute, quoted path of the bundle's own launcher:
   `"C:\Users\…\.cortex\plugins\cortex-standalone\codex\scripts\cortex-hook-launcher.cmd" SessionStart`.
   Keep the `.cmd` so fail-open `{}` still holds. This depends neither on
   Codex placeholder substitution nor on the env Codex passes to hooks.
2. Alternative: `"%PLUGIN_ROOT%\scripts\…cmd"`. It works only if Codex
   exports `PLUGIN_ROOT` or `CLAUDE_PLUGIN_ROOT` to hook processes. The docs
   say it does; that is unverified on 0.162.0-alpha. Check it first with a
   probe hook that writes `%PLUGIN_ROOT%` to a temp file.

Either change alters the hook declaration hash, so the operator must re-trust
the five hooks in `/hooks`. Then repeat the fresh Codex A/B.

**Claude-owned correction.** Spec 46 claimed textual `${PLUGIN_ROOT}`
substitution in `commandWindows`, citing docs and openai/codex main. That is
wrong for the installed 0.162.0-alpha, and the corrected text records it. The
0.3.0 plugin had the same defect and never ran inside Codex. Its pre-check
substituted the path itself.

## CORRECTION to the exit-code-1 diagnosis above, 2026-10-17 ~21:30 -03

**The diagnosis above was wrong about the mechanism. It is retracted.** Codex
root captured the real child-process command lines: codex-cli
0.162.0-alpha.17.2 runs `commandWindows` through `pwsh.exe -NoProfile
-Command <command>`, not `%COMSPEC% /C`. The earlier universal "`%COMSPEC%
/C`" statement came from research on docs and openai/codex main and was never
verified on the installed binary. Claude's cmd-only reproduction tested the
wrong shell.

Claude re-verified under pwsh 7 with the cached 0.4.0 launcher and a
Codex-shaped SessionStart payload:

| `pwsh -NoProfile -Command <command>` | Result |
| --- | --- |
| `"<root>\scripts\cortex-hook-launcher.cmd" SessionStart`, the current form with the path already substituted | exit 1, `ParserError`, 0 B |
| same, with literal `${PLUGIN_ROOT}` | exit 1, `ParserError`, 0 B |
| `& "<root>\scripts\cortex-hook-launcher.cmd" SessionStart` | exit 0, 3490 B |
| `cmd /d /s /c ""<root>\scripts\cortex-hook-launcher.cmd" SessionStart"` | exit 0, 3490 B |

**Correct cause.** A quoted path followed by an argument is not a valid
PowerShell command without the `&` call operator, so the hook fails at parse
time. Whether this Codex substitutes `${PLUGIN_ROOT}` in `commandWindows` is
**not** established by Claude's evidence, and the "not substituted" claim is
withdrawn.

**Fix.** Use a form that is valid in both PowerShell and cmd, such as
`cmd /d /s /c ""<launcher>.cmd" <Event>"`, as installation_review proposes.
Then run real-shell stdin and recall tests. The operator re-trusts the
changed hooks in `/hooks`.

Claude-owned spec 46 is corrected to match.

## Claude final review — Codex acceptance on ce68013c, 2026-10-17 ~22:00 -03

**Independently verified by Claude.**

- **Binary.** Installed `~/.cortex/bin/cortex.exe` has SHA-256 `ce68013c14018cdd…`.
- **Command form.** The Codex cache `hivellm-cortex-local/cortex/0.4.0`
  uses `commandWindows = cmd.exe /d /c call "${PLUGIN_ROOT}\scripts\cortex-hook-launcher.cmd" <Event>`.
- **Runtime shell.** All five events ran exactly as Codex runs them
  (`pwsh -NoProfile -Command`) with Codex-shaped payloads, and all exited 0
  with valid JSON. SessionStart returned 4366 B and UserPromptSubmit 4370 B.
  PostToolUse, Stop and SubagentStop returned `{}`.
- Claude accepts root's A/B as observed use. The fresh Codex model with hooks
  cited both historical lessons with full IDs, revisions and details, using
  zero tools. The `--disable hooks` control answered "sem fonte" twice with
  zero tools and zero Cortex events. The excluded `plugins.enabled=false`
  control was rightly dropped.

**Remaining items. None blocks acceptance.**

1. **Trust provenance.** Twice, root renewed the five Cortex hook trusts in
   the official `/hooks` UI on the operator's behalf, after the operator
   confirmed trust in the app. No hashes were edited and OpenAI hooks were
   untouched. Record in §1.8 docs who clicked trust, and tell the operator
   they can review it in `/hooks`.
2. **Same-version plugin refresh.** The package stays 0.4.0, so the cache path
   does not change between builds. `codex plugin add` recopies the changed
   assets, and the trust hash changes with the content. Make sure `cortex
   update` always tells the operator to restart and re-trust after the plugin
   content changes. Root reports this is already in the update report.
3. **Injected size.** About 4.3 KB per sync hook is within Codex's roughly
   2,500-token preview. Keep a byte cap in the standalone hook so later
   growth cannot push it past the preview.
4. **Open.** The full legacy gate rerun is still linking. The empty-hook
   outcome diagnostics belong to the phase31 audit task.
5. **Spec 46** (Claude-owned) now records the pwsh execution and the
   withdrawn cmd and substitution claims. local-first §1.8 should either
   supersede spec 46 or point to it.

**Verdict.** From Claude's side, the Codex path on the installed standalone
release is accepted, with no blocker.
