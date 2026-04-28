# Spec: Config coherence

## ADDED Requirements

### Requirement: per-surface config readers

`cortex-doctor` MUST expose pure-function readers for each config
surface — `.env`, `adapter.toml`, `cortex-plugin/.mcp.json`,
`cortex-plugin/hooks/hooks.json` — each returning a typed struct or a
`ReadError` describing precisely which file and line failed.

The readers MUST not require the corresponding service to be running;
config audit is a static-analysis pass.

#### Scenario: missing config file is reported, not panicked
Given `~/.cortex/adapter.toml` does not exist
When `cortex-doctor::read_adapter_toml()` is called
Then it MUST return `Err(ReadError::NotFound { path: "..." })`
     AND MUST NOT panic.

### Requirement: live-port reader

`cortex-doctor` MUST expose a function that returns every TCP port in
LISTEN state on the local loopback (`127.0.0.1` / `::1`), with the
owning pid and process name. Implementation uses `netstat -ano` on
Windows and `ss -tlnp` on Linux.

#### Scenario: listening port is detected
Given a process listens on `127.0.0.1:17010`
When `cortex-doctor::live_listening_ports()` runs
Then the returned vector MUST contain an entry with `port: 17010`.

### Requirement: coherence checks

`cortex-doctor::audit()` MUST run at minimum the following checks and
emit one `Finding` per issue:

1. Every `*_URL` env value MUST parse as a URL with an explicit port.
2. Every `*_URL` env value's host:port MUST appear in the live-port
   reader's result.
3. `adapter.toml.endpoint` MUST string-equal `CORTEX_INGESTION_URL`
   (after URL normalization).
4. `adapter.toml.api_endpoint` MUST string-equal `CORTEX_API_URL`.
5. `.mcp.json mcpServers.cortex.env.CORTEX_API_URL` MUST equal
   `.env CORTEX_API_URL`.
6. `hooks.json hooks` MUST register all 7 canonical Claude Code hook
   types: UserPromptSubmit, PreToolUse, PostToolUse, Stop,
   SubagentStop, SessionStart, Notification.
7. Each `*_URL` MUST resolve to a `/healthz` responding within 1500 ms.

Failures of checks 1–5 are `severity: critical`.
Failure of check 6 is `severity: warn` (some hooks may be intentional).
Failure of check 7 is `severity: warn` if the service is in `degraded`
state, `critical` if no response at all.

#### Scenario: port-mismatch detection (the 2026-04-28 bug)
Given `adapter.toml.endpoint = "http://127.0.0.1:15010"`
And nothing listens on port 15010
And `CORTEX_INGESTION_URL = "http://127.0.0.1:17010"`
When `cortex-doctor::audit()` runs
Then the audit MUST contain a `Finding { severity: critical,
     source: "adapter.toml", message: "endpoint :15010 not listening
     (CORTEX_INGESTION_URL says :17010)" }`.

#### Scenario: hook missing from hooks.json
Given `hooks.json` registers UserPromptSubmit and Stop only
When `cortex-doctor::audit()` runs
Then the audit MUST contain `Finding { severity: warn,
     source: "hooks.json", message: "missing hook: PreToolUse,
     PostToolUse, SubagentStop, SessionStart, Notification" }`.

### Requirement: cortex-api /v1/health/config endpoint

`GET /v1/health/config` MUST run the same `cortex-doctor::audit()`
server-side (using cortex-doctor as a library dep) and return the
`ConfigAudit` as JSON.

The endpoint MUST cache results for 10 seconds (config rarely changes).

#### Scenario: GUI consumes audit JSON
Given the GUI calls `GET /v1/health/config`
When any `Finding` has `severity: critical`
Then the response MUST surface that finding so the GUI can render
     a red banner with the offending file/line.

### Requirement: CLI exit codes

`scripts/doctor-config.bat` and `.sh` MUST exit with code `0` when
all findings are `ok`, `1` when any are `warn`, and `2` when any are
`critical` so CI gates can use it as a binary signal.
