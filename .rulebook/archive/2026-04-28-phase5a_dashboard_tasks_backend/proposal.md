# Proposal: phase5a_dashboard_tasks_backend

## Why

The Electron GUI surfaces every domain we capture (events, decisions, laws, analyses, tools, graph) but has no view for the Rulebook task pipeline itself — yet the `.rulebook/tasks/` (active) + `.rulebook/archive/` (archived, 39 dirs at the time of writing) directories are the most concrete record of what has shipped versus what is in flight. Today the only way to see project progress is to `ls` those directories or run `rulebook_task_list` from the MCP, both of which are invisible to anyone using the dashboard.

`cortex-api` has no `/v1/dashboard/tasks*` endpoints. `dashboard.rs` already exposes `overview`, `decisions`, `laws`, `analyses`, `tools/stats`, `sessions`, etc., reading from the same lanes the GUI consumes — but tasks live on the filesystem (`.rulebook/tasks/<id>/{proposal.md,tasks.md,.metadata.json}` and `.rulebook/archive/<date>-<id>/{...}`), not in the envelope store. They need their own loader + routes before the GUI can render them.

This task does the backend half only. The frontend Tasks view is split into `phase5b_gui_tasks_view` so each PR stays one-subsystem.

Source: existing `dashboard.rs` route layout; `mcp__rulebook__rulebook_task_list` shape (has `id`, `title`, `status`, `createdAt`, `updatedAt`); `tasks.md` checklist conventions (`- [ ]` / `- [x]`); `.metadata.json` fields (`status`, `createdAt`, `updatedAt`).

## What Changes

### New `tasks_loader` module
- `crates/cortex-api/src/tasks_loader.rs` — pure-Rust loader that reads `.rulebook/tasks/*` (active) and `.rulebook/archive/*` (archived). For each task directory it parses:
  - `id` (directory name; archived tasks strip the leading `YYYY-MM-DD-` date prefix)
  - `phase` (parsed from id prefix `phaseN<letter?>` — e.g. `phase2g`, `phase4a`)
  - `title` (first H1 of `proposal.md` or fallback to id)
  - `status` (`pending` / `in-progress` / `completed` / `archived`) — `archived` when the task lives under `.rulebook/archive/`; otherwise read from `.metadata.json`
  - `created_at` / `updated_at` (from `.metadata.json`; archived dir's date prefix is the fallback)
  - `archived_at` (parsed from archive dir prefix when applicable)
  - `progress` — `{ done: usize, total: usize }` parsed from `tasks.md` (count `- [x]` vs `- [ ]`; ignore non-checkbox lines)
  - `summary` (first non-heading paragraph of `proposal.md`, trimmed to ~280 chars)
- Loader is read-only; never writes. Cached behind a `RwLock` with file-mtime invalidation so the dashboard doesn't re-walk the filesystem on every request.

### New routes under `/v1/dashboard/tasks`
- `GET /v1/dashboard/tasks` — list all tasks. Query params:
  - `status=pending|in-progress|completed|archived` (optional, repeatable)
  - `phase=phase2|phase4a` (optional, repeatable; matches prefix exactly)
  - `include_archived=bool` (default `true` — the whole point of the view)
  - `limit` / `offset` for pagination (default 200 / 0)
  - Response shape:
    ```json
    {
      "tasks": [TaskRow, ...],
      "total": 50,
      "by_phase": [{ "phase": "phase2", "total": 22, "done": 19, "in_progress": 2, "pending": 1 }, ...],
      "by_status": { "completed": 36, "in-progress": 5, "pending": 9, "archived": 39 }
    }
    ```
- `GET /v1/dashboard/tasks/{id}` — task detail. Returns the row plus:
  - `proposal_md` (full proposal text)
  - `checklist` — array of `{ section: "1. Backend — overview series", items: [{ text, done }] }` parsed from `tasks.md`
  - `specs` — list of `{ path, name }` for files under `specs/` (no body inlined; the GUI can request a follow-up if needed)
- `GET /v1/dashboard/tasks/summary` — aggregate metrics for the sidebar pill: `{ total, completed, in_progress, pending, archived, completion_pct }`.

### Wiring
- `dashboard.rs::router()` mounts the three new routes alongside the existing ones.
- Loader instance lives on `DashboardState` (or a sibling struct) so handlers share the cache.
- Workspace root resolution reuses whatever `cortex-api` already uses to resolve `.rulebook/` (likely `cortex.toml`-driven or env var); no new config keys.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (add the new endpoints + response shapes).
- Affected code: new `crates/cortex-api/src/tasks_loader.rs`; `crates/cortex-api/src/dashboard.rs` (3 routes + state plumbing); `crates/cortex-api/src/lib.rs` (module registration).
- Breaking change: NO — purely additive endpoints.
- Depends on: nothing — `.rulebook/` filesystem layout is stable.
- User benefit: dashboard becomes the single pane that shows what the project has shipped (39 archived tasks) and what is still in flight (11 active), without leaving the Electron GUI.
