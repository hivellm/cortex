# Proposal: phase9g_sqlite_metadata_reaper

## Why

The metadata DB (`~/.cortex/cortex.db`) collects three classes of rows
that grow without bound today:

1. `bootstrap_jobs` — every successful bootstrap run leaves a row
   forever; in CI loops we see hundreds per week.
2. `sessions` — every Claude Code / agent session creates a row that is
   never aged out. A power user has thousands per year.
3. `classifier_spend` — one row per UTC day, harmless on its own but
   never consolidated; we keep raw daily spend back to day one.

Plus two operational logs that are not in SQLite but have the same
"grows forever" problem and live in `~/.cortex/`:

4. `hook-invocations.log` — 519 KB after 24 hours of light use.
5. `hook-errors.log` — append-only.

The reaper consolidates the rows we don't need at high resolution and
rotates the logs so the operator's home directory is bounded.

## What Changes

1. NEW subcommand `cortex-retention metadata-reap`.
2. **`bootstrap_jobs`**: rows where `status='success' AND finished_at <
   now - 30d` are aggregated into a daily summary
   (`bootstrap_jobs_daily { day, repo_path, runs, total_files, total_chunks }`)
   and the source rows are dropped. `failed` rows are retained.
3. **`sessions`**: rows older than 365 d collapse into
   `sessions_monthly { year_month, tool, repo, count, total_event_count }`;
   originals dropped.
4. **`classifier_spend`**: rows older than 365 d collapse into
   `classifier_spend_monthly { year_month, calls, tokens_in, tokens_out,
   est_usd_cents }`; originals dropped.
5. **Log rotation**: when `hook-invocations.log` or `hook-errors.log`
   exceeds 5 MB or 7 days old, rotate to
   `<name>.YYYY-MM-DD.gz` and start a fresh file. Keep the last 8
   rotations, delete older.
6. `VACUUM` after the deletes if free pages > 25%.
7. Bookkeeping in `retention_sweeps.tier_transitions_json.metadata_reap`.
8. `--time-travel`, `--dry-run`, advisory lock — same shape as 9a–9f.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` §"Metadata store",
  `docs/specs/19-retention.md`.
- Affected code: NEW `crates/cortex-retention/src/metadata_reap.rs`,
  schema additions in `crates/cortex-storage/schemas/sqlite/schema.sql`
  (the three `_daily` / `_monthly` tables), small log-rotator helper
  in `crates/cortex-ops/`.
- Breaking change: NO. New summary tables are additive; queries that
  read raw rows can still read the daily/monthly tables for old data.
- User benefit: bounded metadata DB and bounded operator-side log
  surface; closes the last "grows forever" leak in the local stack.
