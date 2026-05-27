# phase20 baseline — 2026-05-27

Snapshot of the retrieval surface before phase20 starts shipping fixes.

## Acceptance harness baseline run

```
PASS=2 FAIL=8 SKIP=0
```

| # | criterion                          | result                           |
|---|------------------------------------|----------------------------------|
| 1 | cortex_query snippets ≥5           | **PASS** — 13 snippets           |
| 2 | Vectorizer coverage ≥95%           | FAIL — 99% missing               |
| 3 | Meili coverage ≥95%                | FAIL — 99% missing               |
| 4 | topic_search ≥1 card               | FAIL — 0                         |
| 5 | ≥3 auto-generated consolidations   | **PASS** — 28 auto (haiku)       |
| 6 | consolidation_costs non-empty      | FAIL — 0 buckets (ts=0 on docs)  |
| 7 | consolidation_lineage non-empty    | FAIL — manual doc has no refs    |
| 8 | graph nodes carry `n.id`           | FAIL — writer not stamping props |
| 9 | law_violations `law_id` filter     | FAIL — attr not filterable       |
| 10 | active_work surfaces phase20      | FAIL — empty                     |

## Notes that recalibrate the proposal

- **Auto-vs-manual consolidations**: the proposal said "32/41 manual"
  based on an early surface read; the live data shows 28/30 auto
  (haiku) for the cortex repo. The phase20 §2.8 acceptance lifts
  from "≥3 auto" to "the auto ratio holds across active repos" —
  consolidator-nightly **is** firing for cortex.
- Vectorizer "99.6% missing" reading from earlier audit confirmed
  by `/v1/status` — only 2 of 567 collections present (re-ingest
  did not run post-restart). Same for Meili 99%.
- Lineage extractor fails even on the corpus's most-cited
  consolidation because the extractor only reads `topics` (e.g.
  `session:<ULID>`, `file:<path>`); the bodies use plain Markdown
  refs (`crates/foo.rs:42`, `DEC-009`). §6 of the task addresses
  this.
- §10 (active_work empty) is independent of data plane —
  cortex-api tasks_loader bug.

## Files

- `status-pre.json` — full `/v1/status` envelope
- `coverage-pre.json` — full `/v1/health/coverage` envelope
- `consolidations-cortex-pre.json` — `consolidations/recent?repo=cortex&limit=50`
- `consolidations-global-pre.json` — `consolidations/recent?limit=50` (empty — global index absent in live setup)
