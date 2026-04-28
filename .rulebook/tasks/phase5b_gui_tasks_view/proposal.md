# Proposal: phase5b_gui_tasks_view

## Why

`phase5a_dashboard_tasks_backend` exposes `/v1/dashboard/tasks*`, but the Electron GUI (`gui/src/`) does not consume it yet. Without a Tasks view, the user still cannot see project progress (39 archived + 11 active tasks at the time of writing) inside the dashboard — defeating the purpose of phase5a.

The existing GUI shell already has the slot for this kind of view: `gui/src/shell/Sidebar.tsx` defines a typed `ViewId` enum and a `NAV` array of items each with an icon + count source; the `App.tsx` switches on `view` to render the matching `gui/src/views/*.tsx`. Adding a Tasks view follows the same pattern as `Decisions.tsx` / `Analysis.tsx` (list + filter + drill-down panel).

Source: `gui/src/shell/Sidebar.tsx` (NAV layout); `gui/src/views/Decisions.tsx` and `gui/src/views/Analysis.tsx` (closest pattern — list + detail); `gui/src/lib/api.ts` (where the new `/v1/dashboard/tasks*` calls land); the backend contracts shipped by `phase5a_dashboard_tasks_backend`.

## What Changes

### `gui/src/lib/api.ts` — types + fetchers
- New types: `TaskRow`, `TaskListResponse`, `TaskDetail`, `TaskSummary`, `TaskChecklistSection`, `TaskChecklistItem`.
- New methods: `api.tasks(params)`, `api.task(id)`, `api.tasksSummary()`. Stay on `axios`/`fetch` consistent with the rest of `api.ts`.

### Sidebar entry
- Extend `ViewId` with `"tasks"`.
- Extend `CountKey` with `"tasks"` and add the entry to `NAV` (icon: a new `clipboard` or reuse `analysis` if no fitting icon exists yet — choose at implementation).
- Wire `counts.tasks = tasksSummaryQ.data?.total` so the sidebar pill shows total tasks (active + archived).

### `gui/src/views/Tasks.tsx` — new view
- Top stats row: 4 small tiles using the existing stats-tile atom — `Total`, `Completed (incl. archived)`, `In progress`, `Pending`. Right-aligned a small "completion %" badge derived from `summary.completion_pct`.
- Filter bar (sticky): status multi-select chips (`pending` / `in-progress` / `completed` / `archived`), phase multi-select chips (populated from `by_phase` keys returned by the list endpoint), a "show archived" toggle (default ON — that is the entire point), text search filtering on `id` + `title` (client-side over the loaded page).
- Main list: tasks grouped by phase (collapsible group headers showing `done/total` and a thin progress bar). Each row shows id (mono), title, status pill, progress bar (`done/total` from the checklist), and `updated_at` relative time (reuse `lib/format.ts` if present, otherwise add a tiny helper).
- Sort: phase asc by default; clicking a column header switches sort to `updated_at desc` / `created_at desc`.
- Click a row → opens a right-side drawer (or in-page detail panel matching `Decisions.tsx`'s style) showing `proposal_md` rendered as Markdown (reuse whatever Markdown renderer the GUI already has, e.g. for analyses) + the sectioned checklist with progress.

### TanStack Query plumbing
- `tasks` list query: `queryKey: ["tasks", filters]`, `refetchInterval: 30_000`.
- `task` detail query: `queryKey: ["task", id]`, fetch on demand when a row is opened, `staleTime: 60_000`.
- `tasks-summary` query: `queryKey: ["tasks-summary"]`, `refetchInterval: 30_000` (drives the sidebar count + the top tiles).

### Filter persistence
- Reuse `gui/src/lib/filters.ts` if it has a generic per-view filter slice; otherwise add a self-contained `useTasksFilters` hook backed by `localStorage` so the user's status/phase selection persists across reloads (matches existing UX of the repo / session filters).

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (Tasks view section); `gui/README.md` (new view + what it surfaces).
- Affected code: `gui/src/lib/api.ts` (types + fetchers); `gui/src/shell/Sidebar.tsx` (NAV entry + count); `gui/src/App.tsx` (route the new `view`); new `gui/src/views/Tasks.tsx`; possibly small additions to `gui/src/atoms/` if a new icon or stats tile variant is needed.
- Breaking change: NO — purely additive UI surface.
- Depends on: `phase5a_dashboard_tasks_backend` (must land first; the GUI cannot render without those endpoints).
- User benefit: a single Tasks pane in the Electron app shows everything that has shipped (39 archived) and what is still moving (11 active), grouped by phase, filterable by status, with drill-down into the original proposal + live checklist progress.
