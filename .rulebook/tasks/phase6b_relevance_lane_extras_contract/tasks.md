## 1. Contract definition
- [x] 1.1 In `crates/cortex-api/src/lanes.rs`, declare a public `pub const LANE_EXTRAS_KEYS: &[&str] = &["decision_id", "decision_status", "supersedes", "turn_id", "model", "summary", "law_id", "severity"]` so every lane impl can reference the same vocabulary
- [x] 1.2 Add doc comments on `LaneHit.extras` referencing the contract and pointing at `docs/specs/11-query-api.md` §Lane projection contract

## 2. Meili lane projection
- [x] 2.1 In `crates/cortex-api/src/meili_lane.rs`, extend the projection function (renamed from `project` — kept private; new `pub(crate) fn project_doc` test seam wraps deserialise + project) to copy every key in `LANE_EXTRAS_KEYS` from the document body when present
- [x] 2.2 Source path: lookup precedence `_meta.<key>` → top-level `<key>` (captured via `#[serde(flatten)] extras_raw: serde_json::Map`) → typed slot fallback for `summary` / `severity` whose typed fields would otherwise shadow the flatten capture
- [x] 2.3 When `kind = "decision"` but `decision_id` is absent, emit `tracing::debug!(doc_id = %..., "decision row without decision_id — worker projection gap")` so worker-side projection bugs surface
- [x] 2.4 Round-trip the existing `source = "keyword"` + `score` extras unchanged

## 3. Vectorizer lane projection
- [x] 3.1 In `crates/cortex-api/src/vectorizer_lane.rs`, extend the `LaneHit` projection (with new `pub(crate) fn project_search_result` test seam wrapping the existing private `project`) to copy every key in `LANE_EXTRAS_KEYS` from `metadata.*`
- [x] 3.2 Lookup precedence: `metadata.<key>` (current Vectorizer ≥ 3.0.3 bootstrap shape) → `metadata.payload.<key>` (legacy embedder-worker nesting). Top-level wins on conflict so a mid-rollout corpus migrates cleanly.
- [x] 3.3 Round-trip existing extras (`source = "vector"`, `collection`, score) unchanged

## 4. Lane contract regression guard
- [x] 4.1 Add a new test module `crates/cortex-api/src/lane_contract.rs` declared under `#[cfg(test)] mod lane_contract;` with a fixture builder (`full_contract_values()`) that returns every `LANE_EXTRAS_KEYS` key with distinguishable values so a swap regression is caught
- [x] 4.2 Drive the Meili projection (`project_doc`) and assert every contract key lands on `LaneHit.extras` 1:1; plus a `_meta` precedence case asserting `_meta.<key>` wins over the top-level `<key>` when both are set
- [x] 4.3 Drive the Vectorizer projection (`project_search_result`) against an analogous fixture and assert the same; plus a `payload.*` fallback case + a `metadata.* > metadata.payload.*` precedence case
- [x] 4.4 "Missing keys round-trip as absent" case for both lanes — fixture without any contract key MUST yield a `LaneHit` whose `extras.contains_key(<key>) == false` for every key in `LANE_EXTRAS_KEYS`

## 5. End-to-end overlay test
- [x] 5.1 In `crates/cortex-api/tests/http.rs`, `decision_overlay_surfaces_decision_id_from_extras` seeds `MemoryKeywordLane` at `cortex-cortex-decisions` with a hit carrying `decision_id` + `decision_status`, runs `/v1/query` with `intent = decision_lookup`, and asserts `results.decisions[0].id == "DEC-0042"` + `status == "accepted"`
- [x] 5.2 `similar_turns_overlay_surfaces_turn_id_from_extras` seeds `MemoryVectorLane` at `cortex-cortex-turns` with a hit carrying `turn_id` + `model` + `summary`, runs `/v1/query` with `intent = similar_problems`, and asserts `results.similar_turns[0].turn_id` + `model` + `summary` round-trip 1:1

## 6. Spec docs
- [x] 6.1 In `docs/specs/11-query-api.md`, added a "Lane projection contract (phase6b)" subsection documenting `LANE_EXTRAS_KEYS`, the upstream-source / consumer table, and lookup precedence per lane
- [x] 6.2 Cross-link from `docs/analysis/relevance/01-findings.md` §F-007 — "Tracked by" line now points at `phase6b_relevance_lane_extras_contract` and lists the regression-guard + spec locations

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` per §6 + closure note in `docs/analysis/relevance/01-findings.md` §F-007
- [x] 7.2 Write tests covering the new behavior — 8-case lane-contract regression guard at `crates/cortex-api/src/lane_contract.rs` + 2-case end-to-end overlay tests in `crates/cortex-api/tests/http.rs`
- [x] 7.3 Run tests and confirm they pass — `cargo test -p cortex-api --lib --tests` 128 + 0 + 6 + 22 + 5 + 3 + 6 = 170 green (was 162; +8 lane_contract / +2 http overlay). `cargo clippy -p cortex-api --lib --tests` shows 7 warnings, all pre-existing in `dashboard.rs` (`type_complexity`, `dead_code` on `GraphBuilder` + `label_to_kind` + `node_label`, `needless_range_loop`, `unnecessary_filter_map`); zero warnings on phase6b-touched files.
