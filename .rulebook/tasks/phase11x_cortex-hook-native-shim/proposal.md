# Proposal: phase11x_cortex-hook-native-shim

Source: hook latency profiling captured 2026-05-06 (this session, no
separate analysis doc).

## Why

The Claude Code hooks shipped by `cortex-adapter-claude-code` cost
**~730 ms each** on Windows. Profiling shows:

| Component | ms | Note |
|---|---|---|
| `pwsh -NoProfile` cold start | ~545 | floor; same for every hook |
| script body (log append, JSON build) | ~95 | unavoidable in PS |
| named-pipe round-trip + daemon work | ~90 | daemon is fine |
| **total** | **~730** | per invocation |

In a typical turn (SessionStart + UserPromptSubmit + 5×PreToolUse +
5×PostToolUse + SubagentStop + Stop = 14 hook invocations) the
adapter alone burns **~9 s of wall-clock** — **~7.6 s of which is
just `pwsh` cold start**. The daemon's actual work (envelope
publish, pre-thinking pipeline, law-check) only accounts for ~90 ms
per call.

Two structural reasons cause this:

1. **Hooks shell out to PowerShell on Windows** because Git Bash's
   `nc -U` does not speak Windows named pipes. The PS process boot
   dominates everything.
2. **Every hook waits synchronously** even when the response is a
   no-op `{}`. PostToolUse / SubagentStop / Stop / SessionStart /
   Notification do not consume `additionalContext` or
   `permissionDecision` and therefore could be fire-and-forget.

## What Changes

- Add a new bin target `cortex-hook` to
  `crates/cortex-adapter-claude-code/src/bin/cortex-hook.rs`. Rust
  release binary cold-start on Windows is ~15–30 ms. The bin reads
  stdin, builds the existing `HookFrame` JSON, connects to the
  daemon's named pipe (Windows) or Unix socket (Linux/macOS), writes
  the frame, optionally reads the response, and prints it on stdout
  — same wire shape as the current `.sh`/`.ps1` shims.
- Add a `--fire-forget` flag (or per-event default) so PostToolUse,
  SubagentStop, Stop, SessionStart, and Notification disconnect
  immediately after `write_all` without waiting for a reply. The
  daemon already publishes asynchronously; the shim simply skips the
  read.
- Replace the 14 platform-specific shell shims under
  `crates/cortex-adapter-claude-code/hooks/cortex-*.{sh,ps1}` with
  per-event invocations of the new bin. Settings registers
  `cortex-hook user-prompt-submit` etc. directly. The legacy `.sh`
  shims remain only as a Linux/macOS fallback when the bin is not
  on PATH; the `.ps1` shims are deleted.
- Move log appends (`hook-invocations.log`, `hook-errors.log`) from
  the shim into the daemon so the per-invocation file open/close
  cost is paid once on the daemon side, with proper rotation.
- `install` step (`crates/cortex-adapter-claude-code/src/install.rs`)
  generates settings entries pointing at `cortex-hook` and confirms
  the bin is on PATH; falls back to the legacy shims if not.
- Tests: unit test on the new bin's frame builder; integration test
  drives the bin against an in-process daemon over a real named pipe
  (Windows) and Unix socket (Linux); cold-start benchmark
  (`benches/hook_cold_start.rs`) asserts the bin starts in <50 ms
  on the build host.
- Docs: `crates/cortex-adapter-claude-code/README.md` updates the
  Configuration table; `docs/specs/10-claude-code-adapter.md` §Hook
  contract gets a "Transport: native bin (Windows / Linux / macOS) +
  legacy shell fallback (Linux / macOS)" subsection.

## Impact

- **Affected specs**: `docs/specs/10-claude-code-adapter.md` (Hook
  transport subsection), no envelope-schema changes.
- **Affected code**:
  - `crates/cortex-adapter-claude-code/Cargo.toml` (new bin target)
  - `crates/cortex-adapter-claude-code/src/bin/cortex-hook.rs` (new)
  - `crates/cortex-adapter-claude-code/src/install.rs` (settings
    generator switches default to the bin)
  - `crates/cortex-adapter-claude-code/hooks/cortex-*.ps1` (deleted)
  - `crates/cortex-adapter-claude-code/hooks/cortex-*.sh` (kept as
    Linux/macOS fallback only, deps trimmed)
  - `crates/cortex-adapter-claude-code/src/dispatcher.rs` (log-on-
    receive moved here from the shim)
  - `crates/cortex-adapter-claude-code/benches/hook_cold_start.rs`
    (new)
- **Breaking change**: NO. The wire protocol (HookFrame /
  HookResponse JSON) does not change. Existing installations that
  still have the legacy shims continue to work; the new bin is an
  additive transport. Operators who already source
  `~/.claude/settings.json` from `install.rs` get the bin path on
  next `cortex-adapter-claude install`.
- **User benefit**: ~7–8 s saved per turn on Windows (~80 % of
  current adapter overhead). Visible improvement in every Claude
  Code session that uses the adapter.
