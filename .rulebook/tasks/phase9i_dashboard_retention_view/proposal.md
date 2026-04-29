# Proposal: phase9i_dashboard_retention_view

## Why

Tasks 9a–9h all write to the same `retention_sweeps` table and emit
`cortex.events.enriched` events with `kind` prefixed `retention.*`.
Without an operator-facing surface, the only way to know whether the
nightly cleanup actually ran is to grep SQLite. That makes regressions
invisible — a sweeper that silently fails for two weeks looks identical
to a sweeper that completes nightly with zero deletions on a fresh
install.

The dashboard already has a "Memory" view (`gui/src/views/Memory.tsx`)
plus a "Tweaks" panel; this task adds the "Retention" tab so the
operator can see what each sweep moved, when, and how much it
reclaimed, with a banner if the latest sweep failed.

## What Changes

1. NEW backend route on `cortex-api`: `GET /v1/retention/sweeps?limit=N`
   returns the recent rows from `retention_sweeps` joined with
   per-stage breakdowns (`tier_transitions_json`).
2. NEW route `GET /v1/retention/state` returns a compact "current
   state" envelope: per-collection size, archive bytes by partition
   age bucket (≤30 d / 30–365 d / >365 d), Meili index doc counts,
   `cas_blobs` row count + bytes, the next-scheduled run for each
   sweep (read from the 9k cron table when available; falls back to
   "never").
3. NEW GUI tab `gui/src/views/Retention.tsx`:
   - Header card: status of the most recent run per sweep type
     (green / amber / red), last-run timestamp, bytes reclaimed.
   - Time series: bytes reclaimed per sweep per day for the last 30 d.
   - Breakdown table: per-collection / per-index sizes today vs 30 d ago.
   - Failure banner: red banner when any sweep type has failed twice
     in a row, with the latest `last_error`.
4. Adds the tab to the existing sidebar in `gui/src/views/Sidebar.tsx`
   between "Memory" and "Tweaks".
5. SSE handle: subscribes to the same `cortex.live.<repo>` topic the
   timeline uses, filters for `kind` starting `retention.`, prepends to
   a live log so the operator can watch a sweep finish in real time.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (new route + view),
  `docs/specs/19-retention.md` (mention dashboard observability).
- Affected code: NEW routes in `crates/cortex-api/src/dashboard.rs`,
  NEW `gui/src/views/Retention.tsx`, sidebar wire-up.
- Breaking change: NO. Pure additive surface.
- User benefit: closes the observability loop on Phase 9; failure
  modes are visible without grep.
