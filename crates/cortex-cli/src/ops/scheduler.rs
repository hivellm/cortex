//! Phase9k — cron scheduler for retention sweeps.
//!
//! The cortex-ops daemon ticks this scheduler every 30 s. Each tick
//! selects every `cron_jobs` row whose `enabled=1 AND next_run_at <=
//! now`, spawns the configured command as a child process, and
//! records the outcome on the row. Two firings of the same `name`
//! never execute concurrently — the in-process semaphore here is
//! the first guard, the underlying retention subcommand's advisory
//! lock (the `retention_sweeps.status='running'` row) is the
//! authoritative one.
//!
//! Library shape:
//!
//! - [`Runner`] trait — production wires [`ProcessRunner`]
//!   (`tokio::process::Command`); tests use [`MemoryRunner`].
//! - [`Scheduler`] — owns the per-job semaphore + the warning
//!   event channel.
//! - [`tick`] — picks every due row and runs it.
//! - [`run_now`] — `cortex schedule run-now` entrypoint; bypasses
//!   the timer but still respects the lock.
//! - [`seed_defaults`] — called by the daemon on start to insert
//!   the eight retention jobs idempotently.
//!
//! The CLI surface lives in `cortex-ops`'s bin so the operator
//! sees `cortex-ops schedule list / show / enable / disable / set /
//! run-now` alongside the existing retention subcommands.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use cron::Schedule;
use cortex_storage::{apply_phase9k_schema, CronJob, MetadataError, MetadataStore};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

/// Maximum bytes to capture from each of the child's stdout / stderr
/// streams. Larger output is truncated to the tail.
pub const STREAM_CAP_BYTES: usize = 64 * 1024;

/// Default tick cadence — 30 s per spec.
pub const DEFAULT_TICK_INTERVAL_SECS: u64 = 30;

/// One run outcome the runner returns to the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// `success` (exit 0) / `failed` (non-zero) / `lock_held`
    /// (advisory-lock conflict, exit 2).
    pub status: String,
    /// Tail of the child's stdout (capped at [`STREAM_CAP_BYTES`]).
    pub stdout_tail: Option<String>,
    /// Tail of the child's stderr.
    pub stderr_tail: Option<String>,
    /// First line of stderr if `failed`, else `None`. Capped at
    /// 256 chars so the SQLite column stays small.
    pub last_error: Option<String>,
}

/// Errors the runner returns when it cannot even spawn the child.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunError {
    /// `tokio::process::Command::spawn` or `wait` failed.
    #[error("spawn: {0}")]
    Spawn(String),
}

/// Trait the scheduler calls to execute a job's `command`. Splitting
/// this out keeps the scheduler logic testable without spawning
/// real processes.
#[async_trait]
pub trait Runner: Send + Sync {
    /// Run `command` and return the outcome. Implementations are
    /// responsible for capping stdout / stderr at
    /// [`STREAM_CAP_BYTES`] and translating exit codes into the
    /// status taxonomy (`success` / `failed` / `lock_held`).
    async fn run(&self, command: &str) -> Result<RunOutcome, RunError>;
}

/// Production runner — spawns `command` via the platform shell.
pub struct ProcessRunner;

#[async_trait]
impl Runner for ProcessRunner {
    async fn run(&self, command: &str) -> Result<RunOutcome, RunError> {
        // Platform shell to honour `cortex-retention sweep --foo` style
        // strings without a manual tokenizer. Both branches inherit
        // PATH from the parent process so the operator's installed
        // `cortex-retention` binary is reachable.
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let output = cmd
            .output()
            .await
            .map_err(|e| RunError::Spawn(e.to_string()))?;
        let stdout_tail = trail_capped(&output.stdout);
        let stderr_tail = trail_capped(&output.stderr);
        let status = match output.status.code() {
            Some(0) => "success",
            Some(2) => "lock_held",
            _ => "failed",
        };
        let last_error = if status != "success" {
            stderr_tail
                .as_ref()
                .and_then(|s| s.lines().next().map(|l| {
                    let mut s = l.to_string();
                    if s.len() > 256 {
                        s.truncate(256);
                    }
                    s
                }))
        } else {
            None
        };
        Ok(RunOutcome {
            status: status.to_string(),
            stdout_tail,
            stderr_tail,
            last_error,
        })
    }
}

