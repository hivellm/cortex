## 1. Contract definition
- [ ] 1.1 In `crates/cortex-api/src/lanes.rs`, declare a public `pub const LANE_EXTRAS_KEYS: &[&str] = &["decision_id", "decision_status", "supersedes", "turn_id", "model", "summary", "law_id", "severity"]` so every lane impl can reference the same vocabulary
- [ ] 1.2 Add doc comments on `LaneHit.extras` referencing the contract and pointing at `docs/specs/11-query-api.md` §Lane projection contract

## 2. Meili lane projection
- [ ] 2.1 In `crates/cortex-api/src/meili_lane.rs`, extend `project_doc` (or its equivalent — the function that produces a `LaneHit` from a Meili hit) to copy every key in `LANE_EXTRAS_KEYS` from the document body when present
- [ ] 2.2 Source path: the meili_loader already preserves the original envelope body under `_meta` / direct fields — use `doc.get("_meta").and_then(|m| m.get(key))` first, then fall back to `doc.get(key)` so legacy + current shapes both work
- [ ] 2.3 When `kind = "decision"` but `decision_id` is absent, emit `tracing::debug!(?doc_id, "decision row without decision_id")` so worker-side projection bugs surface
- [ ] 2.4 Round-trip the existing `source = "keyword"` + `score` extras unchanged

## 3. Vectorizer lane projection
- [ ] 3.1 In `crates/cortex-api/src/vectorizer_lane.rs`, extend the `LaneHit` projection (the function that maps a `SearchResult` payload onto a `LaneHit`) to copy every key in `LANE_EXTRAS_KEYS` from `metadata.*`
- [ ] 3.2 When the metadata shape carries the key under `payload.*` instead, prefer `metadata.*` and fall back to `payload.*` (current Vectorizer ≥ 3.0.3 places turn fields under `metadata` per the bootstrap pipeline)
- [ ] 3.3 Round-trip existing extras (`source = "vector"`, score) unchanged

## 4. Lane contract regression guard
- [ ] 4.1 Add a new test module `crates/cortex-api/src/lane_contract.rs` (declared under `#[cfg(test)] mod lane_contract;`) with a fixture upstream doc that has every `LANE_EXTRAS_KEYS` key populated
- [ ] 4.2 Drive the Meili projection (`project_doc`) and assert every contract key lands on `LaneHit.extras` 1:1
- [ ] 4.3 Drive the Vectorizer projection (`project_search_result`) against an analogous fixture and assert the same
- [ ] 4.4 Add a "missing keys round-trip as absent" case — fixture without `decision_id` MUST yield a `LaneHit` whose `extras.get("decision_id") == None`

## 5. End-to-end overlay test
- [ ] 5.1 In `crates/cortex-api/tests/http.rs`, add a test that seeds the live `MeiliKeywordLane` (or its mock) with a decision-shaped doc carrying `decision_id` + `decision_status`, runs `/v1/query` with `intent = decision_lookup`, and asserts the response's `results.decisions` array is non-empty
- [ ] 5.2 Analogous test for `similar_turns` against a Vectorizer-shaped fixture with `turn_id` + `model` + `summary`

## 6. Spec docs
- [ ] 6.1 In `docs/specs/11-query-api.md`, add a "Lane projection contract" section documenting the `LANE_EXTRAS_KEYS` vocabulary, where each key originates upstream, and which overlay consumes it
- [ ] 6.2 Cross-link from `docs/analysis/relevance/01-findings.md` §F-007 (mark as closed-by phase6b on merge)

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` per §6
- [ ] 7.2 Write tests covering the new behavior — the lane contract regression guard from §4 plus the end-to-end overlay tests from §5
- [ ] 7.3 Run tests and confirm they pass — `cargo clippy -p cortex-api --all-targets -- -D warnings` and `cargo test -p cortex-api` both green
