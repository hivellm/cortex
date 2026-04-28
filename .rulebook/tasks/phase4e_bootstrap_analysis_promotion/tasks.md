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
- [ ] 7.1 Build all affected crates (`cargo check`)
- [ ] 7.2 Run `cortex-bootstrap .` on the Cortex repo and confirm `analysis.imported` events appear for the 11 `docs/analysis/cortex/*.md` files
- [ ] 7.3 Confirm `cortex-Cortex-analyses` materialises in Meili and Vectorizer
- [ ] 7.4 Confirm `(:Analysis)-[:ANALYZES]->(:Repo)` lands in Nexus

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 8.1 Update or create documentation covering the implementation
- [ ] 8.2 Write tests covering the new behavior
- [ ] 8.3 Run tests and confirm they pass
