# 40. App-managed daily runtime lifecycle: started by cortex-hook, no OS autostart

**Status**: accepted
**Date**: 2026-10-17
**Related Tasks**: phase35_self-contained-plugin-install

## Context

The daily runtime (`cortex-api` plus the `cortex-adapter-claude` daemon) did not
come back after a reboot. Between July and October 2026 that left Claude Code
sessions without capture or recall until an operator ran the launcher by hand.
The reference design, akitaonrails/ai-memory, registers an HKCU Run key. The
project rules forbid OS schedules (cron, systemd, launchd, schtasks), and
ADR-039 selected an app-managed lifecycle. Client configuration also carried
host-specific absolute paths and a config-file path. Phase35 removes them in
favour of self-contained Claude Code and Codex plugins.

## Decision

`cortex-hook` starts the runtime on demand. On `SessionStart`,
`UserPromptSubmit` or `PreToolUse` it checks the default adapter endpoint. When
that endpoint is absent, it launches the sibling `cortex-api` (only if its port
does not answer) and `cortex-adapter-claude daemon`. Both run detached from the
client's console and, where the job allows it, from the client's job object.

- Coordination uses `<Cortex home>/lifecycle/spawn.lock`, created exclusively
  and replaced after 10 s, and `last-spawn`, which records the attempt time and
  outcome. Concurrent hooks start at most one runtime.
- After a successful attempt the runtime has a 10 s grace period to come up.
  Attempts are at least 60 s apart. After an attempt that failed or produced no
  endpoint, hooks report the failure without retrying until the backoff
  expires.
- The hook stays within its 1.5 s budget and retries the exchange while time
  remains. An operator `systemMessage` appears only when the start fails, once
  per session, and never as `additionalContext`.
- Explicit endpoints (`--pipe`, `--sock`, or configured adapter endpoints other
  than the default) are never started, and `CORTEX_HOOK_AUTOSTART=0` disables
  starting altogether.
- No Run key, scheduled task, Startup entry or service is registered.

## Consequences

The first session after a reboot brings memory back with no operator step and no
OS integration. A host with no session running has no runtime running, which is
the intended resource profile. A runtime that keeps failing costs at most one
attempt per minute, and the first prompt of each session shows the failure.
Starting depends on the binaries sitting next to `cortex-hook` (installed
together by `cortex-ops install-binaries`) and on `~/.cortex/config.toml`
written by `cortex-ops setup`. Reopening OS autostart needs an explicit operator
decision that supersedes this record.

**2026-10-17:** the install commands moved to the `cortex` binary.
`cortex install` replaces `cortex-ops install-binaries` and `cortex setup`
replaces `cortex-ops setup`; the decision above is unchanged.
