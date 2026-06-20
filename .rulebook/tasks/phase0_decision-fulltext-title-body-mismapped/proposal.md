# Proposal: phase0_decision-fulltext-title-body-mismapped

Source: phase28 manual E2E verification (run 1, 2026-06-20) — finding F2.

## Why

The `cortex_decisions` Meili index is built with the wrong field
mapping, degrading keyword search on decisions. Observed live via
`POST /v1/decisions/search`:

- `title` = the document ULID (e.g. `01KQNYF4JPF1480RQTP2F1A62F`) instead
  of the real decision title.
- `body` = a JSON-stringified payload (`{"body":"# 1. Bypass vectorizer-
  sdk ..."}`) instead of the clean decision text — double-encoded.

The dashboard read path (`GET /v1/dashboard/decisions`) shows the CORRECT
titles ("1. Bypass vectorizer-sdk for /insert ..."), so the data exists;
only the fulltext document builder maps it wrong. Net effect: you cannot
match a decision by its real title via keyword search, and the body is
polluted with JSON envelope noise, hurting BM25 relevance for the whole
decision corpus.

## What Changes

- Fix the decision document builder in the fulltext indexer
  (`crates/cortex-workers/src/fulltext/builders.rs` + routing) so the
  `cortex_decisions` Meili doc carries:
  - `title` = the decision's real title (the `## N. <title>` heading /
    the decision `title` field), not the id.
  - `body` = the decision's markdown/text content, not the
    JSON-serialized payload.
- Reindex `cortex_decisions` (or document the reindex step) so existing
  docs are corrected.
- Add a builder unit test asserting `title != id` and `body` is not a
  JSON object string.

## Impact

- Affected specs: spec 08 (fulltext indexer document schema), spec 27
  (decisions).
- Affected code: `crates/cortex-workers/src/fulltext/builders.rs` (+ the
  decision routing/mapping path); reindex tooling.
- Breaking change: NO (index content fix; re-emit corrects docs).
- User benefit: keyword search on decisions matches real titles + clean
  body; better BM25 relevance in the decision lane.
