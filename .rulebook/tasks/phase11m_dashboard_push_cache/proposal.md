# Proposal: phase11m_dashboard_push_cache

## Why

Dashboard data (tasks, handoffs, decisions, memory) reaches the GUI through
`refetchInterval: 30_000` polling on every react-query. The server-side
`tasks_loader` already keeps a TTL+mtime cache, but the GUI has no push
channel — so every viewer pays the round-trip every 30s, the screen lags
behind real changes by up to 30s, and the file-system watcher work the
mtime check does is invisible to the client.

Synap is already the ingestion event bus (`cortex.events.enriched/graphed/
invalid`) and `cortex-api` already hosts an SSE stream for the timeline
(`/v1/dashboard/timeline/stream`). The same pattern can carry dashboard
deltas: workers/MCP tools that mutate `.rulebook/` state publish a delta
event to `cortex.events.dashboard`; `cortex-api` re-emits over SSE; the GUI
invalidates the matching react-query cache key on receipt. Polling shrinks
to a slow safety net (e.g. 5 min) instead of the active sync mechanism.

## What Changes

1. New Synap stream `cortex.events.dashboard` carrying typed delta events
   (`task.changed`, `handoff.appended`, `decision.changed`,
   `memory.appended`, `knowledge.added`).
2. Publishers wired into the MCP server tool handlers
   (`rulebook_task_create/update/archive`, `rulebook_decision_create/update`,
   `rulebook_knowledge_add`, `rulebook_memory_save`, etc.) so every mutation
   that crosses the MCP boundary emits a delta.
3. File-system watcher (`notify` crate) on `.rulebook/tasks/**` and
   `.rulebook/handoff/**` as a fallback for direct edits made outside the
   MCP — emits the same events into the same stream.
4. `cortex-api` consumer + SSE endpoint `/v1/dashboard/stream` proxying the
   stream to browsers, mirroring `timeline_stream` (keep-alive, 500 ms
   poll, lossy fan-out — last event wins on slow client).
5. GUI hook `useDashboardStream` connecting once on mount, dispatching
   `queryClient.invalidateQueries` per event type. Polling intervals drop
   to a slow safety net (300 s).
6. Optional payload (delta body) — events carry the changed entity id +
   minimal summary so the GUI can patch the cache without a re-fetch when
   the change is small (`task.status` flip, single `tasks.md` checkbox).

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (add stream contract);
  `docs/specs/07-graph-writer.md` (no change — reference only); new spec
  `docs/specs/21-dashboard-push.md` for the stream/event schema.
- Affected code:
  - `crates/cortex-api/src/dashboard.rs` — new SSE endpoint + consumer.
  - `crates/cortex-api/src/tasks_loader.rs` — accept push invalidation
    signal; keep mtime fallback.
  - `cortex-mcp-server/src/tools.rs` — emit on every task/decision/
    knowledge/memory mutation.
  - `gui/src/lib/api.ts` + new `gui/src/hooks/useDashboardStream.ts`.
  - `gui/src/views/Tasks.tsx`, `Handoffs.tsx`, `Decisions.tsx`,
    `Memory.tsx` — wire the hook, slow polling.
- Breaking change: NO. Endpoint is additive; polling stays as fallback.
- User benefit: dashboard updates within ~1 s of any rulebook write,
  no manual refresh; lower steady-state CPU on the API for idle dashboards
  (no 30 s wakeups when nothing changed).
