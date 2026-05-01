# Coverage audit — Vectorizer + Meilisearch inventory diff

> **Phase:** phase11e (in progress) · **Owner:** Core team · **Status:** 🟡 Partial (sections 1 + 2 + 3 + 4 + 6 of 7; §5 backfill pending)

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

## In `/v1/status` and `cortex_status` MCP (phase11e §6)

The boot-time audit's result is cached in `QueryService::coverage_snapshot`
so `/v1/status` (and the `cortex_status` MCP tool that wraps it)
can carry a compact per-backend summary without re-running the
diff on every call:

```json
{
  "service": "cortex-api",
  "version": "0.1.0",
  "indexed_repos": [...],   // legacy keyword-lane view, kept for backwards compat
  "coverage": {
    "overall_severity": "warn",
    "backends": [
      { "backend": "vectorizer", "severity": "warn",
        "expected": 144, "present": 4,  "missing": 140, "unexpected": 0 },
      { "backend": "meili",      "severity": "warn",
        "expected": 144, "present": 29, "missing": 115, "unexpected": 7 }
    ],
    "details_endpoint": "/v1/health/coverage"
  }
}
```

The summary fits well inside the MCP transport's per-tool-result
size cap; callers that need the per-collection breakdown follow
the `details_endpoint` link to the on-demand handler.

## Configuration

| Env | Default | Notes |
|-----|---------|-------|
| `CORTEX_COVERAGE_SLUGS` | (unset) | CSV of extra slugs to pin in the audit, beyond what the keyword lane has seen. Useful when a new repo has been added but no envelopes have flowed yet. |
| `CORTEX_VECTORIZER_URL` | (unset) | Skipped backend when unset. Auth follows phase11a precedence (`_API_KEY` > `_USER`+`_PASSWORD`). |
| `CORTEX_FULLTEXT_MEILI_URL` | (unset) | Skipped backend when unset. Auth via `CORTEX_FULLTEXT_MEILI_API_KEY` or `MEILI_MASTER_KEY`. |

## What this audit does NOT fix

The audit is **observability**: it makes the gap visible. It does
not:

- Backfill envelopes into the missing collections (§5 of phase11e —
  cortex-bootstrap `--kinds` reindex pass; pending).

The structural pieces (per-kind dispatch in the writer side and
shared settings schema) were already in place before phase11e
started — see the "Writer routing verification" section below.

## Writer routing verification (phase11e §4)

§4 was an **audit** of whether the writer pipeline ROUTES per-kind
envelopes correctly. The audit confirms every Kind variant has a
defined target — the gap is upstream (no envelopes of those kinds
flowing through), NOT in the writer's routing table.

### Embedder (Vectorizer write path)

[`crates/cortex-workers/src/embedder/routing.rs`](../../crates/cortex-workers/src/embedder/routing.rs)
defines `family_for(Kind)`:

| Kind          | Collection family |
|---------------|-------------------|
| `ToolCall`    | `code`            |
| `Artifact`    | `docs` (event-level) — chunk routing splits Code/Doc per `ChunkSource` |
| `Decision`    | `decisions`       |
| `Turn`        | `turns`           |
| `LawViolation`| `governance`      |
| `Analysis`    | `analyses`        |
| `Knowledge`   | `knowledge`       |
| `Learning`    | `learnings`       |
| `AgentCall` / `Memory` | `misc` (catch-all) |

Final collection name: `{prefix}-{slug}-{family}` per
`cortex_storage::names::repo_scoped_name`. The embedder passes every
event through `collection_for_chunk` and creates the target
collection lazily on first upsert via
`LiveVectorizerClient::ensure_collection`. **No code change needed
for §4.** The Vectorizer hosts only `cortex-cortex-{code,docs,misc,governance}`
because no Decision / Turn / Memory / Analysis / Knowledge / Learning
envelopes have reached the embedder consumer — that's §5's territory.

### Fulltext (Meili write path)

[`crates/cortex-workers/src/fulltext/routing.rs`](../../crates/cortex-workers/src/fulltext/routing.rs)
mirrors the same family table via `family_for(Kind)`, plus
`family_for_event` which uses `(path, topics)` to split Artifact
into `code` / `docs` / `misc`. Index name follows
`cortex-{slug}-{family}` exactly. Settings come from a single shared
[`settings.v1.json`](../../crates/cortex-workers/settings/settings.v1.json)
applied lazily by `MeiliFulltextIndexer::ensure_settings` on first
encounter per index — same schema across every per-kind index, so
§4.4 ("settings JSON for new collections") is satisfied by reuse
rather than duplication. **No code change needed for §4.**

### Why the audit still shows missing names after §4 confirmed

The writer routing is complete; the gap is upstream of the writer
consumers. Concretely:

- The Vectorizer hosts 4 collections (all `cortex-cortex-*`).
- Meili hosts 29 indexes spread across multiple repos.
- The asymmetry is a function of which kinds the
  classifier-worker → ingestion → bootstrap pipeline has fanned
  out to each consumer subscription. §5 (cortex-bootstrap
  `--kinds` flag) replays the archive into the missing
  destinations.

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