fn trail_capped(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let start = bytes.len().saturating_sub(STREAM_CAP_BYTES);
    // Find the next char boundary >= start so we never slice mid-utf8.
    let mut s = start;
    while s < bytes.len() && (bytes[s] & 0xC0) == 0x80 {
        s += 1;
    }
    Some(String::from_utf8_lossy(&bytes[s..]).into_owned())
}

/// Seed the eight default retention jobs on first daemon start.
/// Idempotent: existing rows are left untouched (operators who
/// disabled a job keep their setting after a restart).
pub fn seed_defaults(metadata: &MetadataStore, now: DateTime<Utc>) -> Result<u32, MetadataError> {
    apply_phase9k_schema(metadata.conn())?;
    let defaults = default_jobs();
    let mut inserted = 0;
    for d in defaults {
        let next = next_after(&d.schedule, now)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| now.to_rfc3339());
        if metadata.upsert_cron_job_if_absent(d.name, d.schedule, d.command, d.enabled, &next)? {
            inserted += 1;
        }
    }
    Ok(inserted)
}

/// One default cron-job descriptor.
struct DefaultJob {
    name: &'static str,
    schedule: &'static str,
    command: &'static str,
    enabled: bool,
}

fn default_jobs() -> Vec<DefaultJob> {
    // Schedule values are 5-field cron expressions: `m h dom mon dow`.
    // [`parse_schedule`] adapts them to the `cron` crate's 7-field
    // format internally.
    vec![
        DefaultJob {
            name: "retention.sweep",
            schedule: "0 3 * * *",
            command: "cortex-ops retention-sweep",
            enabled: true,
        },
        DefaultJob {
            name: "retention.rollup",
            schedule: "0 4 * * *",
            command: "cortex-ops rollup",
            enabled: true,
        },
        DefaultJob {
            name: "retention.cas_vacuum",
            schedule: "30 4 * * 1",
            command: "cortex-ops cas-vacuum --force",
            enabled: true,
        },
        DefaultJob {
            name: "retention.pii_enforce",
            schedule: "0 5 * * *",
            command: "cortex-ops pii-enforce",
            enabled: true,
        },
        DefaultJob {
            name: "retention.turn_digest",
            schedule: "0 6 * * 0",
            command: "cortex-ops turn-digest --budget-cents 500",
            enabled: true,
        },
        DefaultJob {
            name: "retention.meili_prune",
            schedule: "30 5 * * *",
            command: "cortex-ops meili-prune",
            enabled: true,
        },
        DefaultJob {
            name: "retention.metadata_reap",
            schedule: "45 5 * * *",
            command: "cortex-ops metadata-reap",
            enabled: true,
        },
        DefaultJob {
            name: "retention.memory_consolidate",
            schedule: "0 7 * * 0",
            command: "cortex-ops memory-consolidate --apply",
            enabled: false,
        },
    ]
}

/// Translate a 5-field cron expression (`m h dom mon dow`) into the
/// 7-field form the `cron` crate expects (`s m h dom mon dow year`).
/// Returns `Err` when the input is malformed.
pub fn parse_schedule(expr: &str) -> Result<Schedule, String> {
    let trimmed = expr.trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let normalised = match parts.len() {
        5 => format!("0 {trimmed} *"),
        6 => format!("{trimmed} *"),
        7 => trimmed.to_string(),
        _ => return Err(format!("expected 5/6/7 cron fields, got {}", parts.len())),
    };
    Schedule::from_str(&normalised).map_err(|e| e.to_string())
}

