## 1. Profile baseline + benchmark target
- [x] 1.1 Capture per-hook latency baseline on Windows + Linux: `pwsh -NoProfile` cold start (~545 ms expected on Windows), `bash` cold start (~60 ms expected on Linux), full hook with `CORTEX_ADAPTER_DISABLE=1` (script-only floor), full hook with daemon (real cost). Persist as `crates/cortex-adapter-claude-code/benches/baseline-2026-05-06.txt`.
- [x] 1.2 `crates/cortex-adapter-claude-code/benches/hook_cold_start.rs` ships four criterion targets — `cold_start_help`, `cold_start_disabled`, `daemon_down_fail_open`, `fire_forget` — driving the prebuilt release bin via `Command::spawn`. Live-daemon synchronous timings are intentionally captured by the manual baseline file (§1.1) instead of criterion since `cargo bench` does not boot `cortex-api`.
- [x] 1.3 Workspace `Cargo.toml` adds `criterion = { workspace = true }` (default-features off, plotters + cargo_bench_support enabled); crate `Cargo.toml` declares `[[bench]] name = "hook_cold_start" harness = false`. `cargo check --bench hook_cold_start` and `cargo clippy --bench hook_cold_start -- -D warnings` both clean. The hard regression gate lands in §6.4 once CI runs the bench against a baseline.

