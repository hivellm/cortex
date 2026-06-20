# phase0 — laws: index routing vs law_check expectation + malformed cortex_laws docs

Source: phase0_reindex-content-addressable-after-meili-key-fix §2.3
(2026-06-20 live audit).

## Why

Two coupled problems found in the `cortex_laws` global index:

1. **Malformed docs.** All 240 `cortex_laws` docs have `title == id` (a
   random ULID) and `body` = the stringified payload
   (`{"body": "<.claude/rules markdown>", …}`) instead of the extracted
   prose. They were NOT produced by the current `build_doc` `Kind::Law`
   arm (which correctly derives `title = "<law_id>: <title>"` and clean
   `body`) — they predate it / came from a bypassing emit path. They are
   also legacy random-ULID-keyed, not `bootstrap-`-keyed (same Meili-key
   bug fixed for the other kinds).

2. **Routing vs read mismatch.** `routing::index_for_event_global` sends
   only `Kind::LawViolation` to the global `cortex_laws`; `Kind::Law`
   definitions route per-repo (`cortex-<slug>-governance`) only. But
   `cortex-api/src/search/strategies.rs::law_check` reads the GLOBAL
   `cortex_laws` expecting law DEFINITIONS (its comment: "switching the
   keyword lane to the global `cortex_laws` index surfaces both the law
   title AND the body excerpt"). So law definitions never reach the index
   `law_check` queries — `law_check` only works today because the 240
   malformed definition docs happen to sit in `cortex_laws` from the old
   path.

So a mechanical reindex is not enough: the intended contract for which
index holds law definitions must be settled first, then the emit/routing
fixed, then the malformed docs repaired.

## What Changes

- Settle the contract: `cortex_laws` (global) is the law-DEFINITION index
  `law_check` reads (matches the documented intent). Confirm whether
  `LawViolation` should stay there too or move to a violations index, so
  definitions and violations don't collide.
- Fix routing so `Kind::Law` definitions dual-write to `cortex_laws`
  (add to `index_for_event_global`), mirroring decisions.
- Confirm the law emit path (`emit_law_imported` /
  `emit_spec_laws_imported` / `emit_extracted_laws_imported`) produces a
  payload the `build_doc` `Kind::Law` arm parses (clean title/body), and
  fix the path that produced the stringified-body docs.
- Re-emit law definitions from `.claude/rules` (+ spec-extracted) through
  the builder with the stable `bootstrap-` key; prune the 240 malformed
  legacy docs. Verify `law_check` returns real law title+body and the
  dashboard law/violation counts stay correct.

## Impact
- Affected specs: `docs/specs/08-fulltext-indexer.md` (governance
  routing), spec(s) covering law_check / governance.
- Affected code: `crates/cortex-workers/src/fulltext/routing.rs`
  (`index_for_event_global`), builders/emitter law path,
  `crates/cortex-cli/src/bin/cortex-ops/` (law reindex), possibly
  `cortex-api` law_check + dashboard governance read.
- Breaking change: NO (index-content + routing repair; reads stay
  back-compatible / improve).
- User benefit: `law_check` returns real law definitions by title/body;
  no malformed `title==id` rows; laws indexed under stable keys.
