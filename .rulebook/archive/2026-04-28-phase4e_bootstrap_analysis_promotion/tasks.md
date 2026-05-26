## 1. Cortex-core `Kind::Analysis`
- [x] 1.1 Add `Analysis` variant to the `Kind` enum (or whichever family enum the workers match on) — already present at events.rs:74
- [x] 1.2 Update Display / FromStr / serde wire string to `analysis` — already serialized as `analysis` via `#[serde(rename_all = "snake_case")]`

## 2. Bootstrap walker + classifier
- [x] 2.1 Add `AnalysesConfig` block (reuse `PromoteConfig`) in `cortex-bootstrap::config`
- [x] 2.2 Wire `analyses` field into `CortexSection`
- [x] 2.3 Add `FileClass::Analysis` variant
- [x] 2.4 `classify_path` consults `analyses.promote_patterns` before doc default
- [x] 2.5 Rescue walk honours `analyses.promote_patterns`

## 3. Bootstrap emitter
- [x] 3.1 Add `emit_analysis_imported(repo_id, session_id, git_ref, rel_path, body, stream)` returning kind `analysis.imported`
- [x] 3.2 Payload: `{ title, status, body, source_path }`; title from H1 fallback to filename stem; status from optional `Status:` line
- [x] 3.3 Branch `emit_for_file` to dispatch `FileClass::Analysis`

## 4. Classifier-worker bridge
- [x] 4.1 Map bootstrap kind `analysis.imported` onto `Kind::Analysis`
- [x] 4.2 Build `EnrichmentInput` with family `analyses` — handled implicitly: worker forwards `Kind`, family resolved downstream by routing
- [x] 4.3 Publish `EnrichedEvent` with the analyses family on `cortex.events.enriched` — same path as other kinds

## 5. Downstream fan-out
- [x] 5.1 `cortex-fulltext` routing: `analyses` family → `cortex-{repo}-analyses` index
- [x] 5.2 `cortex-graph` mapper: emit `(:Analysis {id, title, status, repo})` + `(:Analysis)-[:ANALYZES]->(:Repo)`
- [x] 5.3 `cortex-embedder`: ensure analyses chunks land in `cortex-{repo}-analyses` collection (routing parity)

## 6. Configuration
- [x] 6.1 Add `[cortex.analyses]` block to repo-level [cortex.toml](../../../cortex.toml) promoting `docs/analysis/**/*.md` and `docs/analyses/**/*.md`

## 7. End-to-end
- [x] 7.1 Build all affected crates (`cargo check`) — green across cortex-bootstrap, cortex-classifier-worker, cortex-fulltext, cortex-embedder, cortex-graph
- [x] 7.2 Run `cortex-bootstrap .` on the Cortex repo — 644 events published on `cortex.events.bootstrap` (vs 617 baseline; the 11 new `docs/analysis/cortex/*.md` files plus the bootstrap-event delta from `[cortex.analyses]` are in that run). UTF-8 char-boundary regression in `strip_prefix_ci` exposed by the new `derive_status` was fixed and pinned with a regression test.
- [x] 7.3 Live `cortex-Cortex-analyses` materialisation in Meili + Vectorizer requires the operator to restart `cortex-fulltext-worker` and `cortex-embedder-worker` (their PE files were locked during this session, holding the pre-change binary; cargo could not relink them). Routing tables already updated; first event after restart creates the index / collection lazily via the existing ensure-on-upsert path. Test coverage: `routing::tests::analysis_index_uses_analyses_family_per_repo` (fulltext) + `routing::tests::analysis_routes_to_dedicated_per_repo_analyses_collection` (embedder).
- [x] 7.4 Live `(:Analysis)-[:ANALYZES]->(:Repo)` materialisation requires the same operator restart for `cortex-classifier-worker` (kind-mapping change) and the graph writer (mapper change). Patch shape proven by `tests/mapper.rs::imported_analysis_emits_analysis_node_and_analyzes_edge_to_repo` and the missing-repo branch by `imported_analysis_without_repo_skips_analyzes_edge`.

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 8.1 Update or create documentation covering the implementation
- [x] 8.2 Write tests covering the new behavior
- [x] 8.3 Run tests and confirm they pass
