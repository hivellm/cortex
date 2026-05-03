# 21 — Dashboard push cache (SSE deltas via Synap)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 16, 20

## Goal

Replace polling-based dashboard refresh with a push channel: every mutation
to rulebook state (tasks, decisions, knowledge, memory, handoffs) emits a
typed delta event that reaches the GUI within ~1 s through a single SSE
endpoint on `cortex-api`. Polling drops to a slow safety net (300 s)
instead of being the active sync mechanism. Mirrors the timeline-stream
pattern (spec 16, `/v1/dashboard/timeline/stream`) and reuses the existing
Synap bus (spec 06 / 07).

## Scope

**In:**
- New Synap stream `cortex.events.dashboard` carrying typed delta events.
- Publisher inside the cortex MCP server (spec 20) so every tool call that
  mutates `.rulebook/` state emits an event.
- File-system watcher in `cortex-api` as a fallback for direct edits made
  outside the MCP (manual `Edit`, git checkout, external scripts).
- Consumer + SSE endpoint `/v1/dashboard/stream` on `cortex-api`.
- GUI hook that subscribes once, dispatches `queryClient.invalidateQueries`
  per event kind, and surfaces a connection-status pill.
- Lossy fan-out semantics — last event wins on slow client; the GUI does a
  resync `invalidateQueries` on every (re)connect.

**Out:**
- Persisting the delta stream beyond Synap's retention.
- Per-user filtering of events (the dashboard is local; everyone sees all).
- Push for `cortex.events.timeline` — already covered by spec 16.
- Bidirectional sync (GUI does not push state back over the stream).

## Inputs / Outputs

### Stream

| Stream                       | Direction | Carrier             |
|------------------------------|-----------|---------------------|
| `cortex.events.dashboard`    | producer → consumer | Synap        |

### Event envelope

```json
{
  "event_id": "01J...ULID",
  "kind": "task.changed",
  "entity_id": "phase11m_dashboard_push_cache",
  "summary": "status: pending → in-progress",
  "ts": "2026-05-02T23:45:00Z",
  "delta": { "status": "in-progress" },
  "source": "mcp" | "watcher"
}
```

| Field        | Type                  | Notes                                           |
|--------------|-----------------------|-------------------------------------------------|
| `event_id`   | `string` (ULID)       | Stable id; consumer dedupes on this.            |
| `kind`       | `string` (enum, see below) | Tagged variant.                            |
| `entity_id`  | `string`              | Task id / decision id / handoff filename / etc. |
| `summary`    | `string`              | Human-readable one-liner. Optional.             |
| `ts`         | RFC-3339 string       | UTC.                                            |
| `delta`      | object                | Optional minimal patch (e.g. `{ status }`).     |
| `source`     | `"mcp" \| "watcher"`  | Who emitted this.                               |

### Event kinds (v1)

| `kind`                | `entity_id` shape         | Triggers (MCP)                                 | Watcher path glob                  |
|-----------------------|---------------------------|------------------------------------------------|------------------------------------|
| `task.changed`        | `<task-id>`               | `rulebook_task_create/update/archive/delete`   | `.rulebook/tasks/**/{tasks,proposal}.md` |
| `handoff.appended`    | `<handoff-filename>`      | `/handoff` skill (writes `_pending.md`)        | `.rulebook/handoff/**`             |
| `decision.changed`    | `<decision-id>`           | `rulebook_decision_create/update`              | `.rulebook/decisions/**`           |
| `memory.appended`     | `<memory-id>`             | `rulebook_memory_save`                         | n/a (memory is DB-backed)          |
| `knowledge.added`     | `<knowledge-id>`          | `rulebook_knowledge_add`                       | `.rulebook/knowledge/**`           |

The watcher emits the same envelope as the MCP publisher; consumers cannot
distinguish them except via `source`. Duplicate `event_id` values (same
event seen from both paths) are dropped on the consumer side.

### SSE endpoint

```
GET /v1/dashboard/stream
Accept: text/event-stream
```

- First frame on subscribe: `event: hello` with body
  `{ "server_ts": "...", "lost_window": <bool> }`. `lost_window` is `true`
  when the consumer's broadcast subscriber lagged at least once since the
  last hello — signal for the GUI to do a full resync.
- Subsequent frames: `event: <kind>` with the envelope as JSON body.
- Keep-alive: `: ka` comment every 15 s (matches `timeline_stream`).
- Backpressure: lossy. The server uses `tokio::sync::broadcast`; slow
  clients get `RecvError::Lagged` and a fresh `hello` with
  `lost_window: true`.

