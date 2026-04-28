# Proposal: phase8g_dashboard_health_view

## Why

phase8a–8f produce rich JSON over `/v1/health/*`, but the user
realistically opens the GUI before the terminal. Surfacing the
health system in the existing dashboard, with the same visual
language as Live Timeline / Conversations / Decisions, makes it
the first thing the user notices when something is off.

The 2026-04-28 incident's user-visible signal was "tool_calls aren't
showing in the timeline" — but the reason was buried 5 layers down.
A Health view that reads `/v1/health`, `/v1/health/freshness`,
`/v1/health/divergence`, `/v1/health/versions`, `/v1/health/config`
and renders red/yellow/green indicators per subsystem would have
made the cause obvious at a glance: "adapter publisher: 0 tool_calls
in last 30 min — divergence with adapter.ipc 850". One look,
not two hours of investigation.

## What Changes

1. NEW GUI route `/health` rendered by `gui/src/views/Health.tsx`.
   Layout:
   - Top banner: overall traffic light (green / yellow / red) +
     "stack healthy" / "1 subsystem degraded" / "2 subsystems down"
   - Subsystems grid (one card per crate):
     - Name, state pill, version (sha + build_ts), uptime
     - Sparkline of `last_event_ts` lag over the past 20 min
     - Click-through expands to show full `extras` payload
   - Freshness table (from /v1/health/freshness): rows sorted by
     gap_seconds desc, colour-coded
   - Divergence table (from /v1/health/divergence): rows where
     severity != ok
   - Version drift section (from /v1/health/versions): rows when
     all_in_sync = false
   - Config audit (from /v1/health/config): findings with
     severity != ok
   - Canary history (from cortex-api `/v1/health/canary/history`):
     last 24 h success / failure timeline

2. Sidebar entry "Health" with badge showing critical issue count.
   Updates via SSE so the GUI doesn't poll.

3. NEW SSE stream `GET /v1/health/stream` that pushes a
   `HealthSnapshot` envelope every 5 s. Reuses the existing
   `useSSE` hook from Live Timeline.

4. Inspector panel: clicking a subsystem opens the same Inspector
   Aside the timeline uses, populated with the full `extras` JSON
   pretty-printed and a "Copy as curl" button for the underlying
   `/healthz` endpoint.

5. Topbar status pill on EVERY page: a tiny indicator showing
   overall health (green/yellow/red dot) so the user can't miss it
   while browsing other views. Clicking jumps to /health.

## Impact

- Affected specs: NEW `specs/health_view/spec.md`.
- Affected code:
  - NEW `gui/src/views/Health.tsx`
  - `gui/src/App.tsx` — route + sidebar entry + topbar status pill
  - `gui/src/lib/api.ts` — typed clients for /v1/health/*
  - `gui/src/styles.css` — new card styles
  - NEW `crates/cortex-api/src/health/stream.rs` — SSE endpoint
  - `crates/cortex-api/src/dashboard.rs` — wire route
- Depends on: phase8a, 8b, 8c, 8d, 8e, 8f.
- Breaking change: NO (additive view).
- User benefit: stack health is visible in the GUI without leaving
  the dashboard. The badge + topbar pill make broken state hard to
  miss; the inspector gives drill-down with one click.
