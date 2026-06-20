# phase0 — reindex content-addressable kinds after the Meili-key fix

Source: phase0_decision-fulltext-title-body-mismapped (2026-06-20).

## Why

`bootstrap_doc_id` was fixed (it emitted an invalid Meilisearch primary
key — `bootstrap:<repo>:<path>:<hash>` with `:` `/` `.`, rejected by
Meili so the doc silently failed to index). That bug affected EVERY
content-addressable kind, not just decisions:

- `Decision` → `cortex_decisions` — **already repaired** by
  `phase0_decision-fulltext-title-body-mismapped` (reindex + prune; 51→27).
- `LawViolation` → `cortex_laws` / governance index
- `Knowledge` → `cortex-<repo>-misc` (knowledge family)
- `Learning` → `cortex-<repo>-misc` (learning family)
- Bootstrap **artifacts** (`code` / `docs`) — bootstrap-keyed docs
  likewise never indexed; only live random-ULID docs persisted.

So the same residue (failed-to-index canonical docs + stale random-ULID
duplicates) very likely exists across these indexes. The fix to the
write path is live (fulltext-worker redeployed), but existing data needs
a one-time reindex/prune like decisions got.

## What Changes

- Audit each content-addressable index for: (a) docs whose id is NOT
  `bootstrap-`-keyed (stale legacy), and (b) missing canonical docs.
- Generalise the `decisions-reindex` approach (or add per-kind reindex
  commands) to re-emit knowledge/learning/law sources through the builder
  with the stable `bootstrap-` key and prune legacy docs.
- For bootstrap artifacts (code/docs), confirm whether a full
  re-bootstrap is the right repair, or a targeted reindex.
- Extend the doctor check to flag non-`bootstrap-`-keyed content-
  addressable docs across all affected indexes.

## Impact
- Affected specs: `docs/specs/08-fulltext-indexer.md` (§Identity already
  updated for the key form; add the per-kind reindex contract).
- Affected code: `crates/cortex-cli/src/bin/cortex-ops/` (reindex +
  doctor commands); possibly `crates/cortex-workers/src/fulltext/`.
- Breaking change: NO (data repair; write path already fixed).
- User benefit: knowledge/learning/law/artifact retrieval returns the
  canonical, deduplicated docs instead of stale random-ULID residue.