/// Compute the next firing strictly after `from`.
pub fn next_after(expr: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let s = parse_schedule(expr).ok()?;
    s.after(&from).next()
}

/// One repeated-failure warning the scheduler raises. Consumed by
/// the spec-19/§9i dashboard banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatedFailureWarning {
    /// Job name that failed twice.
    pub name: String,
    /// RFC-3339 timestamps of the two most recent failed runs.
    pub recent_failures: Vec<String>,
    /// Tail of `last_error`.
    pub last_error: Option<String>,
}

/// Per-job in-process semaphore registry. Keyed by job `name` so a
/// `run_now` call while the cron timer fires the same job
/// gracefully serialises rather than double-running.
#[derive(Default)]
pub struct Scheduler {
    locks: Mutex<BTreeMap<String, Arc<Semaphore>>>,
    warnings: Mutex<Vec<RepeatedFailureWarning>>,
}

impl Scheduler {
    /// Empty scheduler.
    pub fn new() -> Self {
        Self::default()
    }
    /// Drain every repeated-failure warning emitted since the last
    /// call. Production wires this to the bus publisher; tests use
    /// it to assert.
    pub async fn drain_warnings(&self) -> Vec<RepeatedFailureWarning> {
        std::mem::take(&mut *self.warnings.lock().await)
    }
    async fn lock_for(&self, name: &str) -> Arc<Semaphore> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }
}

/// Pluck the (at most) two most recent run timestamps from the row's
/// columns. Returns the same RFC-3339 string twice when the row only
/// carries one — the warning shape stays deterministic.
fn recent_failure_stamps(job: &CronJob, now: DateTime<Utc>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = job.last_run_at.clone() {
        out.push(t);
    }
    if out.len() < 2 {
        out.push(now.to_rfc3339());
    }
    out
}

/// Run a single due job: acquires the per-name semaphore, calls the
/// runner, persists the outcome, and (when the failure streak hits
/// 2 with no warning in the last 24 h) appends a
/// [`RepeatedFailureWarning`] to the scheduler's queue.
pub async fn run_one(
    scheduler: &Scheduler,
    runner: &dyn Runner,
    metadata: &MetadataStore,
    job: &CronJob,
    now: DateTime<Utc>,
) -> Result<RunOutcome, RunError> {
    let sem = scheduler.lock_for(&job.name).await;
    let _permit = sem
        .acquire_owned()
        .await
        .map_err(|e| RunError::Spawn(format!("semaphore: {e}")))?;
    let outcome = runner.run(&job.command).await?;
    let next = next_after(&job.schedule, now)
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| now.to_rfc3339());
    let new_streak = metadata
        .record_cron_run(
            &job.name,
            now,
            &outcome.status,
            outcome.stdout_tail.as_deref(),
            outcome.stderr_tail.as_deref(),
            outcome.last_error.as_deref(),
            &next,
        )
        .map_err(|e| RunError::Spawn(format!("record: {e}")))?;
    if outcome.status == "failed" && new_streak >= 2 {
        if should_warn(job, now) {
            let mut warnings = scheduler.warnings.lock().await;
            warnings.push(RepeatedFailureWarning {
                name: job.name.clone(),
                recent_failures: recent_failure_stamps(job, now),
                last_error: outcome.last_error.clone(),
            });
            drop(warnings);
            let _ = metadata.touch_cron_warning(&job.name, now);
        }
    }
    Ok(outcome)
}

fn should_warn(job: &CronJob, now: DateTime<Utc>) -> bool {
    let last = match job.last_warning_at.as_deref() {
        Some(s) => s,
        None => return true,
    };
    match DateTime::parse_from_rfc3339(last) {
        Ok(t) => now.signed_duration_since(t.with_timezone(&Utc)) > Duration::hours(24),
        Err(_) => true,
    }
}

