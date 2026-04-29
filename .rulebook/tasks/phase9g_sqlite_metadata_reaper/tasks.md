## 1. Schema additions
- [ ] 1.1 In `crates/cortex-storage/schemas/sqlite/schema.sql`: add `bootstrap_jobs_daily(day TEXT, repo_path TEXT, runs INT, total_files INT, total_chunks INT, PRIMARY KEY(day, repo_path))`
- [ ] 1.2 Add `sessions_monthly(year_month TEXT, tool TEXT, repo TEXT, count INT, total_event_count INT, PRIMARY KEY(year_month, tool, repo))`
- [ ] 1.3 Add `classifier_spend_monthly(year_month TEXT PRIMARY KEY, calls INT, tokens_in INT, tokens_out INT, est_usd_cents INT)`
- [ ] 1.4 Migration helper `apply_phase9g_schema(conn)` invoked at process start; uses `CREATE TABLE IF NOT EXISTS`

## 2. Reaper runner
- [ ] 2.1 NEW `crates/cortex-retention/src/metadata_reap.rs`
- [ ] 2.2 `roll_bootstrap_jobs(now, retain_days=30)` aggregates success rows into `bootstrap_jobs_daily`, deletes sources in one tx
- [ ] 2.3 `roll_sessions(now, retain_days=365)` aggregates into `sessions_monthly`, deletes sources
- [ ] 2.4 `roll_classifier_spend(now, retain_days=365)` aggregates into `classifier_spend_monthly`, deletes sources
- [ ] 2.5 Final `VACUUM` decision identical to 9c (free pages > 25%)

## 3. Log rotator
- [ ] 3.1 NEW helper `crates/cortex-ops/src/log_rotate.rs`
- [ ] 3.2 `rotate_if_needed(path, max_bytes=5_000_000, max_age_days=7)` renames to `<name>.<YYYY-MM-DD>.gz` (gzipped)
- [ ] 3.3 Retains the 8 most recent rotations per file; older are unlinked
- [ ] 3.4 Wired from `cortex-retention metadata-reap` against `~/.cortex/hook-invocations.log` and `~/.cortex/hook-errors.log`
- [ ] 3.5 Race-safe: rotates by renaming first, then opening a fresh file (so writers using `O_APPEND` continue cleanly)

## 4. CLI
- [ ] 4.1 `cortex-retention metadata-reap [--time-travel RFC3339] [--dry-run] [--target bootstrap_jobs|sessions|classifier_spend|logs|all]`
- [ ] 4.2 `cortex.toml [retention.metadata]` (`bootstrap_retain_days=30`, `sessions_retain_days=365`, `spend_retain_days=365`, `log_max_bytes=5_000_000`, `log_max_age_days=7`)

## 5. Read-side awareness
- [ ] 5.1 Dashboard queries that span >30 d for bootstrap, >365 d for sessions/spend MUST union the raw and the rolled tables; helper `union_read(table, since, until)` in `cortex-storage`
- [ ] 5.2 Add a unit test that the union returns identical totals before and after the reaper runs

## 6. Spec / docs
- [ ] 6.1 Add §"Metadata reaping" to `docs/specs/19-retention.md`
- [ ] 6.2 Update `docs/specs/02-storage-layout.md` §"Metadata store" with the new tables

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
