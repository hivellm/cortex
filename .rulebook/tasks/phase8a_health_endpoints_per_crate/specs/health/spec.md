# Spec: Health endpoints

## ADDED Requirements

### Requirement: Shared health report shape

The `cortex-health` crate SHALL expose a `SubsystemStatus` type with fields
`name: String`, `state: enum { ok, degraded, down }`, `latency_ms: u64`,
`last_error: Option<String>`, `version: String`, `since: String` (RFC-3339).

A `HealthReport` SHALL bundle `overall: enum { ok, degraded, down }`,
`subsystems: Vec<SubsystemStatus>`, `checked_at: String` (RFC-3339).

`HealthReport::aggregate` MUST compute `overall = down` when any subsystem is
`down`, `degraded` when any is `degraded` (and none is down), otherwise `ok`.

#### Scenario: aggregate picks the worst state
Given two subsystems where A is `ok` and B is `degraded`
When `HealthReport::aggregate([A, B])` runs
Then the report's `overall` MUST be `degraded`.

#### Scenario: aggregate prefers down over degraded
Given three subsystems with states `ok`, `degraded`, `down`
When `HealthReport::aggregate(...)` runs
Then `overall` MUST be `down`.

### Requirement: per-crate /healthz endpoint

Every long-running Cortex binary (cortex-api, cortex-adapter-claude-code,
cortex-ingestion, cortex-classifier-worker, cortex-embedder-worker,
cortex-fulltext-worker, cortex-graph-worker) MUST expose `GET /healthz`
returning its own `SubsystemStatus` as JSON within 200 ms under nominal load.

The endpoint MUST never return 5xx; transport-level errors are encoded as
`state: down, last_error: <message>` with HTTP 200.

The endpoint MUST include an `extras` object with crate-specific signals
(e.g. adapter reports `publisher_queue_depth`, `wal_bytes`,
`last_publish_ok_ts`, `ipc_pipe_alive`).

#### Scenario: adapter degraded when publisher stalled
Given the adapter has not successfully published an envelope for 90 seconds
When `GET /healthz` is called
Then the response MUST contain `state: "degraded"` and
     `extras.last_publish_ok_ts` MUST be older than 60 seconds.

#### Scenario: adapter down when IPC pipe is closed
Given the adapter's named pipe has not bound (or has dropped)
When `GET /healthz` is called
Then the response MUST contain `state: "down"` and
     `extras.ipc_pipe_alive` MUST be `false`.

### Requirement: cortex-api aggregator

`GET /v1/health` on cortex-api MUST fan out to every known subsystem's
`/healthz` in parallel, with a per-probe timeout of 1500 ms, and return
the aggregated `HealthReport`.

The handler MUST cache the `HealthReport` for 2 seconds; subsequent calls
within that window return the cached value without re-probing downstream.

External dependencies (Vectorizer, Nexus, Meili, Synap) MUST appear as
subsystems in the report, even though they are not Cortex-owned crates.

A subsystem whose probe times out or returns a non-2xx response MUST be
recorded as `state: down` with `last_error: <reason>`.

#### Scenario: aggregator surfaces a downstream timeout
Given the adapter `/healthz` does not respond within 1500 ms
When `GET /v1/health` is called
Then the response's `subsystems[adapter].state` MUST be `down` with
     `last_error` containing the word "timeout".

#### Scenario: aggregator caches results within 2 seconds
Given `GET /v1/health` was just called and produced a HealthReport
When a second `GET /v1/health` arrives 1 second later
Then the second response MUST equal the first byte-for-byte and
     downstream `/healthz` endpoints MUST NOT have been hit a second time.

### Requirement: CLI exit codes

`scripts/health.bat` (and `scripts/health.sh`) MUST exit with code `0` when
`overall=ok`, `1` when `overall=degraded`, and `2` when `overall=down`,
so CI gates can treat it as a binary pass/fail.

#### Scenario: script exits non-zero on degraded
Given `/v1/health` returns `overall: degraded`
When `scripts/health.bat` is executed
Then its exit code MUST be `1`.
