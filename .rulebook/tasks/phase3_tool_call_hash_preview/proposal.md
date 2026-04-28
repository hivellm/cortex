# Proposal: phase3_tool_call_hash_preview

## Why

The GUI's live timeline already surfaces `tool_call` envelopes, but two
fields the archive lane has captured **for months** are not exposed to
the user:

1. **`content_hash`** — the sha256 fingerprint that the spec-18 plugin
   stamps on every envelope. `archive_loader.rs:291` populates it onto
   `LaneHit.content_hash`, but `TimelineEvent` (dashboard.rs:270) drops
   it on the way out. The user has no way to spot duplicate calls
   ("did Claude run the same `Bash` 4 times this turn?") or to
   correlate two tool_call rows that originated from the same on-disk
   write.
2. **Full tool-call preview** — the timeline's `detail` field is
   clipped at 280 chars (dashboard.rs:326). For most tool calls that
   loses the actual edit/diff/script. The Inspector has no fallback —
   it just renders the same clipped string back. The user opens
   the inspector hoping to see *what* the tool did, and finds the same
   single sentence as the row.

Both are operational-ergonomics gaps flagged in
`docs/analysis/cortex/05-gui-and-api.md` ("Tool-call hash + content
preview") and explicitly tagged template-only in
`docs/analysis/cortex/08-task-backlog.md` ("either flesh out or close").
This task fleshes it out.

## What Changes

### Server (`crates/cortex-api`)

- Extend `TimelineEvent` (dashboard.rs:270) with two optional fields,
  both serialized only when present so non-tool_call rows stay lean:
  - `content_hash: Option<String>` — straight pass-through from
    `LaneHit.content_hash` (`sha256:<64hex>`).
  - `preview: Option<String>` — un-clipped body when the lane has
    captured the full text. Hard-cap at 8 KiB so a 200-row response
    stays under 2 MiB; rows larger than that get a `preview_truncated:
    true` marker and the user fetches the full body via the existing
    `/v1/dashboard/timeline/{id}` route.
- The `timeline_recent` mapper sets `preview` from `h.text`
  (the lane already holds the full text — the 280-char clip lives only
  in the `detail` field). The `content_hash` field maps from
  `h.content_hash`.
- SSE stream uses the same struct, so live rows pick the new fields up
  for free.
- A doctor probe asserts `content_hash` is non-null for ≥ 99% of
  archive-sourced `tool_call` rows in the last 24 h. (Redacted lane
  hits intentionally drop the hash — see `redaction.rs:86`.)

### GUI (`gui/src/views/Timeline.tsx`)

- Extend `TimelineEvent` in `gui/src/lib/api.ts` with the two new
  optional fields.
- Inspector grows a **Content** section above the existing **Detail**
  block, visible only when `kind === "tool_call"` and `preview` is
  non-empty. Renders `preview` with mono font, syntax-aware
  highlighting (just `<pre><code>` for now — fancy highlighting is
  out of scope), and a "copy" button.
- Inspector's **Envelope** dl gains a `content_hash` row showing the
  short form (`sha256:abc1234…`), with a copy-to-clipboard button.
  Clicking the hash filters the timeline to all rows with the same
  `content_hash` — that's the dedupe / replay-detection workflow the
  user actually needs.

## Impact

- **Affected specs**: spec-16 (dashboard surface) — adds two optional
  fields to the timeline shape; no breaking change because both are
  `Option`.
- **Affected code**:
  - `crates/cortex-api/src/dashboard.rs` (struct + mapper)
  - `crates/cortex-api/src/dashboard.rs` doctor probe
  - `gui/src/lib/api.ts` (type)
  - `gui/src/views/Timeline.tsx` (Inspector render + dedupe filter)
- **Breaking change**: NO. New fields are optional; existing clients
  that don't read them are unaffected.
- **User benefit**: closes the "what did this tool actually do" gap on
  the dashboard, and unlocks a one-click "show me every call with
  this fingerprint" workflow that doesn't exist anywhere today.

## Source

- `docs/analysis/cortex/05-gui-and-api.md` — "GUI gaps tracked as
  tasks" table row "Tool-call hash + content preview".
- `docs/analysis/cortex/08-task-backlog.md` — P5 hygiene action item
  ("either flesh out or close").
- `docs/analysis/cortex/10-improvement-roadmap.md` — Sprint 5 reach
  & ergonomics line "Complete or close `phase3_tool_call_hash_preview`".
