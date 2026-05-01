## 1. Boot-time inventory + observability
- [x] 1.1 Define an `expected_collections(slug)` helper that emits the canonical set per spec 08 (`code, docs, misc, governance, decisions, turns, knowledge, learnings, analyses` × every project slug) — landed in [crates/cortex-api/src/coverage.rs](crates/cortex-api/src/coverage.rs)
- [x] 1.2 At cortex-api boot, after the lane probes pass, list the live Vectorizer collections via `GET /collections` and diff against the expected set — landed in `audit_coverage_at_boot` in [crates/cortex-api/src/main.rs](crates/cortex-api/src/main.rs)
- [x] 1.3 Log a `WARN` per missing collection with `vectorizer_url`, `slug`, `family`, and the backend label
- [x] 1.4 Repeat the same diff for Meili indexes (covers stale 7 non-canonical legacy indexes as `unexpected`)

## 2. Health surface
- [x] 2.1 Add a new `/v1/health/coverage` endpoint returning `{ slugs, families, backends: [{backend, base_url, severity, expected_count, present_count, missing_count, unexpected_count, present, missing, unexpected, error}], overall_severity }` — landed in [crates/cortex-api/src/http.rs](crates/cortex-api/src/http.rs#L300)
- [ ] 2.2 Dashboard Health view renders the new section under the existing extras column
- [ ] 2.3 `cortex-ops doctor-coverage` CLI wrapper exits with severity 0/1/2 mapping to `complete / partial / empty`

## 3. Per-intent diagnostic when a collection is missing
- [ ] 3.1 In the vector lane (and keyword lane), when `not found` / `404` is observed, capture the collection name in a synthetic empty `LaneHit` with `extras["collection_missing"] = true`
- [ ] 3.2 Behind `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1` (default 0), the orchestrator forwards those notes into `debug.notes[]` on the response
- [ ] 3.3 Default behaviour stays fail-open (no change to the JSON surface when the env is unset)

## 4. Writer routing audit (embedder + fulltext)
- [ ] 4.1 Read `crates/cortex-embedder/`'s per-event dispatch and confirm `Decision` / `Turn` / `Memory` / `Analysis` / `LawViolation` envelopes are fanned out to `cortex-{slug}-{kind}` collections
- [ ] 4.2 If the dispatch is missing, add the per-kind branch and the collection-creation guard
- [ ] 4.3 Same audit on `crates/cortex-fulltext/` for the corresponding Meili index per kind
- [ ] 4.4 Each new collection / index gets a creation pass that uses the same settings JSON contract the existing `code` / `docs` collections use (dim 512, BM25 provider, filterable attributes; phase11b adds `path_prefixes`)

## 5. Backfill
- [ ] 5.1 Add a `--kinds=<csv>` filter to `cortex-bootstrap` that limits the replay to the named envelope kinds
- [ ] 5.2 Run `cortex-bootstrap --reindex --kinds=decisions,turns,memory,analyses,laws` against the host's `~/.cortex/archive` and confirm the new collections fill
- [ ] 5.3 `cortex_query` `decision_lookup` "why Meilisearch instead of Lexum" now returns at least one hit

## 6. cortex_status honest reporting
- [ ] 6.1 Trace where `cortex-api` builds the `indexed_repos` list and identify the source backend
- [ ] 6.2 Either (a) make the bootstrap pipeline push the other 14 repos through the embedder, OR (b) split the field into `indexed_repos: { vectorizer: [..], meili: [..], nexus: [..] }` so the report is per-backend honest
- [ ] 6.3 Document the choice in `docs/operations/coverage.md`

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
