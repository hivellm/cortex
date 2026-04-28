# Spec: Pipeline stage metrics & freshness

## ADDED Requirements

### Requirement: per-stage event counters

Every pipeline stage (adapter.ipc, adapter.dispatcher, adapter.publisher,
ingestion.http, archive_loader, meili_loader, classifier-worker,
embedder-worker, fulltext-worker, graph-worker) MUST maintain monotonic
counters for events it processes, partitioned by event kind where
applicable.

Counter names MUST follow the convention `<stage>_<noun>_total{kind=...}`
(e.g. `adapter_ipc_frames_received_total{hook="PostToolUse"}`,
`ingestion_events_archived_total{kind="tool_call"}`).

Counters MUST never decrease and MUST survive across in-process restarts
only if the stage owns durable state (publisher's WAL); otherwise they
reset to zero on process restart and the stage's `since` timestamp
SHALL be exposed alongside.

#### Scenario: counter increments on each event
Given the adapter receives 3 PostToolUse frames in sequence
When the third frame finishes parsing
Then `adapter_ipc_frames_parsed_total{hook="PostToolUse"}` MUST equal 3.

### Requirement: per-stage last_event_ts

Every pipeline stage MUST stamp a `last_<event>_ts` (RFC-3339 string) for
each event kind it processes, updated atomically with the corresponding
counter.

The timestamp MUST be readable from the stage's `/healthz extras` so the
cortex-api aggregator can fan out without a separate /metrics scrape.

#### Scenario: stale stage surfaced as gap
Given the adapter publisher's `last_publish_ok_ts` is 90 seconds in the past
When `GET /v1/health/freshness` is called on cortex-api
Then the response MUST contain a row
     `{ key: "adapter.publisher.tool_call", gap_seconds: ~90 }`.

### Requirement: cortex-api freshness aggregator

`GET /v1/health/freshness` MUST return a flat map keyed by
`<stage>.<kind>` whose values include `last_event_ts` and a derived
`gap_seconds` (now − last_event_ts, in whole seconds).

Rows whose `gap_seconds > 60` MUST carry `severity: warn`; rows whose
`gap_seconds > 300` MUST carry `severity: critical`.

The endpoint MUST cache for 2 seconds (same budget as `/v1/health`).

#### Scenario: critical severity past 5 minutes
Given a stage's last_event_ts is 350 seconds in the past
When `GET /v1/health/freshness` is called
Then the row's `severity` MUST be `critical`.

### Requirement: cortex-api divergence aggregator

`GET /v1/health/divergence` MUST compare each pair of adjacent-stage
counters (ipc → dispatcher → publisher → ingestion → archive_loader)
and report rows of shape:
```
{ pair: "(adapter.ipc.PostToolUse, adapter.publisher.tool_call)",
  upstream_count: U, downstream_count: D,
  delta: U - D,
  delta_growth_60s: <U-D now> - <U-D 60s ago>,
  severity: ok | warn | critical }
```

Severity rules: `delta_growth_60s > 10` → `warn`;
`delta_growth_60s > 50` → `critical`. A static delta is fine — it's a
spike that signals a regression.

#### Scenario: silent drop detection
Given upstream ipc counter increments by 100 PostToolUse in 60s
And downstream publisher counter increments by only 0 tool_call in the same window
When `GET /v1/health/divergence` is called
Then the matching row's `severity` MUST be `critical`
     AND `delta_growth_60s` MUST be ≥ 90.
