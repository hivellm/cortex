# Proposal: phase9b_parquet_rollup_compactor

## Why

Spec 02 §"Event archive (Parquet)" defines a clean rollup contract:
hourly → daily at 90 d, daily → monthly at 365 d, drop monthly at 3 y
unless `pii_risk = "low"` or `kind ∈ {decision, analysis, law_violation}`.

Reality on disk today:
- `~/.cortex/archive/events/year=2026/month=04/day=28/hour=22/` already
  contains 4 parquet files plus 2 `.corrupted*` artifacts in less than
  one hour.
- No compaction has ever run; every hourly file lives forever.
- Corrupted files are not quarantined, so a future reader stumbles on
  them and may abort instead of skipping.

Without rollup the archive grows ~1 GB/repo/month uncompressed and stays
that way. Without quarantine the archive is fragile. This task lands the
compactor and the corruption-handling protocol.

## What Changes

1. NEW subcommand `cortex-retention rollup` (or sibling crate
   `cortex-archive-compactor`) that walks the archive root and:
   - merges hourly Parquet files older than 90 d into one daily file per
     `year=/month=/day=/` prefix,
   - merges daily files older than 365 d into one monthly file per
     `year=/month=/` prefix,
   - deletes monthly files older than 3 y unless the kind/pii whitelist
     applies (re-reads each file, partitions records, writes a slim
     "preserved" file before deletion).
2. Compaction is **read → write tmp → fsync → rename → delete sources**;
   no destructive op happens before the new file is durable.
3. Files matching `*.corrupted*` are moved to
   `events/_quarantine/<original-relative-path>` with a sidecar
   `<file>.reason` describing why; quarantined files are never read by
   the query API.
4. Compactor row in `retention_sweeps` (reuses 9a's table) with
   `tier_transitions_json.parquet_rollup` describing
   `{ files_in, files_out, bytes_reclaimed, quarantined }`.
5. `--time-travel` flag (consistent with 9a) so the tests can drive the
   90/365/1095-day boundaries deterministically.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` (link to 19),
  `docs/specs/19-retention.md` (add rollup section).
- Affected code: NEW `crates/cortex-retention/src/parquet_rollup.rs`
  or sibling crate; reuses `cortex-storage::archive::archive_partition`
  and the existing core Parquet writer.
- Breaking change: NO. Read paths must already tolerate either hourly
  or daily files since spec 02 has always promised this layout.
- User benefit: bounded archive size, automatic quarantine of corrupt
  files (already accumulating), enforces 3-year audit window.
