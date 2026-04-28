# Spec: Synthetic end-to-end canary

## ADDED Requirements

### Requirement: golden frame fixtures match real claude-code shape

Each fixture under `crates/cortex-doctor/fixtures/<hook>_<flavor>.json`
MUST be a verbatim sample of a real claude-code stdin payload for the
corresponding hook, with PII redacted.

Fixtures MUST exercise the regression vectors from 2026-04-28:
- Pretty-printed JSON (whitespace between top-level fields).
- Multi-line strings inside payload fields, encoded with JSON `\n`.

Each fixture MUST have a sibling `<name>_expected.json` describing the
canonical envelope shape the dispatcher should produce.

#### Scenario: pretty-printed PostToolUse fixture round-trips
Given the fixture `post_tool_use_bash.json` (pretty-printed JSON)
When the dispatcher processes a frame derived from the fixture
Then the produced envelope MUST equal the corresponding
     `post_tool_use_bash_expected.json` byte-for-byte after
     normalising volatile fields (event_id, occurred_at).

### Requirement: canary subcommand round-trips through real pipe

`cortex-doctor canary --hook=<HookKind>` MUST connect to the actual
named pipe / unix socket the daemon binds, write a fixture-based
frame whose tool_name is replaced with `Canary-<ulid>`, read the
response, then poll the archive until the same `Canary-<ulid>`
appears or a configurable deadline (default 10 s) elapses.

The subcommand MUST exit with:
- `0` on success (envelope observed in archive within deadline)
- `2` on timeout (no envelope in archive by deadline)
- `1` on transport error (pipe not responding, archive HTTP down)

#### Scenario: round-trip success
Given the daemon is running and healthy
When `cortex-doctor canary --hook=PostToolUse` is invoked
Then the subcommand exits with code `0` within the deadline
     AND the canary envelope MUST be visible in
     `GET /v1/dashboard/timeline/recent?kind=tool_call`.

#### Scenario: timeout when adapter drops the frame
Given the adapter has a regression that drops PostToolUse silently
And the daemon's IPC handler returns `{}` but no envelope is published
When `cortex-doctor canary --hook=PostToolUse` is invoked
Then the subcommand MUST exit with code `2`
     AND its stderr MUST name the missing marker id.

### Requirement: cortex-api background canary runner

When `[canary].enabled = true` in `~/.cortex/cortex.toml`, cortex-api
MUST spawn a `canary_runner` background task at boot.

The runner MUST invoke the canary logic on the configured interval
(default 300 s), append every result (`{ ts, hook, marker_id,
outcome, latency_ms }`) to `~/.cortex/canary-history.jsonl`, and on
failure emit a `law_violation` envelope using the same path as
phase8e (`law_id: "canary-<hook>"`, severity: `critical`).

#### Scenario: failure emits a violation envelope
Given the canary runner is enabled
And a tick observes a timeout from the canary subcommand
When the runner records the failure
Then a `law_violation` envelope MUST be POSTed to cortex-ingestion
     AND its `payload.law_id` MUST equal `"canary-PostToolUse"` (or
     the relevant hook).

### Requirement: history file is append-only

The runner MUST never truncate `canary-history.jsonl`; entries are
append-only with one JSON object per line. Operators read it via
`tail` to inspect recent canary outcomes.

#### Scenario: history persists across restarts
Given the runner has appended N entries to canary-history.jsonl
When cortex-api restarts and the runner spawns again
Then the file MUST still contain the N pre-restart entries
     AND new entries MUST be appended after them.