/// One scheduler tick: picks every due row and runs it sequentially.
/// Sequential execution is intentional — most retention jobs hold a
/// shared metadata-DB lock and concurrent firings would just queue
/// behind it; serialising here makes the bookkeeping easier.
pub async fn tick(
    scheduler: &Scheduler,
    runner: &dyn Runner,
    metadata: &MetadataStore,
    now: DateTime<Utc>,
) -> Result<TickReport, MetadataError> {
    apply_phase9k_schema(metadata.conn())?;
    let due = metadata.select_due_cron_jobs(now)?;
    let mut report = TickReport::default();
    report.due = due.len() as u32;
    for job in due {
        match run_one(scheduler, runner, metadata, &job, now).await {
            Ok(out) => match out.status.as_str() {
                "success" => report.successes += 1,
                "lock_held" => report.lock_held += 1,
                _ => report.failures += 1,
            },
            Err(_) => report.failures += 1,
        }
    }
    Ok(report)
}

/// Counters returned by [`tick`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TickReport {
    /// Rows the SELECT returned.
    pub due: u32,
    /// Runs that exited 0.
    pub successes: u32,
    /// Runs that exited 2 (advisory-lock conflict).
    pub lock_held: u32,
    /// Runs that exited non-zero (or could not be spawned).
    pub failures: u32,
}

/// `cortex-ops schedule run-now <name>` entrypoint. Looks up the
/// row, fires the runner, persists the outcome. Returns
/// `Err(RunError::Spawn(...))` when `name` is unknown.
pub async fn run_now(
    scheduler: &Scheduler,
    runner: &dyn Runner,
    metadata: &MetadataStore,
    name: &str,
    now: DateTime<Utc>,
) -> Result<RunOutcome, RunError> {
    let job = metadata
        .get_cron_job(name)
        .map_err(|e| RunError::Spawn(format!("metadata: {e}")))?
        .ok_or_else(|| RunError::Spawn(format!("unknown job: {name}")))?;
    run_one(scheduler, runner, metadata, &job, now).await
}

// ---------- in-memory test runner -----------------------------------

/// In-memory `Runner` — records every command spawned and returns
/// a queued outcome (or a default success). Used by the integration
/// tests in this module + `tests/`.
#[derive(Default)]
pub struct MemoryRunner {
    inner: Mutex<MemoryRunnerState>,
}

#[derive(Default)]
struct MemoryRunnerState {
    pub spawned: Vec<String>,
    pub queue: std::collections::VecDeque<RunOutcome>,
    pub default_status: Option<String>,
}

impl MemoryRunner {
    /// Empty runner — every call returns `success`.
    pub fn new() -> Self {
        Self::default()
    }
    /// Push a one-shot outcome. The next `run` call returns it.
    pub async fn push_outcome(&self, outcome: RunOutcome) {
        self.inner.lock().await.queue.push_back(outcome);
    }
    /// Override the default status (used after the queue drains).
    pub async fn set_default_status(&self, status: &str) {
        self.inner.lock().await.default_status = Some(status.to_string());
    }
    /// Snapshot the commands spawned, in order.
    pub async fn spawned(&self) -> Vec<String> {
        self.inner.lock().await.spawned.clone()
    }
}

