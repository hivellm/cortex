## 1. Storage layer — purge_before
- [ ] 1.1 New module `crates/cortex-storage/src/archive_purge.rs` exposing `pub fn purge_before(home: &Path, cutoff: DateTime<Utc>, dry_run: bool) -> Result<PurgeReport>`.
- [ ] 1.2 Walk `${home}/archive/**/*.parquet`, parse the `occurred_at` partition (yyyy-MM-dd subdir), drop files whose newest row is `< cutoff`.
- [ ] 1.3 Honour the partial-frame guard from `is_live_partial_frame()` — never delete a file whose tail-frame is incomplete.
- [ ] 1.4 Unit tests: 4 cases (no files, all old, mixed, partial-frame guard).

## 2. CLI wiring
- [ ] 2.1 Add `retention-archive-purge` subcommand to `cortex-ops` with flags `--before <RFC3339>`, `--dry-run`, `--repo <slug>`.
- [ ] 2.2 Print the `PurgeReport` JSON to stdout. Exit 0 on success; exit 2 on a partial failure (per-file delete error).
- [ ] 2.3 Integration test under `crates/cortex-cli/tests/`.

## 3. Cron seeding
- [ ] 3.1 Add `retention.archive_purge` row to `seed_defaults` with schedule `0 3 * * *`, `enabled = true`, command pointing to `cortex-ops retention-archive-purge --before <now - 365d>`.
- [ ] 3.2 Reconcile pass picks up the new row on existing deployments without overwriting operator-tuned cadence.
- [ ] 3.3 Sweep writes `retention_sweeps` row per invocation with `tier_transitions_json` containing the `PurgeReport`.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/19-retention.md` § "Archive purge" + `CHANGELOG.md` `[Unreleased]` Added.
- [ ] 4.2 Tests: §1.4 unit + §2.3 IT + sweep-bookkeeping IT.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test -p cortex-storage archive_purge -p cortex-cli` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
