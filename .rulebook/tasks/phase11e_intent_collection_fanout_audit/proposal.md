# Proposal: phase11e_intent_collection_fanout_audit

## Why

Verified live on 2026-05-01: `cortex_query` with `intent = decision_lookup` and `intent = similar_problems` returns `results: {}` (empty) for every query, even ones with obvious matches in the indexed corpus.

Concrete probe results:

| Intent | Query | Expected (in repo) | Got |
|--------|-------|--------------------|-----|
| `decision_lookup` | "why Meilisearch instead of Lexum" | Memory entry "Cortex — early decisions: Meilisearch (não Lexum) por ora" + several knowledge entries | `results: {}` |
| `similar_problems` | "Electron custom titlebar drag region clicks" | Commits `5cff3c9`, `020d612` + the `phase2_gui_*` task tree | `results: {}` |
| `pre_change_context` | "cortex-embedder Vectorizer integration..." | Many hits | 5 keyword hits, 0 vector hits |

**Root causes (two stacked issues):**

1. **Vectorizer is missing the per-intent collections.** Direct curl (`GET /collections` with admin JWT) returns exactly 4 collections, all for the `cortex` repo:

   ```
   cortex-cortex-code         456 vectors
   cortex-cortex-docs         394 vectors
   cortex-cortex-misc          97 vectors
   cortex-cortex-governance    29 vectors
   ```

   No `cortex-cortex-decisions`, no `cortex-cortex-turns`, no `cortex-cortex-memory`, no `cortex-cortex-analyses`. The 14 OTHER repos `cortex_status` lists as "indexed" (`csharp`, `go`, `gui`, `java`, `nexus`, `project-v`, `python`, `rulebook`, `rust`, `synap`, `tests`, `tml`, `typescript`, `vectorizer`, `x`) have ZERO collections. So `cortex_status.indexed_repos` is reporting on Meili (or some other index source), not the Vectorizer.

2. **The orchestrator's intent → collection fan-out probably hits the missing collections.** Spec 11 §Routing maps each intent to a set of collections; for `decision_lookup` it expects to walk a `*-decisions` collection that doesn't exist. The vector lane returns `not found` (which `vectorizer_lane.rs:203` correctly maps to empty hits — phase10g) so the lane silently contributes nothing. The keyword lane's per-intent fan-out has the same issue if the Meili routing matrix references indexes that the fulltext-worker never created (spec 08 §Routing matrix — `cortex-{slug}-decisions`, `*-governance`, `*-misc`, `*-turns`).

The result is a **silent correctness regression at the orchestrator → lane boundary**: every intent that depends on a non-`code/docs` collection returns empty without a single error in `debug.errors`, because "collection missing" is the same code path as "collection found, no hits".

## What Changes

This task audits the gap and ships the fix in three layers.

### Layer A — Discovery / observability

1. **Boot-time collection inventory.** On `cortex-api` boot, after the lane probes succeed, log every collection the orchestrator's routing matrix expects vs every collection the Vectorizer server actually has. Mismatches log `WARN` per missing collection so the gap is visible at startup.
2. **`/v1/health/lanes` extras.** Extend the existing health aggregator (phase8a) to include a per-lane `collections_expected` / `collections_present` / `collections_missing` set. The dashboard's Health view already renders extras; this surfaces the gap operationally.
3. **`debug.errors[lane]` carries `collection_missing` distinctly from "no hits".** Today the lane swallows `not found` into `Ok(empty)`. Behind a `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS` env (default off), the lane stamps `extras["collection_missing"] = true` on a synthetic empty result so the orchestrator can attach `debug.notes` like "intent X requires collection Y which is not present". Off-by-default keeps the existing fail-open semantics.

### Layer B — Indexing the missing kinds

4. **Audit the embedder/fulltext writer routing.** Confirm that `cortex.events.enriched` envelopes of kind `Decision`, `Turn`, `Memory`, `Analysis`, `LawViolation` actually get fanned out to per-kind collections (Vectorizer write path) and per-kind indexes (Meili write path). If they don't, fix the writer's per-kind dispatch.
5. **Per-kind collection naming.** Decide and document the canonical schema (`cortex-{slug}-{kind}` is the existing convention per spec 08 §Routing matrix). Update the embedder + fulltext workers to use it consistently.
6. **Backfill via the bootstrap CLI.** `cortex-bootstrap` already replays the archive; ensure the replay covers every kind. Run a one-shot `cortex-bootstrap --reindex --kinds=decisions,turns,memory,analyses,laws` against the host's `~/.cortex/archive` so existing decisions/turns appear in the new collections.

### Layer C — Multi-repo coverage

7. **Audit the boot-time `indexed_repos` discrepancy.** `cortex_status` reports 15 indexed repos but the Vectorizer has 4 collections all for `cortex`. Trace where that 15-element list comes from (likely Meili or the rulebook tasks loader, not the Vectorizer) and either (a) get the bootstrap pipeline to actually push the other 14 repos through the embedder, or (b) make `indexed_repos` honest about which backend has data for which repo (e.g. `{ "vectorizer": ["cortex"], "meili": [...], "nexus": [...] }`).

## Impact

- Affected code:
  - [crates/cortex-api/src/main.rs](crates/cortex-api/src/main.rs) — boot-time inventory log.
  - [crates/cortex-api/src/lanes/](crates/cortex-api/src/) — `collection_missing` marker on synthetic empty results.
  - [crates/cortex-api/src/dashboard.rs](crates/cortex-api/src/dashboard.rs) — `/v1/health/lanes` extras for collections.
  - [crates/cortex-embedder/](crates/cortex-embedder/) + [crates/cortex-fulltext/](crates/cortex-fulltext/) — per-kind dispatch verification + fix.
  - [crates/cortex-bootstrap/](crates/cortex-bootstrap/) — `--kinds` filter for the reindex pass.
- Breaking change: NO at the API surface. Adds new collections + indexes; existing callers see strictly more results.
- User benefit: `decision_lookup` and `similar_problems` actually return hits. `cortex_status` reports honest per-backend coverage.

## Source

- Live curl against `cortex-vectorizer` (2026-05-01): exactly 4 collections, listed above.
- Live MCP probe (2026-05-01): every `decision_lookup` / `similar_problems` returns `results: {}`.
- Spec 08 §Routing matrix — defines the `cortex-{slug}-{decisions,governance,misc,turns,code,docs}` index naming convention.
- Spec 11 §Fan-out + fusion — defines the per-intent collection set that the orchestrator queries.
- Phase10g `health_route_registration` — pattern to extend for the lanes inventory.
