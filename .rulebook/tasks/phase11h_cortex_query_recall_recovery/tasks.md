## 1. Redeploy stale daemon

- [ ] 1.1 Capture pre-redeploy snapshot: `/healthz`, `/v1/health/coverage`, `docker inspect cortex-api` → `docs/analysis/phase11h-cortex-query-recall/pre-redeploy.json`
- [ ] 1.2 Rebuild `cortex/api:dev` from current HEAD (`docker compose build cortex-api`)
- [ ] 1.3 Restart `cortex-api` container; assert `/healthz.git_sha` matches `git rev-parse HEAD`
- [ ] 1.4 Assert `/healthz.git_dirty == false` on the redeployed build (b75b96c verification)
- [ ] 1.5 Fire `pre_change_context` with `scope.files=["crates/cortex-api/src/main.rs"]`; assert keyword lane returns no `invalid_search_filter` error (a67687b verification)
- [ ] 1.6 Fire `free_search intent` with high-recall query; assert response ≤ 32 KiB and `clipped` field present when results trimmed (dfb9425 verification)
- [ ] 1.7 Index a `.vue` fixture file via the indexer; assert it lands in the `code` family meili index (048132e verification)

## 2. Re-bootstrap missing collections + clean legacy indexes

- [ ] 2.1 Enumerate the 140 missing vectorizer collections from `/v1/health/coverage.backends[vectorizer].missing`
- [ ] 2.2 Run `cortex-bootstrap --repo <slug>` for every slug in `indexed_repos` so each repo × family target exists (creates the empty collection if the corpus is empty)
- [ ] 2.3 Enumerate the 115 missing meili indexes; run the bootstrap path that creates each `cortex-{slug}-{family}` with `settings.v1.json` applied
- [ ] 2.4 Confirm the 7 unexpected legacy meili indexes (`cortex-{family}` w/o repo prefix) are not referenced by any current code path (grep `crates/`); document the finding
- [ ] 2.5 Delete the 7 legacy meili indexes via authenticated `DELETE /indexes/{name}` (operator-side, with explicit user authorization since this is destructive)
- [ ] 2.6 Re-fetch `/v1/health/coverage`; assert `overall_severity == "ok"`, `unexpected_count == 0` for both backends

## 3. Ingest ADRs + laws into the governance / decisions lanes

- [ ] 3.1 Walk `.rulebook/decisions/*.md`; for each ADR, publish the canonical `cortex_core::events::Envelope` carrying a `Decision` payload onto the ingestion bus so meili + vectorizer index it under `cortex-cortex-decisions`
- [ ] 3.2 Verify each ADR is retrievable via `decision_lookup` with a query covering its title (assert at least 1 hit per ADR)
- [ ] 3.3 Walk `AGENTS.override.md` and `.claude/rules/*.md`; extract every `LAW-*` and behavioral rule into governance-lane envelopes targeting `cortex-cortex-governance`
- [ ] 3.4 Verify `law_check` query for "task sequence" returns LAW-CORTEX-001 with severity + rationale
- [ ] 3.5 Wire the ingestion pipeline: file watcher (or bootstrap-time scan) re-publishes ADRs / laws on change so the lanes stay current without manual republishing
- [ ] 3.6 Document the decisions-and-laws ingestion contract in `docs/cortex/governance-ingestion.md`

## 4. Regression tests

- [ ] 4.1 Add `coverage_drift_it.rs` IT in `crates/cortex-api/tests/`: boots the daemon, calls `/v1/health/coverage`, asserts `present_count == expected_count` and `unexpected_count == 0` for every backend; gated `CORTEX_COVERAGE_IT=1`
- [ ] 4.2 Add `intent_smoke_it.rs` IT in `crates/cortex-mcp-server/tests/`: fires one query per intent (`free_search`, `pre_change_context`, `decision_lookup`, `law_check`, `similar_problems`) against seeded fixtures; asserts non-empty `results` and bounded payload (≤ 32 KiB)
- [ ] 4.3 Add `healthz_release_it.rs` IT: asserts `git_sha != "unknown"`, `git_dirty == false`, `build_ts != "unknown"` on release-profile builds
- [ ] 4.4 Add a unit test in `cortex-api` that pins `meili_lane::build_meili_filter` rejects emitting `STARTS WITH` for any input shape (regression guard for a67687b — the existing assertion at meili_lane.rs:1064 catches the literal but not new code paths)
- [ ] 4.5 Wire all three new ITs into `Justfile` / `docker-compose.test.yml` so the redeploy gate fails fast on any of them

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 5.1 Update or create documentation covering the implementation (`docs/cortex/governance-ingestion.md`, `docs/analysis/phase11h-cortex-query-recall/`, CHANGELOG)
- [ ] 5.2 Write tests covering the new behavior (§4 above; coverage ≥ 95% for new modules)
- [ ] 5.3 Run tests and confirm they pass (`cargo check`, `cargo clippy -- -D warnings`, `cargo test --all-features`, IT suite gated by `CORTEX_*_IT=1`)