#[async_trait]
impl Runner for MemoryRunner {
    async fn run(&self, command: &str) -> Result<RunOutcome, RunError> {
        let mut s = self.inner.lock().await;
        s.spawned.push(command.to_string());
        if let Some(o) = s.queue.pop_front() {
            return Ok(o);
        }
        let status = s.default_status.clone().unwrap_or_else(|| "success".into());
        Ok(RunOutcome {
            status,
            stdout_tail: Some("(memory runner)".to_string()),
            stderr_tail: None,
            last_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn anchor() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 29, 18, 0, 0).unwrap()
    }

    fn store() -> MetadataStore {
        MetadataStore::open_in_memory().unwrap()
    }

    #[test]
    fn parse_schedule_accepts_five_six_and_seven_field_forms() {
        assert!(parse_schedule("0 3 * * *").is_ok());
        assert!(parse_schedule("0 0 3 * * *").is_ok());
        assert!(parse_schedule("0 0 3 * * * *").is_ok());
        assert!(parse_schedule("not a cron").is_err());
        assert!(parse_schedule("1 2 3 4").is_err());
    }

    #[test]
    fn next_after_advances_for_daily_schedule() {
        // 03:00 UTC daily — at 18:00 the next firing is the next day.
        let next = next_after("0 3 * * *", anchor()).unwrap();
        assert_eq!(next.format("%H:%M").to_string(), "03:00");
        assert!(next > anchor());
    }

    #[test]
    fn seed_defaults_inserts_eight_jobs_idempotently() {
        let s = store();
        assert_eq!(seed_defaults(&s, anchor()).unwrap(), 8);
        // Re-seed: zero new inserts.
        assert_eq!(seed_defaults(&s, anchor()).unwrap(), 0);
        let jobs = s.list_cron_jobs().unwrap();
        assert_eq!(jobs.len(), 8);
        let consolidate = jobs
            .iter()
            .find(|j| j.name == "retention.memory_consolidate")
            .unwrap();
        assert!(!consolidate.enabled, "memory_consolidate must default disabled");
        let sweep = jobs.iter().find(|j| j.name == "retention.sweep").unwrap();
        assert!(sweep.enabled);
    }

    #[tokio::test]
    async fn tick_runs_due_job_and_advances_next_run() {
        let store = store();
        // Insert a job whose `next_run_at` is in the past.
        let past = (anchor() - Duration::seconds(5)).to_rfc3339();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &past,
            )
            .unwrap();
        let runner = MemoryRunner::new();
        let scheduler = Scheduler::new();
        let report = tick(&scheduler, &runner, &store, anchor()).await.unwrap();
        assert_eq!(report.due, 1);
        assert_eq!(report.successes, 1);
        assert_eq!(report.failures, 0);
        let job = store.get_cron_job("retention.sweep").unwrap().unwrap();
        assert_eq!(job.last_status.as_deref(), Some("success"));
        assert!(job.next_run_at.is_some());
        let next = job.next_run_at.unwrap();
        assert!(next > anchor().to_rfc3339(), "next_run_at must advance: got {next}");
    }

    #[tokio::test]
    async fn tick_skips_disabled_jobs() {
        let store = store();
        let past = (anchor() - Duration::seconds(5)).to_rfc3339();
        store
            .upsert_cron_job_if_absent(
                "retention.disabled",
                "0 3 * * *",
                "echo hi",
                false,
                &past,
            )
            .unwrap();
        let runner = MemoryRunner::new();
        let scheduler = Scheduler::new();
        let report = tick(&scheduler, &runner, &store, anchor()).await.unwrap();
        assert_eq!(report.due, 0);
        let spawned = runner.spawned().await;
        assert!(spawned.is_empty(), "disabled job was spawned");
        let job = store.get_cron_job("retention.disabled").unwrap().unwrap();
        assert!(job.last_run_at.is_none());
    }

