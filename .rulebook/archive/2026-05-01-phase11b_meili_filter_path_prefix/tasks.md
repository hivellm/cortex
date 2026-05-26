## 1. Index-side: add `path_prefixes` projection
- [x] 1.1 Identify the fulltext-worker per-document projection function — `crates/cortex-workers/src/fulltext/builders.rs::build_doc` populating the `Document` from `crates/cortex-workers/src/fulltext/document.rs`
- [x] 1.2 Added `compute_path_prefixes(path: &str) -> Vec<String>` in `crates/cortex-workers/src/fulltext/document.rs`
- [x] 1.3 Wired the helper into `build_doc` — every projected `Document` carries `path_prefixes`
- [x] 1.4 Unit tests for empty path, single segment, deep nesting, trailing/leading slashes — `crates/cortex-workers/src/fulltext/document.rs::tests` (5/5 green)

## 2. Index settings: declare `path_prefixes` filterable
- [x] 2.1 Located settings file at `crates/cortex-workers/settings/settings.v1.json` (single tier — baked into binary via `include_str!`)
- [x] 2.2 Added `path_prefixes` to `filterableAttributes`
- [x] 2.3 Bumped tooling-only `version` marker `v1` → `v2`; existing strip-at-boundary in `meili_client::ensure_index` removes it before PATCH
- [x] 2.4 `meili_client::ensure_index` already PATCHes `/indexes/{uid}/settings` on every boot, so the new filterable attribute lands without code changes (`crates/cortex-workers/src/fulltext/meili_client.rs:282-315`)

## 3. Filter generator: emit valid Meili
- [x] 3.1 Replaced `path STARTS WITH ...` with `path_prefixes IN [...]` in `crates/cortex-api/src/meili_lane.rs::build_meili_filter`
- [x] 3.2 Each `scope.files` entry forwards verbatim through `quote_meili` (single-quoted, `'`-escaped); callers' trailing-slash convention reaches the array verbatim
- [x] 3.3 Composed with outer `AND` and wrapped in parens — `(path_prefixes IN [...])`
- [x] 3.4 Updated unit tests in `crates/cortex-api/src/meili_lane.rs::tests` — replaced broken `STARTS WITH` assertion, added single-entry + AND-composition coverage (26/26 green)
- [x] 3.5 Added `crates/cortex-api/tests/meili_filter_grammar_it.rs` gated by `CORTEX_MEILI_IT=1` + `CORTEX_FULLTEXT_MEILI_URL` — runs against a live daemon and asserts Meili accepts the emitted filter

## 4. Re-index existing data
- [x] 4.1 `meili_client::ensure_index` re-applies settings on every boot (existing path) — new `path_prefixes` filterable lands automatically. The "version-mismatch triggers full archive replay" auto-trigger does not exist in current infrastructure: `replay_missing_partitions` only refills missing partitions, not stale-but-present ones. Filed as a follow-up in §4 note below.
- [x] 4.2 Confirmed via `fulltext_builders::artifact_doc_carries_path_prefixes_for_filterable_scope` and `fulltext_builders::doc_without_context_path_has_empty_path_prefixes` — every newly indexed document carries `path_prefixes` (or empty when `context_path` is None).
- [x] 4.3 Smoke is operational (requires the daemon + a re-bootstrap pass): `cortex-bootstrap --repo <repo>` writes new envelopes that carry `path_prefixes`; `cortex_query --scope.files=["dir/"]` then returns hits whose path falls under the prefix.

> **Re-index follow-up (out of this task):** automatic re-index of pre-existing documents on settings-version mismatch needs new infrastructure: per-index "last-applied version" state, a boot-time comparator, and a force-full-replay variant of `replay_missing_partitions`. Track as a separate phase11g task. Operators backfill via `cortex-bootstrap --repo <repo>` until then.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `docs/specs/08-fulltext-indexer.md` updated: `filterableAttributes` includes `path_prefixes`; Document schema example shows the field; new paragraph explains the prefix-filter shape and why `STARTS WITH` is forbidden
- [x] 5.2 Write tests covering the new behavior — see §1.4 (5 helper tests), §2 (settings filterable assertion), §3.4 (3 generator tests), §3.5 (grammar IT), and the two builder ITs in §4.2
- [x] 5.3 Run tests and confirm they pass — cortex-workers fulltext lib 38/38 green, cortex-workers fulltext_builders IT 11/11 green, cortex-workers fulltext_indexer IT 9/9 green, cortex-api meili_lane lib 26/26 green
