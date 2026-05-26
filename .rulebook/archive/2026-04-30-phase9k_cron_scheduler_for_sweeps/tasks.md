## 1. Schema
- [x] 1.1 Added `cron_jobs(name TEXT PRIMARY KEY, schedule TEXT NOT NULL, command TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, last_run_at TEXT, last_status TEXT, next_run_at TEXT, last_error TEXT, last_stdout TEXT, last_stderr TEXT, failure_streak INTEGER NOT NULL DEFAULT 0, last_warning_at TEXT)` plus a `cron_jobs_due` index to `crates/cortex-storage/schemas/sqlite/schema.sql`
- [x] 1.2 Migration helper `apply_phase9k_schema(conn)` applied on every `MetadataStore::open` and called explicitly from `seed_defaults` / `tick`

## 2. Scheduler loop
- [x] 2.1 NEW `crates/cortex-cli/src/ops/scheduler.rs` (cortex-ops crate consolidated into cortex-cli)
- [x] 2.2 `tick()` selects every due row and runs each via the registered `Runner`. The production `ProcessRunner` shells out via `tokio::process::Command`, captures stdout/stderr at 64 KB each (`STREAM_CAP_BYTES`), awaits, persists the outcome
- [x] 2.3 `next_run_at` is recomputed from `schedule` via the `cron` crate (5/6/7-field expressions accepted; UTC always)
- [x] 2.4 Non-zero exits map to `last_status='failed'` (or `lock_held` for exit 2). The first stderr line is stored as `last_error` and after two consecutive failures the scheduler queues one `RepeatedFailureWarning` (deduped by `last_warning_at` on a 24 h window) for the dashboard banner to consume
- [x] 2.5 Concurrency-safe: per-job in-process `tokio::sync::Semaphore` registry guarantees one execution per `name`; the underlying retention subcommands' advisory lock (`retention_sweeps.status='running'`) remains authoritative — `lock_held` exits surface as a discrete status

## 3. Defaults
- [x] 3.1 `seed_defaults(metadata, now)` `INSERT OR IGNORE`s the eight default jobs (sweep / rollup / cas_vacuum / pii_enforce / turn_digest / meili_prune / metadata_reap / memory_consolidate). `memory_consolidate` defaults to `enabled=0`; the rest are `enabled=1`
- [x] 3.2 Every default carries a UTC cron expression and the matching `cortex-ops` command (consolidated CLI binary; `cortex-retention` is the proposal's name for the same surface)

## 4. CLI
- [x] 4.1 `cortex-ops schedule list` — table of (name, schedule, enabled, next_run_at, last_status); `--json` for machine-readable
- [x] 4.2 `cortex-ops schedule show <name>` — full row including stdout/stderr tail of the most recent run
- [x] 4.3 `cortex-ops schedule enable <name>` / `cortex-ops schedule disable <name>` — toggle `enabled`
- [x] 4.4 `cortex-ops schedule set <name> "<cron>"` — validates via `parse_schedule`, recomputes `next_run_at`
- [x] 4.5 `cortex-ops schedule run-now <name>` — bypasses the timer, exits with code 2 + a `lock_held` message when the underlying advisory lock is held

## 5. Observability
- [x] 5.1 Run lifecycle persists onto the `cron_jobs` row (`last_run_at`, `last_status`, `last_stdout`, `last_stderr`, `next_run_at`); the dashboard's [retention SSE feed](16-dashboard.md) reads the same metadata DB so live tails flow through the existing `kind=retention.*` channel without a new event family
- [x] 5.2 Repeated-failure surface is the `RepeatedFailureWarning { name, recent_failures, last_error }` payload `Scheduler::drain_warnings` returns; the phase9i banner consumes it via the shared metadata DB row

## 6. Spec / docs
- [x] 6.1 Added §"Scheduler (phase9k)" to `docs/specs/19-retention.md`
- [x] 6.2 Updated `docs/specs/04-cortex-core.md` with a §"Daemon side-channels" describing the scheduler's role inside the `cortex-ops` daemon
- [x] 6.3 The clap-derive surface on `cortex-ops` automatically renders `--help` for every new `schedule` subcommand alongside the existing retention surface (the proposal's `bin/cortex.bat` shim is not part of this repo's `bin/` tree, which already standardises on `cortex-up` / `cortex-down` / `cortex-doctor` / `cortex-logs` Bash + PowerShell wrappers)

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
