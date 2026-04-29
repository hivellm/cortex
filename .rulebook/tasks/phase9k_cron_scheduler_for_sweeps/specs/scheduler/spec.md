# Spec: Cron scheduler for sweeps

## ADDED Requirements

### Requirement: cron_jobs registry

The metadata DB MUST contain a `cron_jobs` table with primary key
`name`, columns `schedule` (5-field cron expression, UTC), `command`,
`enabled` (0/1), `last_run_at`, `last_status`, `next_run_at`,
`last_error`.

On first daemon start the table MUST be seeded with the default eight
retention jobs via `INSERT OR IGNORE`. The `memory_consolidate` job
MUST default to `enabled=0`; every other job MUST default to
`enabled=1`.

### Requirement: Scheduler loop in cortex-ops

The `cortex-ops` daemon MUST run a scheduler tick at most every 30 s.
Each tick MUST select every row where `enabled=1 AND next_run_at <= now`,
spawn the configured command as a child process, capture up to 64 KB of
stdout and stderr, await the exit, and update the row with
`last_run_at`, `last_status`, `last_error`, `next_run_at` (recomputed
from `schedule`).

Two firings of the same `name` MUST NOT execute concurrently. The
advisory lock used by the underlying retention subcommand is
authoritative; the scheduler additionally guards via an in-process
semaphore keyed on `name`.

#### Scenario: due job runs once
Given a `cron_jobs` row with `next_run_at = now - 5s` and `enabled=1`
When the scheduler tick fires
Then the configured command MUST be spawned exactly once
And after exit, `last_run_at` MUST equal the run's start time
And `next_run_at` MUST be advanced by the cron expression.

#### Scenario: disabled job is skipped
Given a row with `enabled=0` and `next_run_at = now - 5s`
When the scheduler tick fires
Then no process MUST be spawned for that job
And `last_run_at` MUST be unchanged.

### Requirement: CLI surface

The `cortex` CLI MUST expose `schedule list`, `schedule show <name>`,
`schedule enable|disable <name>`, `schedule set <name> <cron>`, and
`schedule run-now <name>`.

`schedule set` MUST validate the cron expression before persisting and
MUST recompute `next_run_at` immediately.

`schedule run-now` MUST honor the advisory lock and exit non-zero with
a clear message if another run is already in flight.

#### Scenario: run-now while a sweep is active
Given the daily sweep is currently executing
When the operator runs `cortex schedule run-now retention.sweep`
Then the command MUST exit non-zero with a "lock held" message
And no second sweep MUST start.

### Requirement: Repeated-failure observability

When a job's two most recent runs both have `last_status='failed'`,
the scheduler MUST emit one `cortex.warnings` event with
`kind="schedule.repeated_failure"` containing the job `name`, the
two failure timestamps, and the most recent `last_error`.

The 9i dashboard banner MUST surface this event without further work
on this task.

#### Scenario: third consecutive failure does not double-report
Given a job has already raised `schedule.repeated_failure` after run 5 failed
When run 6 also fails
Then the scheduler MUST NOT emit a second `schedule.repeated_failure`
  for the same name within the same 24-hour window.
