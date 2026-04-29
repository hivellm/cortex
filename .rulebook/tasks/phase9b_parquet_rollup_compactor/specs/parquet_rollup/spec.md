# Spec: Parquet rollup compactor

## ADDED Requirements

### Requirement: Hourly-to-daily rollup at 90 days

The compactor MUST merge every hourly Parquet file under
`events/year=Y/month=M/day=D/hour=*/` into a single
`events/year=Y/month=M/day=D/raw-daily.parquet` once the day boundary is
older than 90 days from the reference time.

The merged file MUST preserve every record from the source files.

#### Scenario: 91-day-old hourly directory becomes a daily file
Given hourly partitions under `year=2026/month=01/day=01/hour=00..23`
And the reference time is 2026-04-01 (91 days later)
When `cortex-retention rollup --granularity hourly_to_daily` runs
Then `year=2026/month=01/day=01/raw-daily.parquet` MUST exist
And the 24 `hour=*` directories MUST be empty or removed
And the row count of the daily file MUST equal the sum of the source rows.

### Requirement: Daily-to-monthly rollup at 365 days

The compactor MUST merge daily files older than 365 days into one
`events/year=Y/month=M/raw-monthly.parquet` per month.

#### Scenario: 366-day-old daily files merge into a monthly file
Given daily files under `year=2025/month=04/day=01..30/raw-daily.parquet`
And the reference time is 2026-04-30
When `cortex-retention rollup --granularity daily_to_monthly` runs
Then `year=2025/month=04/raw-monthly.parquet` MUST exist
And no `day=*` files older than 365 days MUST remain.

### Requirement: 3-year drop with whitelist

After 1095 days from the reference time, the compactor MUST drop monthly
files except for records where `pii_risk = "low"` OR
`kind ∈ {decision, analysis, law_violation}`. Surviving records MUST be
written to `events/year=Y/month=M/preserved.parquet` before the original
monthly file is removed.

#### Scenario: high-pii records are dropped, decisions are preserved
Given a monthly file containing 100 records (10 decisions, 90 high-pii turns)
When the 3-year drop runs
Then `preserved.parquet` MUST contain exactly the 10 decisions
And the original monthly file MUST be removed.

### Requirement: Atomic compaction

The compactor MUST write to `<dest>.tmp`, fsync, rename to `<dest>`, then
unlink sources. Source files MUST NOT be removed before the destination
file is durable.

#### Scenario: crash between fsync and rename leaves a recoverable state
Given the compactor crashes after `fsync(tmp)` but before `rename`
When the next compactor run starts
Then it MUST detect the orphan `.tmp` and remove it
And it MUST re-attempt the partition compaction from scratch.

### Requirement: Corruption quarantine

Files matching `*.corrupted*`, orphan `*.tmp`, or any file that fails to
open as a valid Parquet MUST be moved to
`events/_quarantine/<relative_path>` with a sibling `.reason` text file.
The query layer MUST skip every path under `_quarantine/`.

#### Scenario: existing .corrupted file gets quarantined on first run
Given `events/year=2026/month=04/day=28/hour=22/raw-00000.parquet.corrupted-1907` exists
When the compactor starts
Then the file MUST be moved to `events/_quarantine/...corrupted-1907`
And a `.reason` sibling MUST exist describing the marker suffix.

### Requirement: Reporting

Each rollup invocation MUST update the corresponding `retention_sweeps`
row with `tier_transitions_json.parquet_rollup =
{ files_in, files_out, bytes_reclaimed, quarantined }`.
