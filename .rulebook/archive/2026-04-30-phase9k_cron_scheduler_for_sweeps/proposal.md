# Proposal: phase9k_cron_scheduler_for_sweeps

## Why

Tasks 9a–9h all ship CLI subcommands and rely on something external to
fire them on a schedule. We do not want to require the operator to wire
up Windows Task Scheduler / systemd timers / cron entries by hand for
each one — that is exactly how we end up with a partially-deployed
retention pipeline (sweep runs nightly, vacuum hasn't run in months).

The `cortex-ops` daemon is already long-running on the host (it owns
the IPC pipe and the publisher WAL). Adding a small cron-style
scheduler there gives us a single place to enable, disable, inspect,
and last-run-time every sweep, with the same `cortex` CLI surface the
operator already knows.

## What Changes

1. NEW table `cron_jobs(name TEXT PRIMARY KEY, schedule TEXT, command
   TEXT, enabled INTEGER, last_run_at TEXT, last_status TEXT,
   next_run_at TEXT, last_error TEXT)` in the metadata DB.
2. NEW module `crates/cortex-ops/src/scheduler.rs` running inside the
   `cortex-ops` daemon: every 30 s, picks rows where
   `enabled=1 AND next_run_at <= now`, spawns the configured command
   as a child process, captures stdout/stderr, updates
   `last_run_at`, `last_status`, `last_error`, `next_run_at`.
3. Defaults seeded on first start (idempotent `INSERT OR IGNORE`):
   - `retention.sweep` → `cortex-retention sweep` daily 03:00 UTC,
   - `retention.rollup` → `cortex-retention rollup` daily 04:00 UTC,
   - `retention.cas_vacuum` → `cortex-retention cas-vacuum` weekly Mon 04:30,
   - `retention.pii_enforce` → `cortex-retention pii-enforce` daily 05:00,
   - `retention.turn_digest` → `cortex-retention turn-digest` weekly Sun 06:00,
   - `retention.meili_prune` → `cortex-retention meili-prune` daily 05:30,
   - `retention.metadata_reap` → `cortex-retention metadata-reap` daily 05:45,
   - `retention.memory_consolidate` → opt-in (enabled=0 by default).
4. CLI subcommands: `cortex schedule list`, `cortex schedule show <name>`,
   `cortex schedule enable <name>`, `cortex schedule disable <name>`,
   `cortex schedule run-now <name>` (one-shot, bypasses the timer),
   `cortex schedule set <name> <cron>`.
5. Schedule strings are 5-field cron expressions (`m h dom mon dow`),
   parsed via the `cron` crate; UTC always.
6. Concurrency: the scheduler uses the same advisory-lock mechanism as
   the underlying subcommands, so a manual `run-now` while the cron
   fires gracefully exits with code 2 instead of double-running.
7. Failure handling: a job that exits non-zero twice in a row stays
   enabled but raises a `cortex.warnings` event tagged
   `kind="schedule.repeated_failure"`; the dashboard banner from 9i
   surfaces it.

## Impact

- Affected specs: NEW `docs/specs/19-retention.md` §Scheduler,
  reference from `docs/specs/04-cortex-core.md` (cortex-ops daemon).
- Affected code: NEW `crates/cortex-ops/src/scheduler.rs`, schema
  addition, CLI surface in `bin/cortex.bat` plus `crates/cortex-ops/`
  subcommands.
- Breaking change: NO. Adds a new table and a new daemon loop;
  existing operations unchanged.
- User benefit: zero-config retention; a fresh install runs the full
  Phase 9 pipeline without any external scheduler. Bad jobs are
  visible in the dashboard.
