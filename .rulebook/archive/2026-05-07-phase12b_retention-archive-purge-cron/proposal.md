# Proposal: phase12b_retention-archive-purge-cron

Source: `docs/analysis/rework/02-memory-cleanup.md` Achado 1 (P0); `docs/analysis/rework/opus5.7/03-recommendation.md` patch #4.

## Why

Today the only way to purge the Parquet archive is `/v1/admin/forget` per-event. There is no cron path, no `--before <date>` bulk operator command, and no scheduled sweep. Operators in production fall back to `rm -rf` on the archive directory because the supported path is unusably slow at scale. Both 4-doc and opus5.7 flag this as the root cause of "memory cleanup has to be brute force".

## What Changes

- New `cortex-ops retention-archive-purge --before <RFC3339> [--dry-run] [--repo <slug>]` that walks `${CORTEX_HOME}/archive/**/*.parquet`, parses the partition timestamp, and deletes files whose newest event is older than `--before`.
- New `retention.archive_purge` cron job seeded by `seed_defaults` with schedule `0 3 * * *` (default `enabled: true`, retention 365 days).
- Sweep writes one `retention_sweeps` row per invocation (consistent with the bookkeeping shipped in phase11v_retention-daemon-recovery §6).
- `cortex-storage::archive_purge` module exposes `purge_before(now, cutoff, dry_run) -> PurgeReport { files_deleted, bytes_reclaimed, partitions_visited }`.

## Impact

- Affected specs: `docs/specs/19-retention.md` § "Archive purge" (new section).
- Affected code: `crates/cortex-cli/src/bin/cortex-ops.rs`, `crates/cortex-storage/src/archive_purge.rs` (new), `crates/cortex-workers/src/retention/scheduler.rs::seed_defaults`.
- Breaking change: NO. Net-new tooling.
- User benefit: archive purge becomes one safe command + automated cron. `rm -rf` no longer the operator's only option.
