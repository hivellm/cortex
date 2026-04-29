## 1. Backend routes
- [ ] 1.1 `GET /v1/retention/sweeps?limit=N&since=RFC3339` reads `retention_sweeps` joined with the per-stage JSON breakdown; returns `[{ sweep_id, started_at, finished_at, status, records_demoted, records_dropped, stages: { sweep, parquet_rollup, cas_vacuum, ... } }]`
- [ ] 1.2 `GET /v1/retention/state` aggregates: per-Vectorizer-collection size (via SDK), Parquet archive bytes by age bucket, Meili index doc counts, `cas_blobs` totals, scheduled next-runs (joins 9k's `cron_jobs` if present, else "never")
- [ ] 1.3 Both routes cached for 10 s (matches dashboard cadence)
- [ ] 1.4 Wire to `crates/cortex-api/src/dashboard.rs`; reuse the existing JSON envelope shape

## 2. GUI components
- [ ] 2.1 NEW `gui/src/views/Retention.tsx`
- [ ] 2.2 Header card row: one card per sweep type with color-coded state (`ok` / `degraded` / `failed`) and `last_run` relative time
- [ ] 2.3 Time series chart (reuse `gui/src/atoms/SparkChart.tsx`) of bytes reclaimed per sweep per day, last 30 d
- [ ] 2.4 Breakdown table: collection / index name, size now, size 30 d ago, delta, MUST be sortable
- [ ] 2.5 Failure banner: red bar when any sweep type has `status='failed'` in its two most recent runs; shows `last_error`
- [ ] 2.6 Live log strip: SSE-driven, filters for `kind` starting `retention.`, capped at 100 lines

## 3. Sidebar + routing
- [ ] 3.1 Add "Retention" entry in `gui/src/views/Sidebar.tsx` between "Memory" and "Tweaks"
- [ ] 3.2 Add the route to the GUI router; default sub-tab is "Overview"
- [ ] 3.3 Icon: a lucide `archive` glyph

## 4. SSE
- [ ] 4.1 Reuse `gui/src/hooks/useLiveStream.ts` to subscribe to `cortex.live.<repo>`
- [ ] 4.2 Filter predicate `kind.startsWith("retention.")`; coerce into a typed `RetentionEvent`

## 5. Spec / docs
- [ ] 5.1 Update `docs/specs/16-dashboard.md` with the routes + view
- [ ] 5.2 Reference from `docs/specs/19-retention.md` §Observability

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
