# Proposal: phase2f_dashboard_sse_stream_and_auth

## Why

Today the dashboard polls `/v1/dashboard/timeline/recent` every 5 seconds — a stand-in for the real streaming surface in `phase2_dashboard` §1.3 (`GET /v1/dashboard/timeline/stream` SSE). Polling has three downsides: a 5-second worst-case delay before a captured event becomes visible, wasted bandwidth/CPU when nothing changes, and no way to communicate stream health (the design's "● connected" pill loses meaning when there is no actual connection).

`/v1/dashboard/*` also ships completely unauthenticated. Spec 16 §Auth and `phase2_dashboard` §2 require a `Authorization: Bearer <api_key>` middleware before the dashboard can be exposed beyond `127.0.0.1`.

Source: `phase2_dashboard/tasks.md` items 1.3 + 2.1 + 2.2.

## What Changes

### SSE timeline stream
- New `/v1/dashboard/timeline/stream` endpoint emitting `event: timeline\ndata: {...envelope-shape...}\n\n` per captured event, plus a periodic `event: heartbeat\ndata: {}\n\n` every 15 seconds.
- Reconnect-friendly: every event carries `id: <event_id>`. On reconnect, the client sends `Last-Event-ID`; the server replays any events newer than that id (best-effort — falls back to "live" stream when the requested id is older than the in-memory window).
- Source: subscribe to the same lane the polling endpoint reads from. When new envelopes are appended, fan them out to all SSE subscribers.
- Backpressure: drop the slowest 25% of subscribers when total subscribers > 100, surfacing a `cortex.dashboard.sse.dropped` metric.
- Filters via query params (`?repo`, `?session_id`, `?kind`) — same names as `/timeline/recent`.

### Auth
- New `Authorization: Bearer <api_key>` middleware applied to every `/v1/dashboard/*` route (including the SSE stream).
- Keys live in a SQLite table managed by the `cortex-api` binary; minted via a new sub-command `cortex-api admin issue-api-key --scope dashboard`.
- 401 response body matches the existing `/v1/query` shape: `{ "reason": "missing_or_invalid_api_key" }`.
- Renderer: a tiny `useApiKey()` hook reads from `localStorage` (`cortex.api_key`); on 401 it shows a modal asking for the key.

### GUI integration
- New `gui/src/lib/useSSE.ts` hook — opens an `EventSource`, yields events to a callback, handles reconnect with exponential backoff (1s, 2s, 5s, 10s, 30s).
- Timeline view replaces its `useQuery` with `useSSE` for live data; `useQuery(/timeline/recent)` is used only for the initial backfill.
- Header status pill (already in place) flips to "stale" when SSE has not delivered a heartbeat for > 30 s.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (mark §Auth as 🟢 once shipped); `phase2_dashboard` §1.3 + §2 (close those checklist items).
- Affected code: `crates/cortex-api/src/dashboard.rs` (new SSE handler, new auth middleware), `crates/cortex-api/src/bin/admin.rs` (new sub-command), `crates/cortex-api/src/storage/api_keys.rs` (new module), `gui/src/lib/useSSE.ts` (new), `gui/src/views/Timeline.tsx`, `gui/src/lib/api.ts` (Authorization header on every fetch).
- Breaking change: YES — every existing dashboard request now requires an Authorization header. Bootstrap doc explains how to mint and configure the key.
- Depends on: nothing.
- User benefit: events appear in the GUI within ~100 ms of capture; the dashboard becomes safe to expose on a multi-user host.
