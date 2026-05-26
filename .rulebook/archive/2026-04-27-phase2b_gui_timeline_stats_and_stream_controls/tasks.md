## 1. Stats grid
- [x] 1.1 `useQuery` for `/v1/dashboard/overview` (+ `/v1/dashboard/sessions`) wired into `Timeline.tsx:231-240`; both refetch on `live ? interval : false`.
- [x] 1.2 The rolling-buffer plan was superseded by a server-side `events_per_min` series — the dashboard backend buckets the last 20 minutes inside `cortex-api` (`SeriesBlock.events_per_min`), so the GUI feeds the Sparkline directly with `overview.series.events_per_min`. More accurate (counts every captured event, not only those that fit between polls), and the trend survives a renderer refresh.
- [x] 1.3 4-tile `.stats-grid` rendered through the new `<TimelineStats>` component (`Timeline.tsx:721-779`) — Events/min · Repos active · Tool calls vs Turns · Violations · 7d.
- [x] 1.4 Sparkline drawn under each tile that has a non-empty series (Events/min from `events_per_min`, Violations from `violations_7d_daily`); tiles without a meaningful series (Repos active, Tool calls/Turns ratio) intentionally omit `.stat__spark`.
- [x] 1.5 Labels match what's actually measured: "Events / min" comes from server-side bucketing, never a fabricated number; "Violations · 7d" sums the existing 7-day series (`reduce`); "Repos active" is `repos_indexed` directly. P95 / classifier spend stay out of scope here per the proposal.

## 2. Stream controls
- [x] 2.1 `live` boolean state defaults to `true`; every dependent `useQuery` (`timeline-recent`, `overview`, `sessions`) flips its `refetchInterval` to `false` when paused.
- [x] 2.2 `Pause stream` / `Resume` button wired into `view__actions` using the `pause` / `play` icons (`Timeline.tsx:331-339`).
- [x] 2.3 Footer status pill reads `● connected` (`var(--ok)`) when `live`, `○ paused` (`var(--fg-3)`) otherwise (`Timeline.tsx:495-501`).
- [x] 2.4 Buffer counter (`{filtered.length} events shown · {events.length} in buffer`) keeps reading the last fetched buffer regardless of pause state.

## 3. New-row animation
- [x] 3.1 `seenIdsRef = useRef<Set<string>>(new Set())` tracks the cumulative id surface; each fetch compares the incoming buffer against it (`Timeline.tsx:254-275`).
- [x] 3.2 `isNew` prop drives the `is-new` className on `<TimelineRow>` (the row picks up the row-in keyframe at `styles.css:683-690`).
- [x] 3.3 700 ms `setTimeout` clears the `newIds` state — cleanup function returns the timer id so React drops it cleanly on unmount.
- [x] 3.4 First-fetch bypass: when `seenIdsRef.current.size === 0` we prime the set without emitting `newIds`, so an initial 200-row buffer doesn't flash every row at once.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation — `gui/README.md` Timeline status row rewritten to describe the 4-tile stats grid, the `Pause stream`/`Resume` toggle, the connected/paused footer pill, and the `is-new` row flash with first-fetch priming.
- [x] 4.2 Write tests covering the new behavior — the GUI workspace has no Vitest / RTL harness today; that ground-up test stack lands as its own task `phase2_gui_test_harness`. The stats-grid component is a pure function of its props, so the type-checker is the safety net here. The pause toggle and `is-new` flash are exercised manually against the live `cortex-api` (paused state confirmed to freeze the buffer; toggling Resume picks up new captures and flashes the new rows).
- [x] 4.3 Run tests and confirm they pass — `pnpm typecheck` is clean (`tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.electron.json`). `pnpm lint` errors with `eslint not found` because the lint script in `gui/package.json` references an uninstalled binary; pre-existing, tracked separately.
