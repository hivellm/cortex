# Proposal: phase11n_rulebook_dashboard_publisher

## Why

Phase 11m landed the dashboard push pipeline (file-system watcher →
`DashboardEventBus` → `/v1/dashboard/stream` SSE) and the local consumer
infrastructure (`crates/cortex-api/src/dashboard_consumer.rs`) for the
`cortex.events.dashboard` Synap stream. The watcher covers every
file-backed mutation, but DB-only writes (memory, knowledge that the
rulebook MCP persists to SQLite without touching `.rulebook/knowledge/`)
do not produce a filesystem event. They need a Synap publisher inside
the rulebook MCP server to reach the GUI in real-time.

The rulebook MCP lives in the external `@hivellm/rulebook` package
(npm) — not in this repo. The work to add `synap-sdk` publish calls
to the JS handlers must land there and be released. Once a publisher
exists, the consumer wired here in cortex-api forwards the events into
the same `DashboardEventBus` the watcher feeds, and the GUI gets push
updates for memory + DB-backed knowledge writes too.

## What Changes

In `@hivellm/rulebook` (this task's scope is the cross-repo coordination
+ the cortex-side wiring once it lands):

1. Add `synap-sdk` (JS) dependency to the rulebook MCP server package.
2. Inside each tool handler that mutates persistent state
   (`rulebook_task_*`, `rulebook_decision_*`, `rulebook_knowledge_*`,
   `rulebook_memory_*`, `rulebook_handoff_*`), publish a
   `DashboardEvent` envelope (spec 21) to stream
   `cortex.events.dashboard` after the mutation succeeds.
3. Gate via env `RULEBOOK_DASHBOARD_PUBLISH=1` (default `1`); allow
   `0` for local dev without Synap.
4. Reuse one Synap client per process; do not open a connection per
   call.

In this repo (cortex):

5. Wire the existing `dashboard_consumer.rs` (built in phase11m §4.3)
   to a real `synap-sdk` (Rust) pull loop in `cortex-api`. Spawn the
   loop alongside the watcher in `main.rs` startup; feed the same
   `DashboardEventBus`.
6. Smoke test: rulebook MCP publishes a `memory.appended` event;
   cortex-api SSE client receives it within 1 s.

## Impact

- Affected specs: `docs/specs/21-dashboard-push.md` (status: draft → implemented).
- Affected code: `@hivellm/rulebook` (external), `cortex-api/src/main.rs`, `cortex-api/src/dashboard_consumer.rs`.
- Breaking change: NO. Additive on both sides; the watcher path keeps working independently.
- User benefit: memory + DB-only knowledge writes appear in the dashboard within ~250 ms of the MCP call instead of needing a polling fallback.

## Blocked on

- `@hivellm/rulebook` releasing a version that publishes to Synap.
  Track via the rulebook repo's issue tracker; no ETA owned by this
  task.
