## 1. Schema
- [ ] 1.1 Add `cron_jobs(name TEXT PRIMARY KEY, schedule TEXT NOT NULL, command TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, last_run_at TEXT, last_status TEXT, next_run_at TEXT, last_error TEXT)` to `crates/cortex-storage/schemas/sqlite/schema.sql`
- [ ] 1.2 Migration helper applied at process start

## 2. Scheduler loop
- [ ] 2.1 NEW `crates/cortex-ops/src/scheduler.rs`
- [ ] 2.2 `tick()` runs every 30 s: select due rows, spawn child via `tokio::process::Command`, capture stdout/stderr (cap 64 KB each), wait, update row
- [ ] 2.3 Compute `next_run_at` from the `schedule` cron expression (use the `cron` crate, UTC)
- [ ] 2.4 On non-zero exit, set `last_status='failed'`, `last_error=<tail>`; emit `cortex.warnings` if last two runs failed
- [ ] 2.5 Concurrency-safe: per-job in-process semaphore (one execution at a time per `name`) plus the underlying advisory lock from each subcommand

## 3. Defaults
- [ ] 3.1 On daemon start, `INSERT OR IGNORE` the eight default jobs (sweep, rollup, cas_vacuum, pii_enforce, turn_digest, meili_prune, metadata_reap, memory_consolidate=disabled)
- [ ] 3.2 Each default has a UTC schedule and the matching `cortex-retention` command line

## 4. CLI
- [ ] 4.1 `cortex schedule list` — table of (name, schedule, enabled, next_run_at, last_status)
- [ ] 4.2 `cortex schedule show <name>` — full row including stdout/stderr tail of the most recent run
- [ ] 4.3 `cortex schedule enable|disable <name>` — toggles `enabled`
- [ ] 4.4 `cortex schedule set <name> <cron>` — validates the expression, recomputes `next_run_at`
- [ ] 4.5 `cortex schedule run-now <name>` — bypasses the timer, runs immediately (still respects the advisory lock)

## 5. Observability
- [ ] 5.1 Emit `cortex.events.enriched` events with `kind="schedule.run_started"` / `schedule.run_completed`
- [ ] 5.2 Repeated-failure event `kind="schedule.repeated_failure"` is consumed by the dashboard banner from 9i

## 6. Spec / docs
- [ ] 6.1 Add §Scheduler to `docs/specs/19-retention.md`
- [ ] 6.2 Reference from `docs/specs/04-cortex-core.md` (daemon responsibilities)
- [ ] 6.3 Update `bin/cortex.bat --help` text

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
