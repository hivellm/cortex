# Proposal: phase29_missing-index-empty-200-all-handlers

Source: phase28 manual E2E verification (run 1, 2026-06-20) — finding F1.

## Why

Issue #4 Bug 1 ("a missing Meili index is a recoverable empty state, not
a 502") was fixed in only 2 of ~12 Meili-backed read handlers
(`tool_calls`, `events_by_kind`). Manual verification found the same
502-on-missing-index bug still live on the rest — e.g.
`GET /v1/consolidations/recent` returns
`502 Index `cortex_consolidations` not found`. Any consumer of
consolidations / decisions / topics / law-violations hard-fails on a
daemon that hasn't created those global indices yet. The fix already
exists (`search::is_meili_index_missing`); it just needs to be applied
uniformly so the whole read surface degrades to `200` empty instead of
`502`.

## What Changes

Apply the existing `is_meili_index_missing(status, body)` guard to every
remaining Meili-backed handler's `!status.is_success()` branch, returning
that handler's empty response shape (200) instead of `BAD_GATEWAY`:

- `consolidations_recent`, `consolidations_search`, `consolidations_by_entity`,
  `consolidations_diff`, `consolidation_costs`, `consolidation_get`,
  `consolidation_lineage`
- `decision_search`, `topic_search`, `law_violations`
- audit any other handler in `crates/cortex-api/src/search/` with the same
  `BAD_GATEWAY`-on-non-success pattern (search_proxy keyword already
  returns a structured 404, leave as-is or align).

Each handler returns its own typed empty response (empty hits + zeroed
counts). `consolidation_get` (single-doc) returns 404 not-found rather
than empty, since a missing item is semantically not-found.

## Impact

- Affected specs: spec 11 (query surface error contract), spec 27
  (consolidation read endpoints).
- Affected code: ~10 handlers under `crates/cortex-api/src/search/`.
- Breaking change: NO (502 → 200 empty is a strictly friendlier
  contract; consumers tolerate empty).
- User benefit: consolidation/decision/topic/violation reads stop
  hard-failing on a fresh or partially-indexed daemon.
