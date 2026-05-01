# Proposal: phase11b_meili_filter_path_prefix

## Why

Any `cortex_query` call that includes `scope.files` fails the keyword (Meili) lane with:

```
400 Bad Request: "Was expecting an operation `=`, `!=`, `>=`, `>`, `<=`, `<`,
`IN`, `NOT IN`, `TO`, `EXISTS`, `NOT EXISTS`, `IS NULL`, `IS NOT NULL`,
`IS EMPTY`, `IS NOT EMPTY`, `CONTAINS`, `NOT CONTAINS`, `_geoRadius`,
or `_geoBoundingBox` at `path STARTS WITH 'crates/cortex-embedder/...'"
```

Verified live on 2026-04-30 with a `pre_change_context` call passing `scope.files = ["crates/cortex-embedder/src/vectorizer_client.rs"]` — the response carried `errors.keyword = "...invalid_search_filter..."` and the keyword lane returned no hits.

Root cause: [meili_lane.rs:347-359](crates/cortex-api/src/meili_lane.rs#L347-L359) emits `path STARTS WITH '<prefix>'`, but Meilisearch's filter grammar does **not** support `STARTS WITH`. The valid operator surface is the one the error message lists; for prefix matching, Meili expects either:

- `path = '<exact>'` for exact equality, or
- `path IN ['a', 'b']` for an exact-set match, or
- a **filterable array field** with one entry per ancestor path (e.g. `path_prefixes IN [...]`), or
- moving the filter into the search query (`q`) and relying on tokenisation, which breaks per-prefix scoping semantics.

The bug effectively turns every file-scoped `cortex_query` into a "keyword lane silently empty" call. The unit tests in [meili_lane.rs:1034-1038](crates/cortex-api/src/meili_lane.rs#L1034-L1038) only assert the broken filter shape — they pass because they check the *string we send*, not whether the server accepts it.

## What Changes

1. **Stop emitting `STARTS WITH`.** Replace the `path STARTS WITH '<prefix>'` emission in `meili_lane::build_filter` with a syntactically valid Meili expression.
2. **Index a `path_prefixes` array** at the fulltext-worker indexing path so prefix queries become `path_prefixes IN ['a/', 'a/b/', 'a/b/c.rs']` — a constant-cost filter Meili supports natively. The fulltext worker computes ancestors at index time (split on `/`, accumulate); the worker is the single writer of the index so back-fill is just a re-index pass.
3. **Mark `path_prefixes` filterable** on the Meili index settings JSON (`crates/cortex-fulltext/...settings.v1.json`) and bump the schema marker the loader strips (per the existing anti-pattern entry).
4. **Fix the broken unit tests** in `meili_lane.rs::tests` to assert the new `path_prefixes IN [...]` shape, and add an integration test that runs the produced filter against a live (or mocked-strict) Meili to catch grammar regressions.
5. **Re-index** existing per-project Meili indexes once the worker writes the new field. The fulltext worker already re-indexes on schema bump; bumping the settings version triggers it.

## Impact

- Affected code:
  - [crates/cortex-api/src/meili_lane.rs](crates/cortex-api/src/meili_lane.rs) — filter generator + tests.
  - [crates/cortex-fulltext/](crates/cortex-fulltext/) — index settings + per-document projection (add `path_prefixes`).
  - Settings JSON file (per-tier) — declare `path_prefixes` filterable, strip the version field at the wire boundary as before.
- Breaking change: NO at the API surface — `scope.files` callers don't change. NO at the index — re-index cost is one full pass per project (already the existing schema-bump path).
- User benefit: file-scoped retrieval works again. Every `pre_change_context` call with `scope.files` actually narrows by path instead of silently dropping the keyword lane.

## Source

- Live MCP probe transcript (2026-04-30): `errors.keyword = "...path STARTS WITH '...'... invalid_search_filter"`.
- [meili_lane.rs:347-359](crates/cortex-api/src/meili_lane.rs#L347-L359) — broken generator.
- [meili_lane.rs:1034-1038](crates/cortex-api/src/meili_lane.rs#L1034-L1038) — tests that pin the broken shape.
- Meilisearch docs: filter grammar and array-field `IN` semantics.
