# 10 — Claude Code adapter (hooks + local daemon)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 04

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

### Envelope mapping (Claude Code hook → canonical Cortex event)

The adapter emits the canonical `Envelope` defined by spec 04
(`crates/cortex-core/schemas/envelope.schema.json`). Spec 04's `kind`
enum is the authority — there is no parallel "dotted" `kind` vocabulary.
Hooks without a canonical analogue still fire the synchronous
`HookResponse` path (law-check verdicts, pre-thinking bundles) but
do not produce a published event:

| Hook                | Canonical `kind` | Sync path                        | Notes                                                              |
|---------------------|------------------|----------------------------------|--------------------------------------------------------------------|
| `UserPromptSubmit`  | `turn`           | `pre_change_context` query        | payload `{user_message, assistant_message: null, tokens?, tool_call_event_ids?}`. Captures the prompt-side of the turn — the assistant hasn't replied yet. |
| `PostToolUse`       | `tool_call`      | —                                 | payload `{tool_name, input, output, outcome, duration_ms?, touched?}` |
| `SubagentStop`      | `agent_call`     | —                                 | payload `{agent_type, description, prompt?, model?, team_name?, child_event_ids?, result?, duration_ms?, outcome}` |
| `Stop`              | `turn`           | —                                 | payload `{user_message: "", assistant_message, tokens?, tool_call_event_ids?}`. Captures the reply-side of the turn — `assistant_message` is extracted from the JSONL at `payload.transcript_path` (last `assistant` entry's joined `text` content blocks). Both `Turn` envelopes share the same `turn_id` under `context.extras.claude_code` so downstream readers fold them. Failure to read / parse the transcript still publishes the envelope with `assistant_message: null` so the boundary timestamp is preserved. |
| `PreToolUse`        | _drop_           | blocking law-check                | The matching `PostToolUse` carries the canonical record.            |
| `SessionStart`      | _drop_           | —                                 | The first `turn` opens the session.                                 |
| `Notification`      | _drop_           | —                                 | No canonical kind; recorded only in adapter-side metrics.           |

All published envelopes set `tool = "claude-code"`, `schema_version = "1"`, `stream = "live"`, `model = env CLAUDE_MODEL` when present, and a `context` block with `platform`, `cwd`, `repo` (best-effort resolution from `cwd`), and an `extras.claude_code` sub-object carrying the adapter-side correlation IDs (`turn_id`, `tool_call_id`, `tool_use_id`, `orphan`) so the indexing layer can reconstruct turn / tool-call lineage without polluting the canonical envelope.

#### Session metadata persistence (phase10i)

`MetadataStore::upsert_session` preserves the existing
`sessions.tool` value when a subsequent upsert passes an empty
string — the pre-phase10i upsert wrote `tool=excluded.tool`
unconditionally, so lifecycle hooks that didn't capture the
tool name (`Stop` / `Notification`) would NULL out the
session-start value and the dashboard's session list ended up
with 574 `tool: null` rows. The adapter is unchanged on this
path: it always stamps `tool = "claude-code"`. The
backfill CLI (`cortex-ops sessions backfill-tool`) migrates
the rows the pre-phase10i daemon NULL-ed out.

### Adapter config (`~/.cortex/adapter.toml`)

```toml
[adapter]
endpoint = "http://127.0.0.1:17010"          # cortex-core ingestion router
api_endpoint = "http://127.0.0.1:17000"      # cortex-api (query, law check)
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
   - Daemon drives the `cortex-pre-thinking` pipeline (spec 12) which:
     - derives the `Scope` from `cwd` (mapped to repo) and recent files,
     - selects the `intent` from the prompt (default `pre_change_context`),
     - POSTs a `cortex_api::QueryRequest` (intent in body, `query` field) to `cortex-api /v1/query` with `timeout_ms=600`,
     - formats the `QueryResponse` to a deterministic Markdown bundle and clips it to `max_bundle_kb` (default 32 KB).
   - The Markdown string is returned to Claude Code under `hookSpecificOutput.additionalContext` (camelCase, per Claude Code's hook contract).
   - On timeout / error / empty bundle: the response is `{}` — no `hookSpecificOutput` field, the session continues unchanged.

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

The native `cortex-hook` bin (see §Transport below) cfg-gates the
backend per platform: on Windows it opens the named pipe via
`tokio::net::windows::named_pipe::ClientOptions`, on Unix it connects
the UDS via `tokio::net::UnixStream`. Operators override either path
with `--pipe NAME` / `CORTEX_ADAPTER_PIPE` or `--sock PATH` /
`CORTEX_ADAPTER_SOCK`. The wire shape on either transport is one
JSON line per round-trip: a `HookFrame` request and either a
`HookResponse` reply (synchronous) or no reply at all (`--fire-forget`).

### Transport (phase 11x)

The default Claude Code hook command is the native binary
**`cortex-hook`** (release-mode Rust, ~50 ms cold start on Windows).
`install.rs` writes the following entries into
`~/.claude/settings.json`:

| Hook              | Settings command                                  | Mode           |
|-------------------|---------------------------------------------------|----------------|
| `UserPromptSubmit`| `cortex-hook UserPromptSubmit`                    | synchronous    |
| `PreToolUse`      | `cortex-hook PreToolUse`                          | synchronous    |
| `PostToolUse`     | `cortex-hook PostToolUse --fire-forget`           | publish-only   |
| `SubagentStop`    | `cortex-hook SubagentStop --fire-forget`          | publish-only   |
| `Stop`            | `cortex-hook Stop --fire-forget`                  | publish-only   |
| `SessionStart`    | `cortex-hook SessionStart --fire-forget`          | publish-only   |
| `Notification`    | `cortex-hook Notification --fire-forget`          | publish-only   |

Synchronous hooks block on a one-line response from the daemon
because Claude Code consumes the reply (`additionalContext` for
`UserPromptSubmit`, `permissionDecision` for `PreToolUse`).
Fire-and-forget hooks flush the frame and disconnect; the daemon
still publishes envelopes asynchronously, so no event is lost. This
matrix matches §Synchronous paths above — fire-forget covers exactly
the events with no daemon return value.

Bin behaviour:

- Reads stdin to a string. Treats empty / non-JSON input as `{}` and
  builds the canonical `HookFrame` (fields: `hook` PascalCase,
  `session_id` from `CLAUDE_SESSION_ID`, `cwd`, `payload` raw).
- Honours `CORTEX_ADAPTER_DISABLE=1` — print `{}` and `exit 0` before
  any I/O.
- Default response timeout 1500 ms (`--timeout-ms`). On any error —
  pipe / socket missing, peer dropped mid-write, malformed reply,
  timeout — the bin prints `{}` and exits 0. **Never breaks the
  Claude Code session.**
- Single-thread tokio runtime
  (`#[tokio::main(flavor = "current_thread")]`); no extra threads or
  background work.

Legacy fallback: the `.sh` shims under
`crates/cortex-adapter-claude-code/hooks/` are retained for
Linux/macOS environments where `cortex-hook` is not on PATH. They
speak the same wire shape via `nc -U` / `socat` against the UDS and
honour the same `CORTEX_ADAPTER_DISABLE` kill-switch. The `.ps1`
shims are retired (phase 11x) and removed from the binary's
embedded source; `cortex-adapter-claude install` opportunistically
deletes a stale `.ps1` left by older installs.

Pre-phase-11x measurements on Windows showed `pwsh -NoProfile` cold
start dominating per-hook latency (~545 ms), with named-pipe
round-trip + daemon work only contributing ~90 ms. The native bin
collapses cold start to ~30–50 ms and a 14-hook turn from ~10.2 s
to ~0.9 s of adapter overhead. Baseline numbers and rerun procedure
in `crates/cortex-adapter-claude-code/benches/baseline-2026-05-06.txt`.

### Install / uninstall

```
cortex-adapters install claude-code            # standalone path — writes hooks + settings.json
cortex-adapters install claude-code --no-hooks # plugin path — daemon only, hooks owned by spec 18
cortex-adapters uninstall claude-code
cortex-adapters status
```

- **Install (default):** copies hook shims, patches `~/.claude/settings.json` to wire them up (idempotent — detects existing Cortex entries), creates `~/.cortex/` layout, writes a systemd/launchd/Windows-Service unit, starts the daemon.
- **Install `--no-hooks`:** keeps the daemon socket + adapter binary install but does **not** touch `~/.claude/hooks/` or `~/.claude/settings.json`. Spec-18 plugin users pick this path because their `claude plugin install cortex@hivellm-cortex` already wired hooks via the plugin's `hooks/hooks.json` — running both sides without `--no-hooks` would fire each event twice. `settings.json` stays byte-identical to its pre-install state when `--no-hooks` is set.
- **Uninstall:** reverts settings.json, stops the daemon, removes hook shims (leaves logs + overflow WAL unless `--purge`).
- **Status:** prints daemon PID, uptime, queue depth, overflow WAL size, last N publish errors.

> **Spec 18 supersedes the standalone hook install path for users who install the Cortex Claude Code plugin.** The plugin's `hooks/hooks.json` registers the same shim catalogue at plugin-install time, so a fresh laptop only needs `claude plugin install cortex@hivellm-cortex` (plus `cargo install --path crates/cortex-adapter-claude-code` for the daemon binary) followed by `cortex-adapter-claude install --no-hooks` to bootstrap the daemon. The standalone `cortex-adapter-claude install` (without `--no-hooks`) remains the canonical path for non-plugin contexts (CI, headless, custom installs).

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

Logs are structured JSON lines. The daemon exposes a tiny HTTP endpoint (`http://127.0.0.1:17020/metrics`) for Prometheus scraping when enabled.

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

## Error handling — no production-path panics (phase14i)

The adapter daemon's contract is that no production code path may
`panic!` / `unwrap()` / `expect(...)`. A single panic in the
dispatcher takes the entire user-session capture down.

Structured failure type lives in `cortex_adapter_claude_code::error::AdapterError`:

- `MalformedHook(String)` — incoming `HookFrame` failed validation;
  reason label `"malformed_hook"`.
- `MissingField(&'static str)` — required envelope field absent;
  label `"missing_field"`.
- `IpcWriteFailed(String)` — writing to the IPC transport (named
  pipe on Windows, Unix domain socket elsewhere) failed; label
  `"ipc_write_failed"`.
- `EnvelopeBuildFailed(String)` — canonical envelope construction
  failed; label `"envelope_build_failed"`.

`reason_label()` returns a short, low-cardinality string for the
`adapter_dispatch_errors_total{reason}` counter family.

Production-path migrations landed in this phase:

- `wal.rs::OverflowWal::append` / `drain` — recover from a
  poisoned `Mutex` instead of panicking; a panicking thread
  elsewhere in the daemon no longer drags the WAL down too.
- `publisher.rs::HttpPublisher::new` — falls back to
  `reqwest::Client::new()` with a WARN log when the per-publisher
  `Client::builder().timeout(...).build()` fails (TLS init
  catastrophe). The daemon's boot path never panics.
- `install.rs::patch_settings` — surfaces malformed `settings.json`
  shapes (root not an object, `hooks` not an object) as
  `InstallError::MalformedSettings(&'static str)` instead of
  `expect(...)`. The installer now fails cleanly with an
  actionable message.

The dispatcher's `dispatch` method has always returned
`HookResponse` (never `Result`); every internal error degrades to
`HookResponse::empty()` so the session never breaks. Phase14i
ships a 100-payload fuzz test
(`tests/dispatcher_fuzz.rs::dispatcher_survives_100_random_hook_payloads`)
that pins this structurally — a future refactor re-introducing
`unwrap()` on a frame field will surface as a fuzz failure.

CI gate: `.github/workflows/adapter-no-unwrap.yml` walks every
`*.rs` under `crates/cortex-adapter-claude-code/src/`, excludes
`#[cfg(test)] mod tests { ... }` blocks line-wise, and fails the
build on any `.unwrap()` / `.expect(` outside the SAFETY-tagged
allow-list.

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
