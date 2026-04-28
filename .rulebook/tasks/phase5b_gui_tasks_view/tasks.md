## 1. API client
- [ ] 1.1 Add `TaskRow`, `TaskChecklistItem`, `TaskChecklistSection`, `TaskDetail`, `TaskListResponse`, `TaskSummary` types to `gui/src/lib/api.ts` mirroring the backend shapes from `phase5a`
- [ ] 1.2 Add `api.tasks(params: { status?: string[]; phase?: string[]; include_archived?: boolean; limit?: number; offset?: number; sort?: string; order?: "asc" | "desc" })` returning `TaskListResponse`
- [ ] 1.3 Add `api.task(id: string)` returning `TaskDetail`; pass `id` through `encodeURIComponent`
- [ ] 1.4 Add `api.tasksSummary()` returning `TaskSummary`
- [ ] 1.5 Re-export the new types from the same barrel `lib/api.ts` already uses for `RepoCount` / `SessionRow`

## 2. Sidebar wiring
- [ ] 2.1 Add `"tasks"` to the `ViewId` union in `gui/src/shell/Sidebar.tsx`
- [ ] 2.2 Add `"tasks"` to the `CountKey` union; insert a `NAV` entry between `analysis` and `tools` (label: `"Tasks"`, icon: pick from existing `IconName` — prefer `analysis` or the closest fit; introduce a new icon only if none works)
- [ ] 2.3 Add a `tasksSummaryQ = useQuery({ queryKey: ["tasks-summary"], queryFn: () => api.tasksSummary(), refetchInterval: 30_000 })` and set `counts.tasks = tasksSummaryQ.data?.total`

## 3. App routing
- [ ] 3.1 In `gui/src/App.tsx`, add a `case "tasks":` branch that renders `<Tasks />` from the new view module
- [ ] 3.2 Add the matching import (sorted with the other view imports)

## 4. View — stats + filters
- [ ] 4.1 Create `gui/src/views/Tasks.tsx` with a default export `Tasks()` component
- [ ] 4.2 Use `api.tasksSummary()` to render 4 stat tiles (`Total`, `Completed`, `In progress`, `Pending`) reusing whichever stats-tile atom Timeline / Decisions already use; right-side completion-percent badge from `summary.completion_pct`
- [ ] 4.3 Filter bar: status chips (`pending` / `in-progress` / `completed` / `archived`), phase chips populated from `list.by_phase` keys, "show archived" toggle (default ON), client-side text search input filtering on `id` + `title`
- [ ] 4.4 Persist filter selections in `localStorage` under key `"cortex.tasks.filters"` (use the existing `lib/filters.ts` slice if it generalizes; otherwise add a small `useTasksFilters` hook in the same view file)

## 5. View — list + detail
- [ ] 5.1 Tasks list query: `useQuery({ queryKey: ["tasks", filters], queryFn: () => api.tasks(filters), refetchInterval: 30_000 })`
- [ ] 5.2 Group rows by `phase`; render collapsible group headers showing `phase`, total, and an aggregate `done/total` progress bar from row checklist counts
- [ ] 5.3 Each row: id (mono, click target), title, status pill colored per status, `done/total` progress bar, relative `updated_at`
- [ ] 5.4 Selecting a row sets a `selectedId` state and fires `useQuery({ queryKey: ["task", selectedId], queryFn: () => api.task(selectedId!), enabled: !!selectedId, staleTime: 60_000 })`
- [ ] 5.5 Detail panel renders `proposal_md` as Markdown (reuse the renderer already used by `Analysis.tsx`/`Decisions.tsx`; if none exists, add `react-markdown` to `gui/package.json`)
- [ ] 5.6 Detail panel renders the sectioned checklist with checkbox-style indicators and per-section progress (read-only — clicking does not toggle; the source of truth stays the file)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation — extend `docs/specs/16-dashboard.md` (Tasks view section: stats tiles, filters, list grouping, detail panel) and `gui/README.md` (what the Tasks pane surfaces, including that archived rows are read-only history)
- [ ] 6.2 Write tests covering the new behavior — React Testing Library tests in `gui/src/views/__tests__/Tasks.test.tsx`: stats tiles render from summary, status/phase filters narrow the list, "show archived" toggle hides archived rows when off, clicking a row opens the detail panel and renders `proposal_md`
- [ ] 6.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test --filter Tasks`, and full `pnpm test` clean
