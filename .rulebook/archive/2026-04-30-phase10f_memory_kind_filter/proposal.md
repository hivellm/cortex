# Proposal: phase10f_memory_kind_filter

## Why

The audit confirmed `/v1/dashboard/memory` always returns the
**most-recent** events regardless of the `kind` you ask for.
Passing `?kind=decision` or `?kind=analysis` either crashes the
script (the response isn't valid JSON) or ignores the parameter
silently. The aggregate `?facets=` parameter exists in the URL
contract but never reaches the lane, so the GUI's Memory tab
shows nothing but `tool_call` and `turn` rows even though the
overview reports 287 memories, 26 decisions, 33 analyses, 121
violations sitting in the same lane.

For the agent this is the difference between "10 most recent
shell commands" (currently surfaced) and "every analysis from
2026-04-28 plus the 5 decisions cited last week" (what the
endpoint should return when filtered by kind).

## What Changes

1. `/v1/dashboard/memory` accepts `?kind=<canonical>` and filters
   on the lane row's `kind` field.
2. Multi-kind support: `?kind=decision&kind=analysis` (clap-style
   repeated query param) ORs the kinds.
3. `?facets=` becomes the canonical alias for `?kind=` so
   existing callers don't break.
4. Server returns a structured 400 when an unknown kind is
   requested (`{"error":"unknown_kind","received":"foo"}`).
5. The GUI's Memory view gains a kind facet bar above the search
   input that toggles each kind chip.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` §"Memory browser".
- Affected code: `crates/cortex-api/src/dashboard.rs` (`memory`
  handler), `gui/src/views/Memory.tsx`,
  `gui/src/lib/api.ts`.
- Breaking change: NO. Adds an optional query parameter.
- User benefit: the dashboard's Memory tab finally surfaces
  analyses + decisions + violations + memories instead of just
  recent tool calls.
