## 1. Schema additions
- [x] 1.1 In `crates/cortex-storage/schemas/sqlite/schema.sql`: add `bootstrap_jobs_daily(day TEXT, repo_path TEXT, runs INT, total_files INT, total_chunks INT, PRIMARY KEY(day, repo_path))`
- [x] 1.2 Add `sessions_monthly(year_month TEXT, tool TEXT, repo TEXT, count INT, total_event_count INT, PRIMARY KEY(year_month, tool, repo))`
- [x] 1.3 Add `classifier_spend_monthly(year_month TEXT PRIMARY KEY, calls INT, tokens_in INT, tokens_out INT, est_usd_cents INT)`
- [x] 1.4 Migration helper `apply_phase9g_schema(conn)` invoked at process start; uses `CREATE TABLE IF NOT EXISTS`

## 2. Reaper runner
- [x] 2.1 NEW `crates/cortex-retention/src/metadata_reap.rs`
- [x] 2.2 `roll_bootstrap_jobs(now, retain_days=30)` aggregates success rows into `bootstrap_jobs_daily`, deletes sources in one tx
- [x] 2.3 `roll_sessions(now, retain_days=365)` aggregates into `sessions_monthly`, deletes sources
- [x] 2.4 `roll_classifier_spend(now, retain_days=365)` aggregates into `classifier_spend_monthly`, deletes sources
- [x] 2.5 Final `VACUUM` decision identical to 9c (free pages > 25%)

## 3. Log rotator
- [x] 3.1 NEW helper `crates/cortex-cli/src/ops/log_rotate.rs` (cortex-ops crate consolidated into cortex-cli)
- [x] 3.2 `rotate_if_needed(path, max_bytes=5_000_000, max_age_days=7)` renames to `<name>.<YYYY-MM-DD>.gz` (gzipped)
- [x] 3.3 Retains the 8 most recent rotations per file; older are unlinked
- [x] 3.4 Wired from `cortex-ops metadata-reap` against `~/.cortex/hook-invocations.log` and `~/.cortex/hook-errors.log`
- [x] 3.5 Race-safe: rotates by renaming first, then opening a fresh file (so writers using `O_APPEND` continue cleanly)

## 4. CLI
- [x] 4.1 `cortex-ops metadata-reap [--time-travel RFC3339] [--dry-run] [--target bootstrap_jobs|sessions|classifier_spend|logs|all] [--metadata-db PATH] [--log-dir PATH] [--json]`
- [x] 4.2 `<home>/.cortex/cortex.toml [retention.metadata]` overrides parsed in `cortex-ops`: `bootstrap_retain_days`, `sessions_retain_days`, `spend_retain_days`, `log_max_bytes`, `log_max_age_days`, `log_keep_rotations`. Missing file or missing keys fall back to spec defaults (30 / 365 / 365 / 5 000 000 / 7 / 8)

## 5. Read-side awareness
- [x] 5.1 Dashboard queries that span >30 d for bootstrap, >365 d for sessions/spend union the raw and the rolled tables via `cortex_storage::union_read_bootstrap_jobs`, `union_read_sessions`, and `union_read_classifier_spend`
- [x] 5.2 Unit test `union_read_returns_identical_totals_before_and_after_rollup` in `crates/cortex-storage/src/metadata.rs` proves the union yields identical totals across the rollup boundary

## 6. Spec / docs
- [x] 6.1 Added §"Metadata reaping" to `docs/specs/19-retention.md`
- [x] 6.2 Updated `docs/specs/02-storage-layout.md` §"Metadata store" with the three new rollup tables

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