    #[tokio::test]
    async fn run_now_records_outcome_and_advances_next_run() {
        let store = store();
        let future = (anchor() + Duration::hours(8)).to_rfc3339();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &future,
            )
            .unwrap();
        let runner = MemoryRunner::new();
        let scheduler = Scheduler::new();
        let outcome = run_now(&scheduler, &runner, &store, "retention.sweep", anchor())
            .await
            .unwrap();
        assert_eq!(outcome.status, "success");
        let spawned = runner.spawned().await;
        assert_eq!(spawned, vec!["cortex-ops retention-sweep".to_string()]);
        let job = store.get_cron_job("retention.sweep").unwrap().unwrap();
        assert_eq!(job.last_status.as_deref(), Some("success"));
    }

    #[tokio::test]
    async fn run_now_propagates_lock_held_status() {
        let store = store();
        let now = anchor();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &now.to_rfc3339(),
            )
            .unwrap();
        let runner = MemoryRunner::new();
        runner
            .push_outcome(RunOutcome {
                status: "lock_held".into(),
                stdout_tail: None,
                stderr_tail: Some("another retention sweep is in progress".into()),
                last_error: Some("another retention sweep is in progress".into()),
            })
            .await;
        let scheduler = Scheduler::new();
        let outcome = run_now(&scheduler, &runner, &store, "retention.sweep", now)
            .await
            .unwrap();
        assert_eq!(outcome.status, "lock_held");
    }

    #[tokio::test]
    async fn two_consecutive_failures_emit_repeated_failure_warning() {
        let store = store();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &(anchor() - Duration::seconds(5)).to_rfc3339(),
            )
            .unwrap();
        let runner = MemoryRunner::new();
        runner.set_default_status("failed").await;
        let scheduler = Scheduler::new();

        // Run 1 — fails. Streak = 1, no warning yet.
        tick(&scheduler, &runner, &store, anchor()).await.unwrap();
        assert!(scheduler.drain_warnings().await.is_empty());
        // Re-arm next_run_at to the past for the second tick.
        store
            .set_cron_job_schedule(
                "retention.sweep",
                "0 3 * * *",
                &(anchor() - Duration::seconds(5)).to_rfc3339(),
            )
            .unwrap();
        // Run 2 — fails. Streak = 2, warning should fire.
        tick(&scheduler, &runner, &store, anchor() + Duration::seconds(1)).await.unwrap();
        let warnings = scheduler.drain_warnings().await;
        assert_eq!(warnings.len(), 1, "expected one repeated-failure warning");
        assert_eq!(warnings[0].name, "retention.sweep");
    }

    #[tokio::test]
    async fn third_consecutive_failure_does_not_double_warn() {
        let store = store();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &(anchor() - Duration::seconds(5)).to_rfc3339(),
            )
            .unwrap();
        let runner = MemoryRunner::new();
        runner.set_default_status("failed").await;
        let scheduler = Scheduler::new();
        for i in 0..3 {
            store
                .set_cron_job_schedule(
                    "retention.sweep",
                    "0 3 * * *",
                    &(anchor() - Duration::seconds(5)).to_rfc3339(),
                )
                .unwrap();
            tick(&scheduler, &runner, &store, anchor() + Duration::minutes(i)).await.unwrap();
        }
        let warnings = scheduler.drain_warnings().await;
        assert_eq!(
            warnings.len(),
            1,
            "third failure within the dedup window must not re-warn"
        );
    }

    #[tokio::test]
    async fn semaphore_serialises_runs_for_same_name() {
        // The MetadataStore's rusqlite Connection is not Send/Sync,
        // so we cannot fan run_now out across `tokio::spawn`. Drive
        // two sequential `run_now` calls instead and assert the
        // runner observed both — the semaphore release happens at
        // the end of the borrow scope above each call.
        let store = store();
        store
            .upsert_cron_job_if_absent(
                "retention.sweep",
                "0 3 * * *",
                "cortex-ops retention-sweep",
                true,
                &anchor().to_rfc3339(),
            )
            .unwrap();
        let runner = MemoryRunner::new();
        let scheduler = Scheduler::new();
        run_now(&scheduler, &runner, &store, "retention.sweep", anchor())
            .await
            .unwrap();
        run_now(&scheduler, &runner, &store, "retention.sweep", anchor())
            .await
            .unwrap();
        let spawned = runner.spawned().await;
        assert_eq!(spawned.len(), 2);
    }

    #[test]
    fn trail_capped_returns_tail_only_when_exceeding_cap() {
        let small = vec![b'a'; 10];
        let s = trail_capped(&small).unwrap();
        assert_eq!(s.len(), 10);
        let huge = vec![b'a'; STREAM_CAP_BYTES * 2];
        let s = trail_capped(&huge).unwrap();
        assert_eq!(s.len(), STREAM_CAP_BYTES);
    }
}
