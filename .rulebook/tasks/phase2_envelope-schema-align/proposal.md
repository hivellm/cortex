# Proposal: phase2_envelope-schema-align

## Why

Spec 04 (`cortex-core`) defines the canonical event schema:
`Envelope` with strict ULID `event_id`, `schema_version="1"`,
`occurred_at` RFC-3339, `kind` ∈ {turn, tool_call, agent_call, memory,
decision, analysis, law_violation, artifact}, a typed `Context` with
`platform` required, and a per-`kind` `payload` validated against
`schemas/kinds/*.schema.json`.

Spec 10 (`cortex-adapter-claude-code`) shipped its own ad-hoc
`ClaudeEvent` shape with `ts` (ms epoch), dotted `kind` strings
(`turn.user`, `tool_call.requested`, …), top-level
`session_id`/`turn_id`/`tool_call_id`/`adapter`/`source`/`orphan`,
`redacted_payload` instead of `payload`, and `redactions` as a
number. Posting it at the canonical `/v1/events/batch` triggers
~10 different schema errors per event.

End-to-end live test (`claude --plugin-dir` session → daemon →
`cortex-ingestion`) confirms: hooks fire, daemon dispatches,
publisher posts — and ingestion rejects 100% with 422 because the
adapter envelope is unrelated to the canonical envelope. Capture
silently overflows to `~/.cortex/overflow.wal` even when ingestion
is up. **The plugin's headline promise of "events flow into Cortex"
is false today on every install, not just Windows.**

## What Changes

The adapter is the side that drifted. Spec 04 owns the wire
contract. The adapter becomes a thin translator: hooks → canonical
`cortex_core::events::Envelope`. Concretely:

### Envelope-level alignment

- `event_id`: keep — `ulid::Ulid::new().to_string()` already produces
  Crockford ULIDs that match `^[0-9A-HJ-KM-NP-TV-Z]{26}$`.
- `ts: i64` → `occurred_at: String` (RFC 3339, UTC, ms precision via
  `chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)`).
- Add `schema_version: "1"`.
- Move `adapter: "claude-code"` → `tool: "claude-code"` (canonical
  field name, same enum value).
- `session_id`: keep — already a ULID-shaped string from the
  `SessionManager` (verify it matches the regex; tighten the
  generator if not).
- Add `stream: "live"` (always, for real-time hook capture).
- `model`: optional, populate from `CLAUDE_MODEL` env when present.
- `redacted_payload` → `payload` (the canonical envelope's `payload`
  is the payload itself; "redacted" is implicit).
- `redactions: u32` → `redactions: Vec<String>` (token-level entries
  the redactor returned, formatted `"secret:<class>:<token>"`).
- Add `context: Context { platform, repo, branch, commit, cwd, user,
  ide, extras }`. The current `source` blob's fields (`adapter`,
  `cwd`, `repo`, `model`) reshape into `context`.
- Drop `adapter` / `source` top-level fields (covered by `tool` +
  `context`).
- Move `orphan` / `turn_id` / `tool_call_id` into
  `context.extras["claude_code"]` so correlation data still rides
  along but doesn't pollute the canonical envelope.

### Kind mapping

| Hook (today)        | Today's `kind` (dotted)     | Canonical `kind` | Notes |
|---------------------|------------------------------|------------------|-------|
| UserPromptSubmit    | `turn.user`                  | `turn`           | payload `{user_message: <prompt>, assistant_message: null}`; `Stop` later emits a follow-up event we treat as a `turn` update via `parent_event_id` |
| PostToolUse         | `tool_call.completed`        | `tool_call`      | payload `{tool_name, input, output: {stdout, stderr, exit_code}, outcome: "success"\|"error", duration_ms}` |
| SubagentStop        | `turn.subagent_complete`     | `agent_call`     | payload subset matching `agent_call.schema.json` |
| PreToolUse          | `tool_call.requested`        | _drop_           | Sync law-check only — no published event. The eventual PostToolUse carries the canonical record. |
| Stop                | `turn.session_stop`          | _drop_           | Session lifecycle is implicit; no canonical event. |
| SessionStart        | `turn.session_start`         | _drop_           | Implicit; the first `turn` opens the session. |
| Notification        | `event.notification`         | _drop_           | No canonical kind; recorded only in adapter-side metrics if needed. |

The four "drop" hooks still produce the **sync HookResponse**
(law-check verdicts, additional-context bundles for pre-thinking)
that the shim needs. They simply stop putting frames on the
publisher queue.

### Code changes

- Replace `crates/cortex-adapter-claude-code/src/events.rs::ClaudeEvent`
  with a wrapper that yields `cortex_core::events::Envelope` directly
  (re-export the Rust type from `cortex-core`).
- `build_event` returns `Option<Envelope>` — `None` for the dropped
  hook kinds — so the publisher only sees publishable events.
- `dispatcher.dispatch` keeps calling `maybe_sync_path` for every
  hook; the publisher path becomes `if let Some(env) = event { ... }`.
- The publisher's `enqueue` / `flush_locked` switch from `ClaudeEvent`
  to `cortex_core::events::Envelope` (just a type swap; the wire
  shape was already wrapped in `{"events": [...]}` matching spec 04).
- WAL replay reads the new shape; an automatic migration drains any
  legacy `ClaudeEvent` lines on startup (best-effort: log + drop).
- Tests update: every fixture under
  `cortex-adapter-claude-code/tests/` and the inline `events.rs`
  tests assert canonical-envelope fields.

## Impact

- Affected specs: spec 10 (envelope mapping section rewritten —
  reference spec 04 instead of redefining; remove the dotted-kind
  vocabulary), spec 04 (no schema change, just clarification of the
  adapter binding).
- Affected code: `cortex-adapter-claude-code/src/events.rs`,
  `dispatcher.rs`, `publisher.rs` (type swap), `wal.rs` (replay
  drains legacy lines), every test that constructs a `ClaudeEvent`.
- Affected runtime artifacts: `cortex-adapter-claude.exe` rebuild
  required after merge; existing `~/.cortex/overflow.wal` lines
  written under the old shape get migrated-or-dropped on first
  startup of the new build.
- Breaking change: YES at the wire layer between the adapter daemon
  and `cortex-ingestion`, but no public adapter API exists —
  consumers (the plugin) only talk to the daemon over a Unix
  socket / named pipe in the existing `HookFrame` shape, which is
  unchanged. Operator-visible: a one-time WAL replay-drop log on
  daemon startup the first time the new build runs.
- User benefit: the plugin install promise — "hook events flow into
  Cortex" — actually holds. `claude plugin install
  cortex@hivellm-cortex` + `cortex-adapter-claude daemon` +
  `cortex-ingestion` produces archived NDJSON in `~/.cortex/archive/`
  and increments `cortex_events_received` on every Claude Code turn.
