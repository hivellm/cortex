# Proposal: phase12f_record-cron-run-toctou

Source: `docs/analysis/rework/glm5.1/findings.md` F-008 (HIGH).

## Why

`crates/cortex-storage/src/metadata.rs::record_cron_run` reads the row, mutates the in-memory copy, then UPDATEs. Two concurrent cron supervisors racing on the same job (multi-replica deploy or restart-and-old-tick) can lose a write. The TOCTOU window is small but real and corrupts `failure_streak` / `last_run_at` accounting.

## What Changes

- Replace the read-then-update with a single atomic SQL statement: `UPDATE cron_jobs SET last_run_at = ?, last_status = ?, failure_streak = CASE ... WHERE name = ?`.
- Add an advisory lock per job name via SQLite `BEGIN IMMEDIATE` so concurrent updaters serialise rather than race.
- Add a regression test that drives 4 concurrent threads recording runs on the same job and asserts post-condition invariants.

## Impact

- Affected specs: `docs/specs/19-retention.md` § Cron supervisor.
- Affected code: `crates/cortex-storage/src/metadata.rs`.
- Breaking change: NO. Same SQL surface.
- User benefit: cron bookkeeping correct under concurrency.
