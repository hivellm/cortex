# Spec: Dashboard retention view

## ADDED Requirements

### Requirement: Sweep history endpoint

`cortex-api` MUST expose `GET /v1/retention/sweeps?limit=N&since=RFC3339`
returning recent `retention_sweeps` rows merged with their per-stage
breakdown from `tier_transitions_json`. The default `limit` MUST be 50.

#### Scenario: response shape carries per-stage counters
Given three sweeps that ran today (one each of sweep, rollup, cas-vacuum)
When the route is queried with `limit=10`
Then the response array MUST contain three items
And each item MUST contain a `stages` object keyed by stage name
And each stage MUST carry numeric counters appropriate to its kind.

### Requirement: Retention state endpoint

`GET /v1/retention/state` MUST return per-collection sizes, Parquet
archive bytes by age bucket (`le_30d`, `30d_to_365d`, `gt_365d`), Meili
index document counts, `cas_blobs` total bytes and rows, and the next
scheduled run per sweep type (or `"never"` when no schedule exists).

#### Scenario: state reports a 30-day archive bucket
Given the local archive contains 15 hourly files in the last 30 days
When the route is queried
Then `archive_bytes.le_30d` MUST be > 0
And `archive_bytes.gt_365d` MUST equal 0 on a fresh install.

### Requirement: Retention tab

The GUI MUST render a "Retention" tab between "Memory" and "Tweaks". The
tab MUST contain: a per-sweep status header row, a time-series chart of
reclaimed bytes per day for the last 30 days, a sortable breakdown
table, a live log strip filtered to `retention.*` events, and a red
failure banner when any sweep type has two consecutive failures.

#### Scenario: failure banner appears after two failures
Given the last two `retention_sweeps` rows for `parquet_rollup` have `status='failed'`
When the operator opens the Retention tab
Then a red banner MUST be visible at the top
And it MUST display the most recent `last_error`.

### Requirement: Live log

The live log strip MUST subscribe to `cortex.live.<repo>` SSE and only
display events whose `kind` starts with `retention.`. The list MUST cap
at 100 entries.

#### Scenario: a sweep emits a live event
Given a sweep emits `kind="retention.tier_transition"`
When the operator is on the Retention tab
Then the event MUST appear in the live log within 2 s
And events on other `kind` prefixes MUST NOT appear.
