# 10 — Claude Code adapter (hooks + local daemon)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 04

## Goal

Capture every meaningful signal from a Claude Code session — user prompts, tool calls, tool results, session boundaries — and publish them as envelope-compliant events to the local Cortex ingestion router, without slowing the interactive loop and without shipping secrets off the user's machine. This adapter is the reference implementation; spec 17 adapts it for Cursor/Codex/Gemini.

## Scope

**In:**
- Hook scripts for Claude Code's supported events (`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SessionStart`, `SubagentStop`, `Notification`).
- A tiny local daemon (`cortex-adapter-claude`) the hooks talk to — collects events, batches them, POSTs to `cortex-core` (`/v1/events`).
- In-process static redaction (defense in depth — second pass even though `cortex-core` redacts too).
- Session state: `session_id`, `turn_id`, `tool_call_id` generation + correlation.
- Blocking-law enforcement path: `PreToolUse` synchronously asks `cortex-api` if any **critical** law detector fires; if yes, emits the protocol-defined block response.
- Pre-thinking injection path: `UserPromptSubmit` synchronously calls `cortex-api /v1/query` with `intent=pre_change_context` and prints the JSON bundle Claude Code will merge into the model's system prompt.
- Install / uninstall commands.

**Out:**
- `cortex-core` (spec 04) and `cortex-api` (spec 11, 12, 14) internals.
- Cursor/Codex/Gemini adapters (spec 17).
- Multi-user / remote session replication (future).
- Retro-ingestion of past session logs (that's bootstrap — spec 09).

## Inputs / Outputs

### Files on disk

```
~/.claude/hooks/
  cortex-user-prompt.sh          # UserPromptSubmit
  cortex-pre-tool.sh             # PreToolUse
  cortex-post-tool.sh            # PostToolUse
  cortex-stop.sh                 # Stop
  cortex-session-start.sh        # SessionStart
  cortex-subagent-stop.sh        # SubagentStop
  cortex-notification.sh         # Notification
~/.claude/settings.json          # (hooks wired up; see below)

~/.cortex/
  adapter-claude.sock            # Unix domain socket (or \\.\pipe\cortex-adapter on Windows)
  adapter.pid
  adapter.log
  adapter.toml                   # adapter config
```

Hooks are thin shims: they pipe their JSON stdin to the local daemon over the UDS/pipe and print its response. The daemon owns the heavy lifting (HTTP to `cortex-core`, correlation, redaction).

### Hook ↔ daemon protocol

Hooks send:

```jsonc
{
  "hook": "PreToolUse",
  "session_id": "cc-sess-<uuid>",          // from env or generated
  "cwd": "/path/to/repo",
  "payload": { /* verbatim Claude Code hook JSON */ }
}
```

Daemon replies (must be printed to stdout unchanged by the hook script):

```jsonc
// UserPromptSubmit:
{ "additionalContext": "<pre-thinking JSON block>" }
// PreToolUse (allow):
{}
// PreToolUse (block):
{ "permissionDecision": "deny", "permissionDecisionReason": "LAW-007 ..." }
// Any hook on a daemon error:
{}                                         // empty ⇒ Claude Code proceeds unmodified
```

The adapter's rule: **never break the session.** Any internal error produces an empty JSON response, a structured log entry, and a metric bump — it never bubbles up as a Claude Code failure.

### Envelope mapping (Claude Code hook → Cortex event kind)

| Hook                | Cortex kind(s)                         | Notes                                                          |
|---------------------|----------------------------------------|----------------------------------------------------------------|
| `SessionStart`      | `turn.session_start`                    | Emits `Session` node props                                     |
| `UserPromptSubmit`  | `turn.user`                             | + synchronous `pre_change_context` query                       |
| `PreToolUse`        | `tool_call.requested`                   | + synchronous blocking-law check                               |
| `PostToolUse`       | `tool_call.completed`                   | Carries status + latency + truncated output                    |
| `SubagentStop`      | `turn.subagent_complete`                 | Attaches the subagent's final message                          |
| `Stop`              | `turn.session_stop`                      | Triggers session summary in `cortex-api` (spec 15 hook)        |
| `Notification`      | `event.notification`                     | Permission prompts, idle warnings                              |

All events carry `adapter = "claude-code"`, `model = env CLAUDE_MODEL`, and the session's `cwd` as `context_repo` (best-effort resolution to repo name).

### Adapter config (`~/.cortex/adapter.toml`)

```toml
[adapter]
endpoint = "http://127.0.0.1:15010"          # cortex-core ingestion router
api_endpoint = "http://127.0.0.1:15011"      # cortex-api (query, law check)
timeout_ms = 1500                             # per request; hard cap
queue_bounded = 2048                          # in-memory queue for async publish

[adapter.pre_thinking]
enabled = true
max_bundle_kb = 32
timeout_ms = 600

[adapter.laws]
block_on_critical = true
timeout_ms = 300                              # must stay under Claude Code's hook budget

[adapter.redaction]
extra_patterns = [
  # adapter-side extras; cortex-core still runs the primary pass
]

[adapter.logging]
level = "info"                                # trace | debug | info | warn | error
path = "~/.cortex/adapter.log"
```

## Design

### Component layout (`cortex-adapters/claude-code/`)

```
cortex-adapters/claude-code/
├─ Cargo.toml
├─ src/
│  ├─ main.rs                (daemon)
│  ├─ ipc.rs                 (UDS / named-pipe server)
│  ├─ session.rs             (session_id, turn_id, tool_call_id correlation)
│  ├─ events.rs              (Claude Code hook JSON → Cortex envelope)
│  ├─ redact.rs              (in-process static redactor; mirrors cortex-core patterns)
│  ├─ pre_thinking.rs        (UserPromptSubmit → cortex-api query)
│  ├─ law_check.rs           (PreToolUse → cortex-api /v1/laws/check)
│  ├─ publisher.rs           (HTTP client to cortex-core)
│  ├─ install.rs             (writes hook scripts + settings.json stanzas)
│  └─ config.rs
├─ hooks/                    (source for the shim scripts)
│  ├─ cortex-user-prompt.sh
│  ├─ cortex-pre-tool.sh
│  └─ ...
└─ tests/
```

### Session / turn / tool-call correlation

- `session_id`: comes from `CLAUDE_SESSION_ID` env var (Claude Code exports it) or is synthesized on `SessionStart` and cached by session pid.
- `turn_id`: generated on `UserPromptSubmit`; cached until the next `Stop` or next `UserPromptSubmit`.
- `tool_call_id`: generated on `PreToolUse`; propagated to the corresponding `PostToolUse` via the hook's `tool_use_id` field (Claude Code supplies it).
- If correlation fails (hook fires without a parent), the event is still emitted with `orphan=true` and downstream graph writer (spec 07) handles it.

### Synchronous paths (tight latency budget)

Two hooks run **blocking**, inline with the model loop:

1. **`UserPromptSubmit`**
   - Daemon assembles a `pre_change_context` query from:
     - the user prompt text
     - the current `cwd` (mapped to repo)
     - recently-edited files (via `git status` in `cwd`, TTL-cached 10 s)
   - POSTs to `cortex-api /v1/query` with `timeout_ms=600`.
   - Response JSON (decisions, similar turns, active laws) is returned to Claude Code as `additionalContext`.
   - On timeout / error: empty `additionalContext` — the session continues unchanged.

2. **`PreToolUse`**
   - Daemon POSTs to `cortex-api /v1/laws/check` with the proposed tool call.
   - Response: `{ "violations": [ { "law_id": "...", "severity": "critical", "message": "..." } ] }`.
   - If any `severity=critical`: emit `permissionDecision: deny` + concatenated messages.
   - Timeout budget: `300 ms`. On timeout: **allow** (fail-open for UX); the violation is still captured asynchronously if the check completes later.

Both paths ship the event to the async publisher regardless — the sync calls are about **affecting the loop**, not about capture.

### Asynchronous publisher

- In-memory bounded queue (`queue_bounded`, default 2 048 events).
- Background task drains the queue to `cortex-core /v1/events` in batches of 32, up to every 200 ms.
- On Cortex-core 5xx: retry 3× with exp backoff; after that, write to a local **overflow WAL** (`~/.cortex/overflow.wal`) and continue. The WAL is replayed on next daemon startup.
- Queue-full policy: **drop-oldest** (capture continuity > historical fidelity), with a metric bump. Dropped events are also mirrored to the overflow WAL.

### In-process redaction

Mirrors `cortex-core`'s pattern catalog. Redaction happens **before** anything leaves the hook's process (and again in core — defense in depth). The adapter ships pattern updates alongside the daemon binary, versioned. A divergence between adapter and core patterns is not a bug: core is the authority, the adapter is a cheap first filter.

### Windows vs Unix IPC

- **Unix:** UDS at `~/.cortex/adapter-claude.sock`, perms 0600.
- **Windows:** named pipe `\\.\pipe\cortex-adapter-claude`, ACL restricted to current user.

Hook shim detects OS and uses `nc -U` / `socat` (Unix) or PowerShell `NamedPipeClientStream` (Windows). The shim scripts live in `hooks/` and are copied into `~/.claude/hooks/` by `cortex-adapters install`.

### Install / uninstall

```
cortex-adapters install claude-code
cortex-adapters uninstall claude-code
cortex-adapters status
```

- **Install:** copies hook shims, patches `~/.claude/settings.json` to wire them up (idempotent — detects existing Cortex entries), creates `~/.cortex/` layout, writes a systemd/launchd/Windows-Service unit, starts the daemon.
- **Uninstall:** reverts settings.json, stops the daemon, removes hook shims (leaves logs + overflow WAL unless `--purge`).
- **Status:** prints daemon PID, uptime, queue depth, overflow WAL size, last N publish errors.

### Failure modes

| Failure                                     | Handling                                                                   |
|---------------------------------------------|----------------------------------------------------------------------------|
| Daemon not running when a hook fires         | Hook prints `{}` to stdout and exits 0; event lost; Claude Code continues |
| `cortex-core` unreachable                    | Publisher retries; overflow WAL catches drops; adapter stays healthy       |
| `cortex-api` unreachable (sync path)         | Empty `additionalContext`; allow on law check; async event still queued    |
| Hook budget exceeded (>300 ms for laws)       | Fail-open; async follow-up                                                  |
| Corrupt hook input JSON                       | Log + `{}` + metric; never throw                                            |
| Overflow WAL > 100 MB                         | Metric alert; daemon keeps writing (rotated)                                |
| Session-id collision across concurrent Claude Codes | Include pid in fallback synth: `cc-sess-<pid>-<uuid>`                   |

### Observability

```
cortex.adapter.events.total               counter, labels: kind
cortex.adapter.events.dropped             counter, labels: reason (queue_full | hook_error)
cortex.adapter.sync.latency_ms            histogram, labels: hook
cortex.adapter.sync.timeouts              counter, labels: hook
cortex.adapter.overflow.wal_bytes         gauge
cortex.adapter.publisher.errors           counter, labels: status
cortex.adapter.pre_thinking.bundle_bytes  histogram
cortex.adapter.laws.blocks                counter, labels: law_id
```

Logs are structured JSON lines. The daemon exposes a tiny HTTP endpoint (`http://127.0.0.1:15020/metrics`) for Prometheus scraping when enabled.

## Acceptance criteria

- [ ] `cortex-adapters install claude-code` on a fresh machine wires hooks into `~/.claude/settings.json` idempotently; re-running is a no-op.
- [ ] A real Claude Code session emits envelope-compliant events for each hook; verified via a recorded trace against the `cortex-core` ingestion router.
- [ ] `UserPromptSubmit` returns a non-empty `additionalContext` within 600 ms against a local `cortex-api`; on forced 2 s latency, returns `{}` without stalling the session.
- [ ] `PreToolUse` with a synthetic `git commit --no-verify` on a repo carrying LAW-007 returns `permissionDecision: deny` and the session shows the block message.
- [ ] `PreToolUse` against a 400 ms-lagging `cortex-api` (above the 300 ms budget) allows the tool call but captures the late-arriving violation asynchronously.
- [ ] Daemon restart replays the overflow WAL; no event loss in the power-off/on drill.
- [ ] Queue-full drill: 10 000 events in 1 s with core unreachable → oldest events spill to WAL, drop counter increments, adapter stays healthy.
- [ ] Windows parity: named-pipe path passes the full hook suite on Windows 10/11.
- [ ] Uninstall restores settings.json to its pre-install state; re-running `install` → `uninstall` produces an identical file (diff is empty).
- [ ] In-process redaction strips a synthetic token from a `Bash` tool-input before the event leaves the daemon; verified by captured Synap payload.
- [ ] Hook-never-throws: a malformed hook-input JSON produces `{}` on stdout, exit 0, and a log line; Claude Code session continues unaffected.
- [ ] Telemetry counters are non-zero after a 10-turn recorded session.

## Decisions

1. **Daemon, not in-process per hook.** Hooks fire 30–100 times per minute; a persistent daemon amortizes TLS/HTTP connection setup and centralizes correlation state.
2. **UDS / named-pipe, not HTTP to localhost.** Lower latency, cheaper ACLs, no port-conflict surprises.
3. **Fail-open on law check timeout.** UX cost of false blocks is higher than governance cost of a rare missed block — we capture asynchronously and punish post-hoc per spec 14.
4. **Hooks never break the session.** Empty-JSON-on-error is a hard invariant; tested as an acceptance criterion.
5. **In-process redaction is best-effort, core is authoritative.** Two layers > one; adapter divergence is acceptable as long as core's catalog is never weaker than the adapter's.
6. **Drop-oldest under queue pressure.** Preserves ingestion continuity for the ongoing session; the WAL catches the history. Rationale: in a backlog, the newest events correlate with the current failure we're probably debugging.
7. **Single binary for daemon + install.** Simpler distribution; subcommands.

## Open questions

1. **Session-resume correlation.** If the user `/clear`s mid-session and then issues another prompt, is that a new `Session` or a continuation? Leaning new Session with a `continues_from` edge in Nexus. Finalize when spec 11 defines the edge semantics.
2. **Multi-root projects.** A session whose `cwd` spans repos (e.g., a multi-repo workspace) currently maps to the first-matching repo. Do we emit the full set? Defer — revisit after Phase 1 retrieval quality pass.

## References

- Architecture §5.1 (capture layer), §8 (end-to-end flow example).
- Spec 01 — Event schema.
- Spec 04 — Cortex Core (ingestion router, `/v1/events`).
- Spec 11 — Query API (`/v1/query`, `intent=pre_change_context`).
- Spec 13 — Laws DSL (detector contract used by `/v1/laws/check`).
- Spec 14 — Governance engine (async punishment ladder).
- Spec 17 — Additional adapters (this spec is the reference).
- Claude Code hooks docs: https://docs.anthropic.com/en/docs/claude-code/hooks (hook event list, input/output JSON).
