# Coverage audit — Vectorizer + Meilisearch inventory diff

> **Phase:** phase11e (in progress) · **Owner:** Core team · **Status:** 🟡 Partial (sections 1 + 2.1 of 7)

The cortex-api daemon audits the live Vectorizer + Meilisearch
collection / index inventories against the routing matrix's expected
set at boot, and exposes the same diff on demand at
`GET /v1/health/coverage`. The audit answers two questions:

1. **What does the daemon expect?** Every `cortex-{slug}-{family}`
   name where `slug ∈` (slugs the keyword lane has seen +
   `CORTEX_COVERAGE_SLUGS`) and `family ∈` (`code`, `docs`,
   `decisions`, `turns`, `governance`, `analyses`, `knowledge`,
   `learnings`, `misc`).
2. **What does the live backend host?** `GET /collections` against
   the Vectorizer; `GET /indexes` against Meilisearch.

The diff produces three sets per backend: `present` (expected ∩ live),
`missing` (expected − live), `unexpected` (live − expected — orphans
or pre-canonical-naming debris).

## At boot

After the lane probes pass, `audit_coverage_at_boot` in
[crates/cortex-api/src/main.rs](../../crates/cortex-api/src/main.rs)
runs the diff for each backend and emits:

- One `INFO` summary line per backend:
  `coverage audit backend=vectorizer base_url=... expected=144 present=4 missing=140 unexpected=0 severity=Warn`
- One `WARN` per missing collection, with structured fields
  (`backend`, `collection`, `slug`, `family`) so an external log
  pipeline can group / count without re-parsing.

A non-zero `missing` count is **not** fatal — the daemon stays up
and queries fall through fail-open (the lane returns empty for the
missing collection). The audit's value is making the gap visible
instead of leaving it to surface only at query time.

## At runtime

```
GET /v1/health/coverage
```

Response:

```json
{
  "slugs": ["cortex", "rulebook", ...],
  "families": ["code", "docs", "decisions", "turns", "governance",
               "analyses", "knowledge", "learnings", "misc"],
  "backends": [
    {
      "backend": "vectorizer",
      "base_url": "http://vectorizer:15002",
      "severity": "warn",
      "expected_count": 144,
      "present_count": 4,
      "missing_count": 140,
      "unexpected_count": 0,
      "present":   ["cortex-cortex-code", "cortex-cortex-docs", ...],
      "missing":   ["cortex-cortex-decisions", "cortex-cortex-turns", ...],
      "unexpected": [],
      "error": null
    },
    { "backend": "meili", "base_url": "...", "severity": "warn", ... }
  ],
  "overall_severity": "warn"
}
```

`severity` is `ok` (every expected name present), `warn` (at least
one missing), or `critical` (nothing expected is present at all).
The CLI wrapper (`cortex-ops doctor-coverage`, item 2.3 — not yet
shipped) will map these to exit codes 0 / 1 / 2.

`error` is non-null when the upstream probe failed (e.g. server
unreachable, JSON shape mismatch). The diff still runs — `live`
falls back to empty, so every expected name lands in `missing` —
but the operator sees the transport error string in the field.

## Configuration

| Env | Default | Notes |
|-----|---------|-------|
| `CORTEX_COVERAGE_SLUGS` | (unset) | CSV of extra slugs to pin in the audit, beyond what the keyword lane has seen. Useful when a new repo has been added but no envelopes have flowed yet. |
| `CORTEX_VECTORIZER_URL` | (unset) | Skipped backend when unset. Auth follows phase11a precedence (`_API_KEY` > `_USER`+`_PASSWORD`). |
| `CORTEX_FULLTEXT_MEILI_URL` | (unset) | Skipped backend when unset. Auth via `CORTEX_FULLTEXT_MEILI_API_KEY` or `MEILI_MASTER_KEY`. |

## What this audit does NOT fix

The audit is **observability**: it makes the gap visible. It does
not:

- Create the missing collections (Layer B, item 4.4 — settings JSON
  for new collections).
- Backfill envelopes into them (Layer B + Layer 5 backfill).
- Reconcile the `cortex_status.indexed_repos` reporting per backend
  (Section 6 of phase11e).

Those land in subsequent phase11e commits.

## Why two backends are reported separately

The 2026-05-01 audit found a stark asymmetry:

| Backend | Expected | Present | Missing |
|---------|---------:|--------:|--------:|
| Vectorizer | 144 | 4 | 140 |
| Meili | 144 | 29 | 115 |

Same routing matrix, very different live state — the embedder is
populating four `cortex-cortex-*` collections only, while the
fulltext worker has a richer set across multiple repos. Reporting
per backend makes that asymmetry impossible to miss.
