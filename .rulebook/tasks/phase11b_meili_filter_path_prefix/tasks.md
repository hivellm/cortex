## 1. Index-side: add `path_prefixes` projection
- [ ] 1.1 Identify the fulltext-worker per-document projection function (where it writes `{ path, kind, body, ... }` per envelope)
- [ ] 1.2 Add a pure helper `path_prefixes(path: &str) -> Vec<String>` that returns every ancestor segment plus the full path (e.g. `crates/`, `crates/cortex-api/`, `crates/cortex-api/src/`, `crates/cortex-api/src/meili_lane.rs`)
- [ ] 1.3 Wire the helper into the projection so each indexed document carries `path_prefixes: [..]`
- [ ] 1.4 Unit-test the helper for empty path, single-segment, deep nesting, and trailing-slash inputs

## 2. Index settings: declare `path_prefixes` filterable
- [ ] 2.1 Locate the Meili `settings.v1.json` file(s) per tier
- [ ] 2.2 Add `path_prefixes` to `filterableAttributes`
- [ ] 2.3 Bump the tooling-only `version` marker (existing strip-at-boundary pattern stays in place)
- [ ] 2.4 Confirm the worker re-applies settings on boot when the version differs from the live index

## 3. Filter generator: emit valid Meili
- [ ] 3.1 In `crates/cortex-api/src/meili_lane.rs::build_filter`, replace the `path STARTS WITH ...` branch with `path_prefixes IN [...]`
- [ ] 3.2 Each scope.files entry contributes one normalised prefix string (collapse trailing slashes, escape single quotes via the existing `quote_meili`)
- [ ] 3.3 Compose with the outer `AND` exactly as today (wrap in parens)
- [ ] 3.4 Update the existing unit tests in `meili_lane.rs::tests` to assert the new filter shape
- [ ] 3.5 Add a test that the generator produces a filter Meili accepts (parse via the SDK's filter validator if exposed; otherwise hit a real index in an `it_meili` integration test gated by `CORTEX_MEILI_IT=1`)

## 4. Re-index existing data
- [ ] 4.1 On worker boot, when settings version mismatches, the worker re-indexes every per-project index from the archive (existing path)
- [ ] 4.2 Confirm the re-index pass populates `path_prefixes` on every artifact / doc / code envelope
- [ ] 4.3 Smoke: hit `cortex_query` with `scope.files = ["crates/cortex-api/src/"]` and assert non-empty keyword hits all start with that prefix

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
