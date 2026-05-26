# Proposal: phase11h_cortex_query_recall_recovery

## Why

`/v1/query` and the MCP `cortex_query` tool currently return empty or
oversize results for the most common intents the agent loop fires.
Three independent regressions stack on top of each other:

1. **Stale daemon binary** — the `cortex/api:dev` image in production
   was built at `2026-05-01T15:25Z` from `git_sha=888aeda`. Four
   subsequent commits never reached the running container:
   - `b75b96c` — real git SHA in `/healthz`
   - `048132e` — `.vue` SFCs routed to `code` family
   - `a67687b` — Meili filter uses `path_prefixes IN [..]` (replaces
     `path STARTS WITH ...` which Meili rejects with HTTP 400)
   - `dfb9425` — byte-budget clipper for `/v1/query` (32 KiB default)
     that prevents the MCP transport from dumping responses to a
     side-file.

   Observed symptoms:
   - `pre_change_context` with `scope.files` → keyword lane errors
     `invalid_search_filter` (a67687b not deployed).
   - `free_search` returns 72.5 KB JSON, exceeds MCP cap, dumped to
     disk (dfb9425 not deployed).
   - `/healthz` reports `git_dirty=true, build_ts="unknown"` (b75b96c
     not deployed).

2. **Bootstrap coverage gap** — `/v1/health/coverage` reports
   `overall_severity=warn` for both backends:
   - **vectorizer**: 4 / 144 collections present (only
     `cortex-cortex-{code,docs,governance,misc}`). 140 missing — every
     other repo × family combination never indexed.
   - **meili**: 29 / 144 indexes present, plus **7 unexpected** legacy
     indexes (`cortex-{family}` with no repo prefix) left over from
     the pre-slug naming scheme.

   Consequence: `decision_lookup`, `law_check`, `similar_problems`,
   `pre_change_context` against any non-`cortex` repo return empty,
   and even within `cortex` the per-family stores
   (`knowledge`, `learnings`) are absent.

3. **Decisions / laws lanes empty even where indexes exist** — the
   meili `cortex-cortex-decisions` index is healthy but yields zero
   hits for obvious project queries (e.g. "meilisearch vs lexum").
   `.rulebook/decisions/` only contains 2 ADRs (001, 002), neither
   covering the topics queried; `LAW-CORTEX-001` lives in
   `AGENTS.override.md` and was never ingested into the laws lane.
   This is institutional knowledge ingestion missing, not a lane bug.

## What Changes

- **§1 Redeploy**: rebuild `cortex/api:dev` from current HEAD, restart
  the container, verify `/healthz.git_sha == HEAD` and that the four
  fixes in flight (.vue routing, real SHA, path_prefixes IN, byte
  clipper) are all active end-to-end.
- **§2 Re-bootstrap**: run `cortex-bootstrap` for every indexed repo
  to fill the 140 missing vectorizer collections and 115 missing meili
  indexes. Drop the 7 unexpected legacy meili indexes
  (`cortex-{family}` with no repo prefix) after confirming nothing
  reads them.
- **§3 Ingest decisions + laws**: capture every ADR under
  `.rulebook/decisions/` into `cortex-cortex-decisions`, capture every
  LAW-* in `AGENTS.override.md` (and any `.claude/rules/*.md` that
  declares behavioral law) into the governance lane. Make this part
  of the bootstrap pipeline so future ADRs / laws auto-ingest.
- **§4 Regression tests**: 1) coverage drift IT that fails CI when
  any backend's `present_count < expected_count`; 2) MCP smoke test
  that fires one query per intent and asserts non-empty results for
  the seeded fixtures; 3) `/healthz` IT that asserts `git_sha != "unknown"`
  and `git_dirty == false` on release builds.

## Impact

- Affected specs: `spec-08` (Meili filter grammar), `spec-11`
  (orchestrator), `spec-18` (MCP transport), `spec-bootstrap`.
- Affected code: `crates/cortex-bootstrap/`, `crates/cortex-api/`
  (coverage IT + healthz IT), `crates/cortex-mcp-server/` (smoke
  test), `docker/` (image rebuild trigger).
- Breaking change: NO (every change is a fix or a new ingestion path).
- User benefit: every Cortex query intent the agent loop fires
  (`pre_change_context`, `decision_lookup`, `law_check`,
  `similar_problems`, `free_search`) returns useful, bounded
  results across all 16 indexed repos instead of empty / oversize
  / 400-erroring lanes.

## Source

Manual investigation 2026-05-01 — five MCP `cortex_query` calls
covering one example of each intent, cross-checked against the
running daemon's `/healthz`, `/v1/health/coverage`, and the
post-`888aeda` commit log. Results:
`docs/analysis/phase11h-cortex-query-recall/findings.md` (to be
written as part of §1.0).