### GUI integration contract

```ts
type DashboardEvent =
  | { kind: "task.changed";     entity_id: string; ts: string; delta?: Partial<Task> }
  | { kind: "handoff.appended"; entity_id: string; ts: string }
  | { kind: "decision.changed"; entity_id: string; ts: string; delta?: Partial<Decision> }
  | { kind: "memory.appended";  entity_id: string; ts: string }
  | { kind: "knowledge.added";  entity_id: string; ts: string };
```

| Event kind          | `queryClient.invalidateQueries` keys                |
|---------------------|------------------------------------------------------|
| `task.changed`      | `[connKey, "tasks", *]`                              |
| `handoff.appended`  | `[connKey, "handoffs", *]`                           |
| `decision.changed`  | `[connKey, "decisions", *]`, `[..., "decisions", entity_id]` |
| `memory.appended`   | `[connKey, "memory", *]`                             |
| `knowledge.added`   | `[connKey, "knowledge", *]`                          |

## Failure modes

| Failure                                | Behavior                                                |
|----------------------------------------|---------------------------------------------------------|
| Synap unreachable at startup           | Watcher path runs alone; SSE still works locally.       |
| Watcher fails to register              | Publisher path runs alone; manual edits invisible until next polling tick. |
| Subscriber lags                        | `RecvError::Lagged` → drop oldest, send `hello` with `lost_window: true`. |
| Duplicate event_id (MCP + watcher)     | Consumer dedupes by `event_id` over a sliding window of 1 000 ids. |
| GUI EventSource disconnects            | Exponential reconnect (cap 30 s); on reconnect, GUI fires global `invalidateQueries` for all dashboard keys. |

## Performance targets

| Metric                                     | Target  |
|--------------------------------------------|---------|
| MCP write → GUI invalidation (p50, local)  | < 250 ms |
| MCP write → GUI invalidation (p99, local)  | < 1 s   |
| Idle CPU / open dashboard tab              | ~0 (no polling once stream is up) |
| Memory per connected SSE client            | < 64 KiB |

## Telemetry

- `cortex_dashboard_stream_subscribers_total` (gauge)
- `cortex_dashboard_stream_events_emitted_total{kind, source}` (counter)
- `cortex_dashboard_stream_events_dropped_total{reason}` (counter)
- `cortex_dashboard_stream_lag_total` (counter — number of `Lagged` recoveries)

## Migration notes

- Polling intervals in the GUI move from 30 s to 300 s — a safety net that
  catches any window where the SSE stream was down. Not removed entirely.
- The MCP publisher is gated by env `CORTEX_DASHBOARD_PUBLISH=1` (default
  `1`); set to `0` to opt out for environments that do not run a Synap
  consumer locally.
- The file watcher is gated by env `CORTEX_DASHBOARD_WATCH=1` (default
  `1`).

## Cross-references

- Spec 16 (Dashboard) — overall view layout and existing endpoints.
- Spec 20 (MCP tool surface) — publisher hook points.
- Spec 06 / 07 — Synap bus pattern + worker integration.

## Verification

Manual checklist run end-to-end after a fresh `cargo run -p cortex-api`
+ `npm run dev` against this repo's `.rulebook/`.

1. Open the GUI Tasks view.
2. Header shows the green `stream` pill within 1 s of page load.
3. From a separate terminal, run a rulebook MCP write that lands in a
   `.rulebook/tasks/<id>/` directory — e.g. mark an item `[x]` in any
   `tasks.md`, or call `rulebook_task_update`.
4. Within 2 s, the affected row in the Tasks view reflects the change
   (status badge, counter, list position) without a page reload.
5. Archive a task via `rulebook_task_archive`. The row's status flips
   to `archived` within 2 s; the sidebar `tasks` counter decrements.
6. Stop `cortex-api`. Within ~5 s the header pill flips to amber
   `stream offline`. Restart the daemon. Pill returns to green;
   `lost_window: true` triggers a global `invalidateQueries` that
   refetches every dashboard view so the GUI re-syncs.

Automated coverage:

- `crates/cortex-api/tests/dashboard_stream.rs` —
  `stream_emits_hello_frame_then_published_event` exercises the SSE
  contract (hello frame + typed event).
- `crates/cortex-api/src/dashboard_watcher.rs::tests::watcher_emits_event_on_real_file_write`
  exercises the file → bus path against a real `tempdir`.
- `crates/cortex-api/src/dashboard_consumer.rs::tests::*` exercise the
  dedup ring + parse-error counters used by phase11n's Synap loop.
