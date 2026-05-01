## 1. Boot-time inventory + observability
- [ ] 1.1 Define an `expected_collections(slug)` helper that emits the canonical set per spec 08 (`code, docs, misc, governance, decisions, turns, memory, analyses, laws` × every project slug)
- [ ] 1.2 At cortex-api boot, after the lane probes pass, list the live Vectorizer collections via the SDK's `list_collections` and diff against the expected set
- [ ] 1.3 Log a `WARN` per missing collection with `vectorizer_url`, `slug`, `kind`, and the routing-matrix entry that depends on it
- [ ] 1.4 Repeat the same diff for Meili indexes

## 2. Health surface
- [ ] 2.1 Extend `/v1/health/lanes` (or add a new `/v1/health/coverage`) returning `{ vector: { expected: [..], present: [..], missing: [..] }, keyword: {..}, graph: {..} }`
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
