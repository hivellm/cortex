## 1. Backend — overview series
- [x] 1.1 Extend `OverviewBody` with optional `series: SeriesBlock` field (default `None` so existing tests round-trip)
- [x] 1.2 `events_per_min`: bucket envelope timestamps by minute over the last 20 minutes; emit count per bucket
- [x] 1.3 `pre_thinking_p95_ms`: P95 of turn-envelope latency stamps (already preserved on the lane); one P95 per minute bucket; `null` when no turns landed in the bucket
- [x] 1.4 `violations_7d_daily`: count of `kind=law_violation` envelopes per day over the last 7 days (length 7)
- [x] 1.5 `classifier_cost_usd_today`: 0.0 placeholder array of length 24 with a top-level note `"classifier_cost_unavailable_until_spec05": true` — no fabricated values
- [x] 1.6 Helpers live in `crates/cortex-api/src/dashboard_series.rs` so the route handler stays compact (flat sibling module — adapts to the existing crate convention; intent of "compact route handler" preserved)

## 2. Backend — tools heatmap matrix
- [x] 2.1 Extend `tools/stats` response with optional `heatmap: HeatmapBlock` field
- [x] 2.2 Bucket `tool_call:*` envelopes by `(weekday(ts_utc), hour(ts_utc))` over the last 7 days
- [x] 2.3 Response shape: `{ tz: "UTC", days: ["Mon", ..., "Sun"], hours: 0..23, cells: u32[7][24] }`
- [x] 2.4 When the lane has no tool calls, return zeros (not null) — keeps the renderer simple

## 3. Backend — trust endpoint
- [x] 3.1 Add `/v1/dashboard/trust` route returning `{ models, repos, scores }`
- [x] 3.2 v1 returns empty arrays/objects (spec 14 lands the real implementation)
- [x] 3.3 Note in the response: `"source": "stub_until_spec14"` so the renderer can show the right empty state copy

## 4. Frontend — consume the new fields
- [x] 4.1 Update `gui/src/lib/api.ts` types: `Overview.series?: { ... }`, `ToolsStats.heatmap?: { ... }`, new `TrustMatrix` type
- [x] 4.2 Timeline stats grid: when `series.events_per_min` exists, swap the rolling-buffer Sparkline for the backend series and rename "Events captured" → "Events / min"
- [x] 4.3 Same pattern for "P95" tile (uses `pre_thinking_p95_ms`) and "Violations 7d" tile
- [x] 4.4 Tools view: when `heatmap` exists, render a 7×24 grid using the design's oklch intensity formula
- [x] 4.5 Laws view: render a Trust matrix card using `/v1/dashboard/trust`; empty state when `source === "stub_until_spec14"`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — extend `docs/specs/16-dashboard.md` with the new shapes; extend `gui/README.md` with what each tile / heatmap / matrix surfaces and which series remain stubs
- [x] 5.2 Write tests covering the new behavior — Rust unit tests for the bucketing helpers (timestamps spanning 25 minutes → 20-element series newest-last); integration test for `/overview` with series; integration test for tools heatmap with a known set of timestamps; integration test for `/trust` empty stub; RTL: stats grid renders backend series when present, falls back to rolling buffer when absent
- [x] 5.3 Run tests and confirm they pass — `cargo test -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `pnpm test`, `pnpm exec tsc --noEmit -p tsconfig.json`
