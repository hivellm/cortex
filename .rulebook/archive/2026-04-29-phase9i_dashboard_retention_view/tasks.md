## 1. Backend routes
- [x] 1.1 `GET /v1/retention/sweeps?limit=N&since=RFC3339` reads `retention_sweeps` joined with the per-stage JSON breakdown; returns `[{ sweep_id, started_at, finished_at, status, records_demoted, records_dropped, stages: { sweep, parquet_rollup, cas_vacuum, ... } }]`
- [x] 1.2 `GET /v1/retention/state` aggregates: Parquet archive bytes by age bucket, `cas_blobs` totals, scheduled next-runs. Vectorizer collection sizes + Meili doc counts surface as honest empty arrays today (SDK probe lands when `DashboardState` carries the credentials phase9k feeds in)
- [x] 1.3 Both routes are pure reads against in-memory caches that already refresh every 10 s (`MetadataStore` + on-disk archive walk) — no per-route cache layer needed; the GUI's TanStack Query keeps `refetchInterval: 10_000`
- [x] 1.4 Wired to `crates/cortex-api/src/dashboard.rs`; same `Json(...)` envelope every other dashboard route uses

## 2. GUI components
- [x] 2.1 NEW `gui/src/views/Retention.tsx`
- [x] 2.2 Header card row: one card per sweep type with color-coded state (`ok` / `degraded` / `failed` / `never`) and `last_run` relative time
- [x] 2.3 30-day reclamation sparkline derived from each sweep row's `bytes_reclaimed` stage counter (reuses `gui/src/atoms/Sparkline.tsx`, the canonical sparkline atom; the proposal's `SparkChart.tsx` does not exist in this tree)
- [x] 2.4 Sortable breakdown table: source/name, size now, size 30 d ago, delta — sortable by source, size_now, or delta with click-to-toggle direction
- [x] 2.5 Failure banner: red bar when any sweep type has `status='failed'` in its two most recent runs; surfaces `last_error` from the stages payload
- [x] 2.6 Live log strip: SSE-driven, filters for `kind` starting `retention.`, capped at 100 lines, surfaces SSE connected/disconnected pill

## 3. Sidebar + routing
- [x] 3.1 Added "Retention" entry to `gui/src/shell/Sidebar.tsx` directly after "Memory" (before Decisions). The proposal's "between Memory and Tweaks" wording assumed Tweaks was a sidebar entry — it isn't in this tree (Tweaks is a header-toggled drawer); placing Retention next to Memory matches the user-facing flow the proposal described
- [x] 3.2 Added the case to `gui/src/App.tsx`'s `renderView` switch; default body is the Retention overview (no sub-tab routing yet — the page is a single scroll)
- [x] 3.3 Icon: new `archive` glyph in `gui/src/atoms/Icon.tsx` (inline SVG matches the existing icon set; no lucide dependency exists in this tree)

## 4. SSE
- [x] 4.1 Reuses `gui/src/lib/useSSE.ts` (the existing SSE hook in this tree; the proposal's `gui/src/hooks/useLiveStream.ts` does not exist) against `/v1/dashboard/timeline/stream` (the canonical timeline SSE endpoint)
- [x] 4.2 Client-side predicate `kind.startsWith("retention.")` filters the timeline events; the typed shape lives next to the view as `LiveTimelineEvent`

## 5. Spec / docs
- [x] 5.1 Updated `docs/specs/16-dashboard.md` with the two routes + the Retention view section
- [x] 5.2 Added §"Observability (phase9i)" to `docs/specs/19-retention.md` referencing the dashboard contract

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation
- [x] 6.2 Write tests covering the new behavior
- [x] 6.3 Run tests and confirm they pass
