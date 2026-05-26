# Proposal: phase2g_dashboard_enriched_metrics

## Why

Three of the design's signature numbers — Events/min, Pre-thinking P95, Classifier spend — and the day×hour Tool-call density heatmap (`view-late.jsx` lines 122–144) need data the backend does not derive yet. They are not fakeable in the renderer because they require time-bucketed aggregations over the captured envelopes (`/v1/dashboard/overview` only returns instantaneous counts; `/v1/dashboard/tools/stats` only returns totals). The Trust matrix on the Laws view (`views-mid.jsx` lines 207–234, model × repo heat shading) needs `/v1/dashboard/trust` from `phase2_dashboard` §1.10 — currently absent.

The honest tile labels shipped in phase2b cover what we have today; this task widens the backend to make the design's labels truthful.

Source: `phase2_dashboard/tasks.md` items 1.2 (sparkline series) + 1.8 (heatmap) + 1.10 (trust); `gui/assets/view-timeline.jsx` lines 144–189 (sparks); `gui/assets/views-late.jsx` lines 122–144 (heatmap); `gui/assets/views-mid.jsx` lines 207–234 (trust matrix).

## What Changes

### `/v1/dashboard/overview` enrichment
- New optional `series` block in the response:
  ```json
  {
    "events_per_min": [12, 18, 14, ...],
    "pre_thinking_p95_ms": [142, 138, ...],
    "violations_7d_daily": [2, 1, 3, ...],
    "classifier_cost_usd_today": [0.12, 0.34, ...]
  }
  ```
- Each series is an array of N samples (default 20) representing the last N intervals of width 1 minute.
- `pre_thinking_p95_ms` derives from the existing query latency stamping (already captured in turn envelopes); when no turns landed in an interval, the bucket is `null` (rendered as a gap by the Sparkline atom).
- `classifier_cost_usd_today` is `0.0` until the spec-05 classifier worker is wired (acknowledged honestly in the response — no fabricated values).

### `/v1/dashboard/tools/stats` heatmap matrix
- New `heatmap` field: `{ tz: "UTC", days: ["Mon",...,"Sun"], hours: 0..23, cells: number[7][24] }`.
- Cells count tool calls bucketed by `(weekday(ts), hour(ts))` over the last 7 days.
- Frontend renders via the design's oklch intensity formula already present in `view-late.jsx` line 137.

### `/v1/dashboard/trust` (new endpoint)
- Returns `{ models: string[], repos: string[], scores: Record<model, Record<repo, number>> }` where `score ∈ [0, 1]`.
- v1 is a stub returning `{ models: [], repos: [], scores: {} }` so the Laws view can render an empty state without conditionally hiding the section. Spec 14 will populate it with real trust scores derived from violation rates.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (overview/tools shapes); `phase2_dashboard` §1.2 + §1.8 + §1.10 close.
- Affected code: `crates/cortex-api/src/dashboard.rs` (extend overview, tools/stats, add trust handler), `crates/cortex-api/src/lanes.rs` (expose timestamp-bucketing helpers if missing).
- Breaking change: NO — new fields are additive; old clients ignore them.
- Depends on: nothing.
- User benefit: Timeline stats grid surfaces real eps/P95 instead of "events captured", Tools view gets the heatmap, Laws view gets the trust matrix slot ready for spec 14.