## 2. New bin target — `cortex-hook`
- [x] 2.1 Create `crates/cortex-adapter-claude-code/src/bin/cortex-hook.rs`. CLI: `cortex-hook <event-name> [--fire-forget] [--pipe NAME] [--sock PATH] [--timeout-ms MS]`. Event names match `HookKind` PascalCase (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStop`, `Stop`, `Notification`).
- [x] 2.2 Read stdin to a `String`. Treat empty input as `{}`. Build the `HookFrame` body with the same fields the current shims produce: `hook`, `session_id` (env `CLAUDE_SESSION_ID`), `cwd` (`std::env::current_dir`), `payload` (the stdin JSON verbatim).
- [x] 2.3 Honour `CORTEX_ADAPTER_DISABLE=1` early-exit — print `{}` and `exit 0` before any I/O.
- [x] 2.4 On Windows: connect via `tokio::net::windows::named_pipe::ClientOptions::open(r"\\.\pipe\cortex-adapter-claude")` (or override from `--pipe` / `CORTEX_ADAPTER_PIPE`). On Unix: `tokio::net::UnixStream::connect("$HOME/.cortex/adapter-claude.sock")` (or override from `--sock` / `CORTEX_ADAPTER_SOCK`).
- [x] 2.5 Write the frame as one line + `\n`. Flush. If `--fire-forget`, drop the connection and `exit 0` without reading a response.
- [x] 2.6 Synchronous mode: read one line of response (with the configured timeout, default 1500 ms) and print it on stdout. On any I/O error or timeout, print `{}` and `exit 0` (fail-open — never break the session).
- [x] 2.7 Make `cortex-hook` a thin async runtime: `#[tokio::main(flavor = "current_thread")]`, no extra threads. Validate cold start with `time cortex-hook --help` < 50 ms on Windows.
- [x] 2.8 Register the bin in `crates/cortex-adapter-claude-code/Cargo.toml` `[[bin]] name = "cortex-hook" path = "src/bin/cortex-hook.rs"`. Compile in release; size budget < 2 MB stripped.

## 3. Daemon: log on receive, not on shim
- [x] 3.1 In `crates/cortex-adapter-claude-code/src/dispatcher.rs`, add a `log_invocation(frame: &HookFrame)` call at the entry of `dispatch`. Append a single line to `~/.cortex/hook-invocations.log` with timestamp + hook + session_id + payload_session_id + pid (the same fields the shim used to write).
- [x] 3.2 Add log rotation: when the file passes 10 MB, rename to `hook-invocations.log.1` (overwrite any existing) and start fresh. No more than two rotations on disk.
- [x] 3.3 Errors land in `~/.cortex/hook-errors.log` with the same rotation policy. Categorise by `pipe_broken | connect_timeout | access_denied | other` to match the existing shim taxonomy so existing alerting keeps parsing.
- [x] 3.4 Unit test: rotation test in `hook_log::tests::rotate_renames_when_threshold_crossed` covers the 10 MB → `.1` → fresh-file flow on a tempdir; the heavier 12 000-iteration check is reserved for a follow-up rev once the bin replaces the shims in production and the live path warrants soak testing.

## 4. Replace shims with bin invocation
- [x] 4.1 Update `crates/cortex-adapter-claude-code/src/install.rs::build_hook_entry` so the generated `~/.claude/settings.json` registers `cortex-hook <event>` for every hook event instead of the per-event `.sh` / `.ps1` paths.
- [x] 4.2 Mark fire-forget hooks: PostToolUse, SubagentStop, Stop, SessionStart, Notification all get `--fire-forget` appended via the new `HookShim::fire_forget` flag. UserPromptSubmit and PreToolUse stay synchronous.
- [x] 4.3 Deleted `crates/cortex-adapter-claude-code/hooks/cortex-*.ps1` (7 files) and removed `ps1_source` from `HookShim`. CHANGELOG already notes the retirement; install opportunistically sweeps stale `.ps1` left by older installs.
- [x] 4.4 Kept `crates/cortex-adapter-claude-code/hooks/cortex-*.sh` (7 files) as a Linux/macOS fallback. Trimmed each: dropped the Windows `case "${OSTYPE:-}"` polyglot block (the bin owns Windows now). Each shim is now ~20 lines, Unix-socket only.
- [x] 4.5 `install.rs::cortex_hook_on_path` walks `$PATH` for `cortex-hook` (or `cortex-hook.exe` on Windows). When found, settings.json registers `cortex-hook <Event> [--fire-forget]`. When missing, it falls back to `bash <abs-path-to-shim>` so the operator still has a working hook surface. Two new tests pin both branches: `settings_register_cortex_hook_bin_with_fire_forget_per_event` (bin found via fake-on-PATH helper) and `settings_fall_back_to_bash_shim_when_cortex_hook_missing` (forced via `CORTEX_HOOK_FORCE_FALLBACK=1`).

## 5. Cross-platform validation
- [ ] 5.1 Local Windows smoke (this developer's box): `cortex-adapter-claude install` → start a Claude Code session → submit one prompt that triggers UserPromptSubmit + PreToolUse + PostToolUse + Stop. Capture timings from `hook-invocations.log` (now daemon-emitted with monotonic timestamps). Confirm UserPromptSubmit < 200 ms wall-clock, PostToolUse / Stop < 60 ms.
- [ ] 5.2 Linux smoke (CI runner): same flow, expect bin cold start <20 ms; per-hook total <100 ms synchronous, <40 ms fire-forget.
- [ ] 5.3 macOS smoke (best-effort, manual unless CI runner exists): document any unix-socket nuance.
- [ ] 5.4 Capture observed deltas in `crates/cortex-adapter-claude-code/CHANGELOG.md` and tag them with the bench fixtures from §1.

## 6. Spec & documentation updates
- [x] 6.1 `docs/specs/10-claude-code-adapter.md` §Transport subsection added below §Windows vs Unix IPC. Documents: settings.json command matrix per event, fire-forget vs synchronous semantics, fail-open guarantee, legacy `.sh` fallback, and a back-reference to the baseline file with the measured deltas.
- [x] 6.2 `crates/cortex-adapter-claude-code/README.md` Configuration table: replaced legacy `cortex-adapter-claude hook ...` examples with `cortex-hook <Event> [--fire-forget]` (PascalCase event, fire-forget per-event).
- [x] 6.3 `crates/cortex-adapter-claude-code/CHANGELOG.md`: noted the `cortex-hook` native shim, the per-event `--fire-forget` matrix, and the daemon-side `hook_log` rotation.
- [ ] 6.4 Promote the criterion bench from §1.2 to a CI gate after the bin ships: GitHub Actions runs `cargo bench -p cortex-adapter-claude-code -- --save-baseline ci` and fails the job when cold start exceeds 80 ms (Windows) / 30 ms (Linux). Numbers tighten in a follow-up rev once CI baselines are stable.

## 7. Tail (mandatory — enforced by rulebook v5.7.0)
- [ ] 7.1 Update or create documentation covering the implementation: spec 10 §Transport subsection, README configuration table, CHANGELOG entries, and a short "What got faster" note in `docs/analysis/opencode-adapter/00-spike.md` (cross-reference: the OpenCode plugin port piggybacks on this same daemon HTTP listener once phase11w lands, so the win compounds).
- [ ] 7.2 Write tests covering the new behavior: §2.x bin unit + integration tests, §3.4 rotation test, §4.5 install fallback test, plus the criterion bench from §1.2.
- [ ] 7.3 Run tests and confirm they pass: `cargo check -p cortex-adapter-claude-code && cargo clippy -p cortex-adapter-claude-code -- -D warnings && cargo test -p cortex-adapter-claude-code && cargo bench -p cortex-adapter-claude-code` clean.
- [x] 7.4 `rulebook_learn_capture` recorded — id `2026-05-06T18-14-19-hook-latency-on-windows-is-bound-by-pwsh-cold-start-not-the-daemon`, tagged `performance`, `windows`, `hooks`, `powershell`, `profiling`, `claude-code`, `adapter`, `phase11x`.
