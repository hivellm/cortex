## 1. API client
- [x] 1.1 Add `TaskRow`, `TaskChecklistItem`, `TaskChecklistSection`, `TaskDetail`, `TaskListResponse`, `TaskSummary` types to `gui/src/lib/api.ts` mirroring the backend shapes from `phase5a`
- [x] 1.2 Add `api.tasks(params: { status?: string[]; phase?: string[]; repo?: string[]; include_archived?: boolean; limit?: number; offset?: number; sort?: string; order?: "asc" | "desc" })` returning `TaskListResponse`
- [x] 1.3 Add `api.task(id: string)` returning `TaskDetail`; pass `id` through `encodeURIComponent`
- [x] 1.4 Add `api.tasksSummary()` returning `TaskSummary`
- [x] 1.5 Re-export the new types from the same barrel `lib/api.ts` already uses for `RepoCount` / `SessionRow`

## 2. Sidebar wiring
- [x] 2.1 Add `"tasks"` to the `ViewId` union in `gui/src/shell/Sidebar.tsx`
- [x] 2.2 Add `"tasks"` to the `CountKey` union; insert a `NAV` entry between `analysis` and `tools` (icon: `decision`)
- [x] 2.3 Add a `tasksSummaryQ = useQuery({ queryKey: ["tasks-summary"], queryFn: () => api.tasksSummary(), refetchInterval: 30_000 })` and set `counts.tasks = tasksSummaryQ.data?.total`

## 3. App routing
- [x] 3.1 In `gui/src/App.tsx`, add a `case "tasks":` branch that renders `<TasksView />` from the new view module
- [x] 3.2 Add the matching import (sorted with the other view imports)

## 4. View — stats + filters
- [x] 4.1 Create `gui/src/views/Tasks.tsx` with a `TasksView()` component
- [x] 4.2 Use `api.tasksSummary()` to render 4 stat tiles (`Total`, `Completed`, `In progress`, `Pending`); completion-pct surfaced as the `Completed` sub-label
- [x] 4.3 Filter bar: project chips (multi-project — phase5b extension), status chips (`pending` / `in-progress` / `completed` / `archived`), phase chips populated from `list.by_phase` keys, "show archived" toggle, client-side text search input
- [x] 4.4 Persist filter selections in `localStorage` under key `"cortex.tasks.filters"`

## 5. View — list + detail
- [x] 5.1 Tasks list query: `useQuery({ queryKey: ["tasks", "all"], queryFn: () => api.tasks({...}), refetchInterval: 30_000 })`
- [x] 5.2 Group rows by repo first, then by phase within each repo; collapsible group headers showing `phase`, total, and an aggregate `done/total` progress bar
- [x] 5.3 Each row: id (mono, click target), title, status pill colored per status, `done/total` progress bar, relative `updated_at`
- [x] 5.4 Selecting a row sets a `selectedId` state and fires `useQuery({ queryKey: ["task", selectedId], queryFn: () => api.task(selectedId!), enabled: !!selectedId, staleTime: 60_000 })`
- [x] 5.5 Detail panel renders `proposal_md` as preformatted text (no react-markdown dep added — preserves whitespace and is rules-compliant against the no-shortcuts hook)
- [x] 5.6 Detail panel renders the sectioned checklist with checkbox-style indicators and per-section progress (read-only)

## 6. Phase5b multi-project extension (operator request)
- [x] 6.1 Backend: add `repo: Option<String>` to `TaskRow`; new `MultiTaskLoader` aggregator; new `CORTEX_RULEBOOK_ROOTS` env var (semi/comma-separated `.rulebook/` paths); per-loader `with_repo` builder; repo filter through `ListQuery`
- [x] 6.2 Docker compose: bind-mount the workspace parent at `/workspaces` (read-only) so every sibling project's `.rulebook/` is reachable; default config registers cortex/hivegpu/nexus/rulebook/synap/tml/tmldocs/vectorizer
- [x] 6.3 Frontend: project chip row in the filter bar, group rendering by repo → phase, repo column shown on each row when present
- [x] 6.4 Defensive UTF-8 char-boundary truncation in `summarize_proposal` so multi-byte glyphs in non-Cortex projects no longer panic the loader

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — task spec + multi-project notes shipped via this tasks.md update; backend module docstrings extended on `MultiTaskLoader` and main.rs `CORTEX_RULEBOOK_ROOTS`
- [x] 7.2 Write tests covering the new behavior — workspace `cargo test` green (including the 30 dashboard + 7 tasks-loader regression tests after the multi-loader refactor); GUI `tsc --noEmit` clean; visual verification via Playwright (845 tasks across 8 projects, project filter narrows to per-project view, detail panel renders proposal_md + checklist)
- [x] 7.3 Run tests and confirm they pass — `cargo test -p cortex-api` (273 lib + 6 dashboard_tasks + 30 http + others) and `gui/tsc --noEmit` both clean
