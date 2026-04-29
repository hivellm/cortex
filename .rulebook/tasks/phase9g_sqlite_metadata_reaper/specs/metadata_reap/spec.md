# Spec: SQLite metadata reaper

## ADDED Requirements

### Requirement: Bootstrap-job rollup at 30 days

`cortex-retention metadata-reap` MUST aggregate every
`bootstrap_jobs.status='success'` row whose `finished_at < now - 30d`
into one row per `(day, repo_path)` in `bootstrap_jobs_daily`, then
delete the source rows.

Rows with `status='failed'` MUST be retained for full-detail debugging.

#### Scenario: 31-day-old success row collapses to a daily summary
Given a `bootstrap_jobs` row with `status='success'`, `finished_at = now-31d`,
  `files_processed=120`, `chunks_emitted=2400`
When the reaper runs
Then a row in `bootstrap_jobs_daily(day, repo_path)` MUST exist
And it MUST contribute `runs=1`, `total_files=120`, `total_chunks=2400`
And the source row MUST be removed.

#### Scenario: failed row is preserved
Given a `bootstrap_jobs` row with `status='failed'`, `finished_at = now-90d`
When the reaper runs
Then the row MUST remain in `bootstrap_jobs`.

### Requirement: Session monthly rollup at 365 days

Sessions whose `started_at < now - 365d` MUST collapse into one row per
`(year_month, tool, repo)` in `sessions_monthly` carrying
`count` and `total_event_count`. Source rows MUST be deleted.

#### Scenario: year-old sessions roll up
Given 200 sessions in 2025-04 for tool="claude-code" repo="cortex"
When the reaper runs in 2026-05
Then `sessions_monthly` MUST contain a row for `(2025-04, claude-code, cortex)` with `count=200`
And the 200 source rows MUST be absent from `sessions`.

### Requirement: Classifier spend monthly rollup at 365 days

`classifier_spend` rows older than 365 d MUST collapse into
`classifier_spend_monthly(year_month)` with summed counters; source
rows MUST be deleted.

### Requirement: Hook-log rotation

`~/.cortex/hook-invocations.log` and `~/.cortex/hook-errors.log` MUST
be rotated to `<name>.<YYYY-MM-DD>.gz` when they exceed 5 MB or 7 days
of age (whichever first). Rotation MUST be safe against concurrent
appenders. The 8 most recent rotations per file MUST be retained;
older rotations MUST be removed.

#### Scenario: 6-MB live log triggers rotation
Given `hook-invocations.log` is 6 MB
When the reaper runs
Then a `hook-invocations.<YYYY-MM-DD>.gz` MUST exist
And `hook-invocations.log` MUST exist with size < 1 KB.

### Requirement: Read-side union

Reads that span more than the rolled-window MUST transparently union
raw rows and rolled rows so totals do not drop after a reap.

#### Scenario: dashboard chart spans before and after the rollup boundary
Given the dashboard requests bootstrap counts for the last 60 days
When the request is served
Then the helper MUST union the still-raw rows (last 30 d) with
  `bootstrap_jobs_daily` rows (30–60 d)
And the resulting daily series MUST equal the pre-reap totals.

### Requirement: Idempotence

Re-running the reaper without new aged rows MUST be a no-op (zero
deletions, zero rolled rows added).
