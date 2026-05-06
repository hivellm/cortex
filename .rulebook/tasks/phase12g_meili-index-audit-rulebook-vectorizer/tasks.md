## 1. Audit subcommand
- [ ] 1.1 New `cortex-ops meili audit [--repo <slug>] [--json]` subcommand.
- [ ] 1.2 For each configured index, fetch live `numberOfDocuments` and the matching Synap event-count over the same `(repo, kind)` filter.
- [ ] 1.3 Print one row per index: `{ index, repo, meili_count, synap_count, drift_pct }`. Exit 2 if any drift > 5%.
- [ ] 1.4 Boot-time integration: `cortex-api` calls `audit()` at startup, logs WARN for every drifting index.

## 2. Reindex subcommand
- [ ] 2.1 New `cortex-ops meili reindex --repo <slug> [--from <RFC3339>] [--batch <n>]` subcommand.
- [ ] 2.2 Stream events from Synap via the existing `EventStream`, project each through `fulltext/projection.rs`, batch-write 1k at a time.
- [ ] 2.3 Resume-safe: write a checkpoint to `${CORTEX_HOME}/reindex/<index>.checkpoint` after each successful batch.
- [ ] 2.4 Idempotent: re-running with the same `--from` overwrites docs (Meili `addDocuments` semantics).

## 3. Live verification
- [ ] 3.1 Run audit against the running stack pre-fix; record `cortex-rulebook-*` and `cortex-vectorizer-*` drift baseline in the task PR.
- [ ] 3.2 Run reindex for both repos; rerun audit; drift drops below 5%.
- [ ] 3.3 Sample 5 queries that previously returned 0 hits; confirm non-empty result-set post-reindex.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/06-fulltext.md` § Reindex tooling + `CHANGELOG.md` Added.
- [ ] 4.2 Tests: audit unit (stub Meili + Synap) + reindex IT (in-memory backend, 100 events, 2 batches).
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test -p cortex-workers fulltext -p cortex-cli` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
