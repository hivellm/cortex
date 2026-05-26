# Spec: Dashboard Health view

## ADDED Requirements

### Requirement: SSE health stream

cortex-api MUST expose `GET /v1/health/stream` as an SSE endpoint
emitting one `health` event every 5 seconds carrying the full
`HealthSnapshot { aggregator, freshness, divergence, versions,
config, canary_recent, generated_at }`.

The stream MUST emit a `heartbeat` event every 15 seconds so clients
can detect a stalled producer (matches the existing
`/v1/dashboard/timeline/stream` contract).

The snapshot MUST be capped at 64 KB; if the assembled payload is
larger, `findings[]` arrays MUST be truncated and `truncated: true`
set on the snapshot.

#### Scenario: stream emits at 5s cadence
Given a client subscribes to `/v1/health/stream`
When 12 seconds elapse
Then the client MUST have received at least 2 `health` events
     AND at least 0 `heartbeat` events.

### Requirement: GUI Health view layout

`gui/src/views/Health.tsx` MUST render the following sections in
order, each tied to one of the `/v1/health/*` endpoints:
1. **Overall banner** — traffic light + summary text driven by
   `health.overview().overall`.
2. **Subsystems grid** — one card per subsystem from
   `health.overview().subsystems[]`.
3. **Freshness table** — rows from `health.freshness()`, sorted
   by `gap_seconds` desc, colour-coded.
4. **Divergence table** — rows from `health.divergence()` where
   `severity != ok`.
5. **Version drift** — rendered only when
   `health.versions().all_in_sync == false`.
6. **Config audit** — findings from `health.config()` where
   `severity != ok`.
7. **Canary history** — last 24 h from
   `health.canaryHistory()` rendered as a Sparkline of pass/fail.

When the underlying endpoint returns 5xx or times out, the
corresponding section MUST display an empty-state message naming
the failed endpoint, NOT crash the whole view.

#### Scenario: degraded subsystem renders red
Given `/v1/health` returns a subsystem with `state: "down"`
When the user navigates to `/health`
Then the matching subsystem card MUST render with the red state pill
     AND the overall banner MUST read "1 subsystem down".

### Requirement: real-time updates via SSE

The Health view, the sidebar Health badge, and the topbar status
pill MUST all subscribe to `useHealthStream()` so the UI reflects
state changes without manual refresh.

The sidebar badge MUST display the count of subsystems whose state
is not `ok`.

The topbar pill MUST be visible from every view (Live Timeline,
Conversations, Decisions, etc.), not only from `/health`.

#### Scenario: topbar pill turns red when stack degrades
Given the user is viewing Live Timeline
And a subsystem transitions to `down`
When the SSE stream pushes the new snapshot
Then the topbar status pill MUST update from green to red within
     5 seconds without page refresh.

### Requirement: inspector drill-down

Clicking a subsystem card MUST open the existing Inspector aside
populated with:
- The full `extras` JSON pretty-printed inside a `<pre>` block.
- A "Copy as curl" button that copies a `curl` command hitting the
  subsystem's `/healthz` directly.

#### Scenario: copy-as-curl copies the right URL
Given the user clicks the cortex-adapter subsystem card
When the user clicks "Copy as curl" in the inspector
Then the clipboard MUST contain
     `curl http://127.0.0.1:17011/healthz` (the configured admin URL).

### Requirement: graceful degradation when health endpoints are down

When `/v1/health` itself is unreachable, the Health view MUST render
a single error banner ("cortex-api unreachable; start it with `cargo
run -p cortex-api`") and MUST NOT continue polling.

The topbar pill MUST display a grey dot in this case, with hover
text "health stream offline".
