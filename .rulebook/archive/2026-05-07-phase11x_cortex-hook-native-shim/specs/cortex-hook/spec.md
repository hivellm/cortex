# cortex-hook — native hook shim

## ADDED Requirements

### Requirement: Native bin replaces shell shims as the default hook transport

The `cortex-adapter-claude-code` crate MUST ship a `cortex-hook` bin
that the daemon's `install` step registers as the default Claude
Code hook command on every supported platform (Windows, Linux,
macOS).

The bin MUST accept the canonical hook event name as its first
positional argument (`SessionStart`, `UserPromptSubmit`,
`PreToolUse`, `PostToolUse`, `SubagentStop`, `Stop`, `Notification`)
and MUST emit the same `HookFrame` wire shape the existing shell
shims produce.

#### Scenario: install writes bin paths into settings.json

Given `cortex-hook` is on PATH
When the operator runs `cortex-adapter-claude install`
Then `~/.claude/settings.json` SHALL register `cortex-hook <event>` for every hook event and SHALL NOT register any `.ps1` or `.sh` paths

#### Scenario: install falls back to shell shims when bin missing

Given `cortex-hook` is NOT on PATH
When the operator runs `cortex-adapter-claude install`
Then `~/.claude/settings.json` SHALL register the legacy `.sh` (Linux/macOS) or `.ps1` (Windows) shims as before and `install` SHALL print one warning advising the operator to put `cortex-hook` on PATH

---

### Requirement: Cold-start budget

The release-mode `cortex-hook` bin MUST start in under 50 ms on
Windows and under 20 ms on Linux when invoked with `--help`. A
criterion bench MUST enforce this in CI.

#### Scenario: cold-start regression fails CI

Given the criterion bench `hook_cold_start` is wired into CI
When a code change pushes Windows p50 cold start above 80 ms or Linux p50 above 30 ms
Then the bench SHALL fail the build with a regression report

---

### Requirement: Fire-and-forget for non-synchronous events

When the bin is invoked with `--fire-forget` (or the equivalent
default for events that do not consume `additionalContext` or
`permissionDecision`), the bin MUST disconnect immediately after
flushing the frame and MUST NOT block waiting for a daemon response.

The events that default to fire-and-forget are: `PostToolUse`,
`SubagentStop`, `Stop`, `SessionStart`, `Notification`.

The events that REMAIN synchronous are: `UserPromptSubmit` (consumes
`additionalContext`) and `PreToolUse` (consumes
`permissionDecision`).

#### Scenario: PostToolUse runs sub-100ms wall-clock

Given the daemon is running on the local host
When the bin is invoked as `cortex-hook PostToolUse --fire-forget` with a 4 KB stdin payload
Then the wall-clock duration SHALL be under 100 ms p50 on Windows and the bin SHALL print `{}` to stdout

#### Scenario: UserPromptSubmit returns the daemon's bundle

Given the daemon is running and `cortex-api /v1/query` returns a non-empty bundle
When the bin is invoked as `cortex-hook UserPromptSubmit` with a user-prompt JSON on stdin
Then the bin SHALL print a JSON object containing `hookSpecificOutput.additionalContext` matching the daemon's reply, and the wall-clock duration SHALL be under 250 ms p50 on Windows

---

### Requirement: Fail-open on every error path

The bin MUST never break the Claude Code session. On any I/O error,
timeout, malformed payload, missing socket / pipe, or panic, the bin
MUST print `{}` to stdout and exit with status 0.

#### Scenario: daemon down

Given the named pipe / Unix socket is not bound by any process
When the bin is invoked
Then the bin SHALL print `{}` and exit 0 within `--timeout-ms` (default 1500 ms)

#### Scenario: malformed stdin

Given stdin contains non-JSON garbage
When the bin is invoked
Then the bin SHALL still publish a frame with `payload: {}` and SHALL print `{}` if the daemon does not respond

#### Scenario: kill-switch honoured

Given `CORTEX_ADAPTER_DISABLE=1` is set in the environment
When the bin is invoked for any event
Then the bin SHALL print `{}` and exit 0 without opening the pipe / socket

---

### Requirement: Logging moves into the daemon

Per-invocation logging (`~/.cortex/hook-invocations.log`,
`~/.cortex/hook-errors.log`) MUST happen on the daemon side, not in
the shim. The daemon MUST rotate each log when it crosses 10 MB and
MUST keep at most two rotations on disk.

#### Scenario: log rotation under load

Given the daemon has been running long enough to write more than 10 MB to `hook-invocations.log`
When the next dispatch arrives
Then the daemon SHALL rotate the file to `hook-invocations.log.1` (overwriting any existing rotation) and start a new `hook-invocations.log` for subsequent entries

---

### Requirement: Backwards compatibility

Existing shells, settings, and call paths MUST keep working:

- Operators who already have legacy `.sh` / `.ps1` shims in their `~/.claude/settings.json` SHALL keep functioning until they re-run `cortex-adapter-claude install`.
- The wire protocol on the named pipe / Unix socket SHALL NOT change. The daemon MUST keep accepting the existing `HookFrame` JSON shape.
- Tests under `crates/cortex-adapter-claude-code/tests/dispatcher.rs` MUST keep passing without modification.

#### Scenario: legacy shim still works against the new daemon

Given a `~/.claude/settings.json` registering the legacy `.sh` shim
When Claude Code triggers a hook event
Then the daemon SHALL accept and process the frame identically to a frame sent by `cortex-hook`
