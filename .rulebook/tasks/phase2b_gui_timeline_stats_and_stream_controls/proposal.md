# Proposal: phase2b_gui_timeline_stats_and_stream_controls

## Why

The design's Timeline view (`gui/assets/view-timeline.jsx`) opens with a 4-tile stats grid (Events/min · Pre-thinking P95 · Active violations 7d · Classifier spend) and a `Pause stream` / `Resume` toggle. The current implementation has none of that — it shows a bare filter bar over a row list with a hard-coded 5-second poll. The footer pill says `poll: 5s` instead of the design's connected/paused indicator. New rows arrive without any visual cue; the design briefly highlights them via a `is-new` flash animation that is already in `styles.css` (lines 683–690) but never triggered.

The honest version of the stats grid (without faking time-series numbers) uses what `/v1/dashboard/overview` already returns: events_total, repos_indexed, kind breakdown. Series for Sparklines we derive locally from the polled buffer (last N intervals).

Source: `gui/assets/view-timeline.jsx` (TimelineFilters, TimelineView), `gui/src/styles.css` (rowIn keyframes already shipped).

## What Changes

- Timeline view gets a 4-tile stats grid above the filter bar, populated from data we actually have:
  - **Events captured** (overview.events_total, sparkline from rolling buffer of overview polls)
  - **Repos active** (overview.repos_indexed)
  - **Tool calls / Turns** (kind_breakdown counts)
  - **Sessions** (sessions endpoint length)
- Each tile uses the `Sparkline` atom for a 28-px trend line where a series exists; tiles without a series omit the spark.
- `Pause stream` / `Resume` button: when paused, `useQuery` `refetchInterval` is set to `false`; status pill in footer reads `● connected` / `○ paused`.
- New-row detection: track previously-seen `id` set; rows whose id is new since last refetch render with `is-new` class for ~700 ms (re-uses the existing keyframes).
- Footer line replaces `poll: 5s` with the connected/paused state pill.

Stays inside frontend — no backend changes. Honest metric labels, no fake numbers (Events/min, P95, classifier spend stay in phase2g).

## Impact

- Affected specs: none.
- Affected code: `gui/src/views/Timeline.tsx`, `gui/src/atoms/Sparkline.tsx` (no change expected, just new consumer).
- Breaking change: NO.
- User benefit: Timeline stops looking half-finished, user can pause the stream when reading a long row, and visual feedback when new events land matches the design.
