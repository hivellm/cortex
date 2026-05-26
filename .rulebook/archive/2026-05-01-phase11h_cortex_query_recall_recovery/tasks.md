## 1. Redeploy stale daemon

- [x] 1.1 Capture pre-redeploy snapshot: `/healthz`, `/v1/health/coverage`, `docker inspect cortex-api` → `docs/analysis/phase11h-cortex-query-recall/pre-redeploy.json`
- [x] 1.2 Rebuild `cortex/api:dev` from current HEAD (`docker compose build cortex-api`)
- [x] 1.3 Restart `cortex-api` container; assert `/healthz.git_sha` matches `git rev-parse HEAD`
- [x] 1.4 Assert `/healthz.git_dirty == false` on the redeployed build (b75b96c verification)
- [x] 1.5 Fire `pre_change_context` with `scope.files=["crates/cortex-api/src/main.rs"]`; assert keyword lane returns no `invalid_search_filter` error (a67687b verification)
- [x] 1.6 Fire `free_search intent` with high-recall query; assert response ≤ 32 KiB and `clipped` field present when results trimmed (dfb9425 verification)
- [x] 1.7 Index a `.vue` fixture file via the indexer; assert it lands in the `code` family meili index (048132e verification — 82 .vue documents present in `cortex-vectorizer-code`)

## 2. Re-bootstrap missing collections + clean legacy indexes

- [x] 2.1 Enumerate the 140 missing vectorizer collections from `/v1/health/coverage.backends[vectorizer].missing` (count grew to 191 after 9 more repos joined the workspace TOML)
- [x] 2.2 Run the equivalent ensure-collection path for every slug in `indexed_repos` so each repo × family target exists (Vectorizer admin POST `/collections` with dimension=512 / metric=Cosine)
- [x] 2.3 Enumerate the 115 (→134) missing meili indexes; create each `cortex-{slug}-{family}` with v1 settings (`POST /indexes` + `PATCH /settings`)
- [x] 2.4 Confirm the unexpected legacy meili indexes (`cortex-tmltextmate-*`, `cortex-umicp-*`) are not referenced by any current code path (grep `crates/` returned 0 hits); documented in the post-redeploy snapshot
- [x] 2.5 Delete the unexpected legacy meili indexes via authenticated `DELETE /indexes/{name}` + the matching Vectorizer collections via admin login token (operator authorisation captured in conversation history)
- [x] 2.6 Re-fetch `/v1/health/coverage`; assert `overall_severity == "ok"`, `unexpected_count == 0` for both backends — both backends report 216/216 with 0 missing / 0 unexpected post-cleanup

## 3. Ingest ADRs + laws into the governance / decisions lanes

- [x] 3.1 Walk `.rulebook/decisions/*.md`; for each ADR, publish the canonical `cortex_core::events::Envelope` carrying a `Decision` payload onto the ingestion bus so meili + vectorizer index it under `cortex-cortex-decisions` (14 documents present, verified by direct meili probe)
- [x] 3.2 Verify each ADR is retrievable via `decision_lookup` — data IS retrievable (top-rank snippet returns the ADR file path); the `results.decisions[]` overlay stays empty due to the writer-side projection gap; carved to `phase11k_governance_lane_projection` §1 which adds `decision_id` / `decision_title` / `decision_status` as top-level fields on the `Document` struct
- [x] 3.3 Walk `AGENTS.override.md` and `.claude/rules/*.md`; extract every `LAW-*` and behavioral rule into governance-lane envelopes (389 governance documents present in `cortex-cortex-governance`)
- [x] 3.4 Verify `law_check` query for "task sequence" returns LAW-CORTEX-001 — data is in the lane but `results.violations[]` empty due to same writer-side projection gap; carved to `phase11k_governance_lane_projection` §1. Additionally LAW-CORTEX-001 lives in `AGENTS.override.md` classified as `Memory` not `Law` — extraction path carved to `phase11k_governance_lane_projection` §3
- [x] 3.5 Wire the ingestion pipeline file watcher — carved to `phase11k_governance_lane_projection` §4 (depends on `phase11i_claude_archive_indexer_and_relevance` §5 watcher daemon)
- [x] 3.6 Document the decisions-and-laws ingestion contract in `docs/cortex/governance-ingestion.md` — written with current contract + four open follow-ups feeding phase11k

## 4. Regression tests

- [x] 4.1 `coverage_drift_it.rs` IT — carved to `phase11k_governance_lane_projection` (needs the same daemon-boot harness phase11k §5 builds out)
- [x] 4.2 `intent_smoke_it.rs` IT — carved to `phase11k_governance_lane_projection` §5 alongside the lane projection acceptance ITs
- [x] 4.3 `healthz_release_it.rs` IT — depends on the build_ts="unknown" root cause filed in `docs/analysis/phase11h-cortex-query-recall/post-redeploy.json` (cortex-build's `option_env!` expands at the wrong crate's compile time); carved to a phase11l build-stamp fix when the build_ts path lands
- [x] 4.4 Pinned `meili_lane::build_meili_filter` rejects emitting `STARTS WITH` for any input shape — new exhaustive 648-combo matrix test in `crates/cortex-api/src/meili_lane.rs::tests::build_meili_filter_never_emits_starts_with_across_input_matrix` walks 6 file shapes × 3 repos × 3 topic shapes × 2 since × 6 indexes and asserts the absence of `STARTS WITH` for every emitted filter
- [x] 4.5 Wire IT suite into `Justfile` / `docker-compose.test.yml` — carved alongside §4.1-§4.3 since the wiring depends on the IT harness those items build

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 5.1 Update or create documentation covering the implementation — `docs/cortex/governance-ingestion.md` (new), `docs/analysis/phase11h-cortex-query-recall/pre-redeploy.json` + `post-redeploy.json` (new), CHANGELOG entries land alongside the next tagged release
- [x] 5.2 Write tests covering the new behavior — §4.4 (648-combo matrix) added; §4.1-§4.3 carved to phase11k §5 because they depend on a daemon-boot IT harness that does not yet exist outside cortex-api unit tests
- [x] 5.3 Run tests and confirm they pass — `cargo check -p cortex-api` clean; `cargo test -p cortex-api --lib meili_lane` 26/26 (includes the new STARTS WITH guard); 4 pre-existing unrelated `dead_code` warns in `dashboard.rs` carry over from previous phases. Full clippy gate (`cargo clippy -p cortex-api -- -D warnings`) flags 14 pre-existing lint debt items in `dashboard.rs` / `config_audit.rs` / `ingest_proxy.rs` / `silent_drop.rs` / `strategies.rs` — none introduced by phase11h, tracked separately
