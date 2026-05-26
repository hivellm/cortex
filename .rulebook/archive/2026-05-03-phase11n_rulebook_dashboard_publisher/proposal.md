# Proposal: phase11n_rulebook_dashboard_publisher

## Why

Phase11m landed the dashboard push pipeline (FS watcher →
`DashboardEventBus` → `/v1/dashboard/stream` SSE). The watcher covers
`.rulebook/{tasks,handoff,decisions,knowledge}/**` — every path-backed
mutation reaches the GUI in ~250 ms.

**Audit of `@hivehub/rulebook` storage** (sibling repo
`../Rulebook/src/{core,memory}/`, 2026-05-03):

| Surface | Storage | Watcher coverage today |
|---|---|---|
| `rulebook_decision_*` | `.rulebook/decisions/<id>.md` + `<id>.metadata.json` (`decision-manager.ts`) | ✅ phase11m |
| `rulebook_knowledge_*` | `.rulebook/knowledge/<id>.md` + `<id>.metadata.json` (`knowledge-manager.ts`) | ✅ phase11m |
| `rulebook_learn_*` | `.rulebook/learnings/<id>.md` + `<id>.metadata.json` (`learn-manager.ts`) | ❌ not in watcher path |
| `rulebook_memory_*` | `.rulebook/memory/memory.db` (`better-sqlite3`, WAL) | ❌ DB-only, FS watcher cannot read rows |
| `rulebook_handoff_*` | `.rulebook/handoff/*.md` | ✅ phase11m |
| `rulebook_task_*` | `.rulebook/tasks/<id>/{proposal.md,tasks.md,.metadata.json}` | ✅ phase11m |

**Two real gaps** (not five):

1. **learnings** — file-backed but `.rulebook/learnings/` is not in the
   phase11m watcher path glob. One-line config fix.
2. **memory** — SQLite-only. mtime on `memory.db` jitters via WAL
   without committed-row changes; even when the watcher fires, it
   can't tell the GUI which entries appeared without reading the rows.

The original phase11n premise — "open an upstream issue in `hivellm/rulebook` and add `synap-sdk` (JS) to its handlers" — pollutes a generic third-party MCP package with Cortex-specific stream names + envelope shapes. The cleaner architecture: **Cortex owns the entire push path**. The watcher is already ours; the SQLite tail can be too. Zero coupling on Rulebook's release cadence, zero dep added to the npm package.

## What Changes

In this repo only (no Rulebook PR, no Synap PR):

1. **Extend the phase11m FS watcher** scope from
   `.rulebook/{tasks,handoff,decisions,knowledge}/**` to also cover
   `.rulebook/learnings/**`. New event kind `learning.added` on the
   `DashboardEventBus`. Emitted on watcher fire, dedupe via the
   existing 1024-id ring.

2. **Add a SQLite tail loop** in `cortex-api` that polls
   `.rulebook/memory/memory.db` at the same 250 ms cadence the
   watcher debounces at. Implementation:
   - Cache `(rowid_max, updated_at_max)` per memory table.
   - On each tick: `SELECT id, name, type, updated_at FROM memories WHERE rowid > $last_rowid ORDER BY rowid` (read-only handle, `?mode=ro` URI).
   - Emit one `memory.appended` event per new row through
     `DashboardEventBus`.
   - Reuse a single `rusqlite::Connection` per process with a
     `SQLITE_OPEN_READ_ONLY` flag so concurrent rulebook writes
     are never blocked.
   - Gate behind `CORTEX_DASHBOARD_MEMORY_TAIL=1` (default `1`);
     unset the env to disable when running without Rulebook.

3. **GUI hook** — `useDashboardStream` already dispatches
   `queryClient.invalidateQueries` per kind. Add the two new kinds
   (`learning.added`, `memory.appended`) to the dispatch map; no UI
   shape change.

4. **Smoke test** — write a row to a fixture `memory.db`, assert the
   tail loop emits the event within 1 s.

## Impact

- **Affected specs:** `docs/specs/21-dashboard-push.md` (Draft → Implemented; document the SQLite tail strategy).
- **Affected code:** `cortex-api/src/dashboard_consumer.rs`, `cortex-api/src/main.rs`, the phase11m FS watcher module (locate during §1), `gui/src/hooks/useDashboardStream.ts`.
- **No rulebook upstream change.** No `synap-sdk` dep added anywhere. No new npm release dependency.
- **No Synap upstream change.** SQLite tail uses local file I/O; the broker is not in this loop.
- **Breaking change:** NO. Additive on every surface; FS watcher still fires for everything it covers today.
- **User benefit:** memory mutations appear in the dashboard within ~1 s; learnings within ~250 ms (FS watcher cadence).

## Why this scope (and not the original)

The original §1 ("open an issue in `hivellm/rulebook` requesting a Synap publisher hook") forced the WRONG repo to take a hard dep on a Cortex-specific stream name (`cortex.events.dashboard`) and envelope shape (spec 21). Rulebook is a generic npm package. The right boundary: Rulebook writes to its own files / DB; Cortex (the consumer that cares about dashboard push) owns the read-side observation. Both surfaces in this revised proposal — FS watcher extension and SQLite tail — live entirely inside `cortex-api`.

## Source

Audit performed against `../Rulebook/src/{core,memory}/` on 2026-05-03; phase11m landing notes (`.rulebook/archive/2026-05-03-phase11m_dashboard_push_cache/`) for the existing watcher + bus contract.
