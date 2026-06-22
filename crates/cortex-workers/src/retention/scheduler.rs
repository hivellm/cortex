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
use cortex_storage::{apply_phase9k_schema, CronJob, MetadataError, MetadataStore};
use cron::Schedule;
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
        // 2026-05-19 — the cron rows reference sibling bins by bare
        // name (`cortex-consolidator nightly`, `cortex-ops X`, …).
        // When the daemon is started from a release / debug target
        // dir that is NOT on the operator's PATH (the common dev
        // layout), `cmd /C cortex-consolidator …` fails with
        // "command not recognised" before any sweep bookkeeping
        // runs — the cron row records `failed` but no
        // `retention_sweeps` row exists. Prepend the daemon's own
        // bin directory so every sibling bin shipped alongside
        // `cortex-ops` resolves regardless of PATH layout.
        if let Some(path) = sibling_bin_path() {
            cmd.env("PATH", path);
        }
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
            stderr_tail.as_ref().and_then(|s| {
                s.lines().next().map(|l| {
                    let mut s = l.to_string();
                    if s.len() > 256 {
                        s.truncate(256);
                    }
                    s
                })
            })
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

/// Resolve a PATH value with the running daemon's bin directory
/// prepended to whatever PATH the parent already has. Returns `None`
/// when `current_exe()` is unavailable; callers then leave PATH
/// untouched.
///
/// The prepend (not append) is intentional: when an operator both
/// installs `cortex-ops` system-wide AND runs a freshly-built copy
/// out of `target/release`, the running copy's siblings must win —
/// otherwise the daemon would mix bins across versions.
fn sibling_bin_path() -> Option<std::ffi::OsString> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let separator = if cfg!(windows) { ";" } else { ":" };
    let mut out = std::ffi::OsString::from(dir);
    if let Some(existing) = std::env::var_os("PATH") {
        if !existing.is_empty() {
            out.push(separator);
            out.push(existing);
        }
    }
    Some(out)
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

/// Seed the ten default retention jobs on first daemon start, then
/// reconcile drift on existing rows.
///
/// Two passes:
///
/// 1. **Insert** new rows (`upsert_cron_job_if_absent`). Returns the
///    new-row count, the same value the historical contract reports.
/// 2. **Reconcile drift** (phase11v §3.1). When the default for a
///    job's `enabled` flag flips from `false` → `true` after a row
///    has already been seeded with the old default, the existing
///    row stays at the old value forever — `INSERT OR IGNORE` never
///    revisits it. This pass detects that drift and updates
///    `enabled` + `command` to the new defaults, leaving every other
///    column (operator-tuned `schedule`, `last_run_at`,
///    `next_run_at`, `failure_streak`, …) untouched.
///
///    Operator-disabled rows (rows whose `last_warning_at IS NOT
///    NULL` or `failure_streak > 0`) are NOT reconciled — those
///    signals indicate the operator deliberately stopped the job
///    and we must not silently re-enable it.
pub fn seed_defaults(metadata: &MetadataStore, now: DateTime<Utc>) -> Result<u32, MetadataError> {
    apply_phase9k_schema(metadata.conn())?;
    let defaults = default_jobs();
    let mut inserted = 0;
    for d in &defaults {
        let next = next_after(d.schedule, now)
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| now.to_rfc3339());
        if metadata.upsert_cron_job_if_absent(d.name, d.schedule, d.command, d.enabled, &next)? {
            inserted += 1;
        }
    }
    // Pass 2 — reconcile drift on rows that already existed.
    reconcile_default_drift(metadata, &defaults)?;
    Ok(inserted)
}

/// phase11v §3.1 — walk the existing rows and update `enabled` /
/// `command` to the current default when they diverge AND the
/// operator has not deliberately changed them. Heuristic for
/// "operator deliberately changed":
///
/// - `failure_streak > 0` — operator may have disabled the job
///   while debugging a flapping sweep; do not re-enable.
/// - `last_warning_at IS NOT NULL` — same signal at the warning
///   level; do not re-enable.
///
/// `schedule` is never reconciled because operators tune cadences
/// in production and a default-overwrite here would silently
/// rewrite their downtime windows.
fn reconcile_default_drift(
    metadata: &MetadataStore,
    defaults: &[DefaultJob],
) -> Result<(), MetadataError> {
    let existing = metadata.list_cron_jobs()?;
    let by_name: BTreeMap<&str, &CronJob> = existing.iter().map(|j| (j.name.as_str(), j)).collect();
    for d in defaults {
        let Some(row) = by_name.get(d.name) else {
            continue;
        };
        let operator_disabled = row.failure_streak > 0 || row.last_warning_at.is_some();
        if operator_disabled {
            continue;
        }
        if row.enabled != d.enabled || row.command != d.command {
            tracing::info!(
                name = %d.name,
                old_enabled = row.enabled,
                new_enabled = d.enabled,
                old_command = %row.command,
                new_command = %d.command,
                "seed_defaults: reconciled drift"
            );
            metadata.update_cron_job_default_state(d.name, d.enabled, d.command)?;
        }
    }
    Ok(())
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
        // phase11x — turn-digest production wiring. `--apply` switches
        // from the in-memory preview to the live admin enumerator +
        // cortex-ingestion path; `--purge-originals` clears the
        // source rows once the digest persists. Sunday 06:00 UTC
        // sits 30 min before tool_call_digest (06:30) so the two
        // summarisers do not contend for classifier budget.
        DefaultJob {
            name: "retention.turn_digest",
            schedule: "0 6 * * 0",
            command: "cortex-ops turn-digest --apply --purge-originals --budget-cents 500",
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
        // Phase11p §3.2 — flipped from `enabled: false` to `enabled:
        // true` so the auto-memory consolidator (Claude Code memory
        // dir; phase9h, fully implemented in
        // `crates/cortex-cli/src/ops/memory_consolidate.rs`)
        // actually fires on the nightly slot. The pre-phase11p
        // default kept this job dormant which left the user
        // observing unbounded memory growth despite the
        // implementation having shipped.
        DefaultJob {
            name: "retention.memory_consolidate",
            schedule: "0 7 * * 0",
            command: "cortex-ops memory-consolidate --apply",
            enabled: true,
        },
        // Phase11p §3.1 — nightly envelope consolidator. Sits at
        // 02:00, one hour before `retention.consolidation_prune`
        // (03:00) so the pruner sweeps over fresh consolidation
        // rows. Without this seed the pruner walks an empty
        // `cortex_consolidations` index every night.
        //
        // 2026-05-19 fix: the `cortex-consolidator nightly`
        // subcommand defaults `--dry-run=true` (see the bin's
        // clap definition) so a stray operator invocation never
        // burns budget. The cron seed MUST explicitly disable
        // dry-run, otherwise every nightly tick is a no-op and
        // the dashboard `cortex_consolidations` panel stays at
        // zero forever. Pre-2026-05-19 deployments shipped without
        // the flag; `reconcile_default_drift` advances the row on
        // next daemon boot.
        DefaultJob {
            name: "retention.consolidator_nightly",
            schedule: "0 2 * * *",
            command: "cortex-consolidator nightly --dry-run=false",
            enabled: true,
        },
        // 2026-05-20 — populate `sessions` from the parquet archive.
        // The ingestion router writes envelopes but never inserted a
        // session row, so the 24h enumeration in
        // `cortex-consolidator nightly` returned zero and the daily
        // consolidator produced zero summaries. Hourly cadence keeps
        // freshness tight enough that the 02:00 UTC nightly always
        // sees yesterday's sessions; the upsert is idempotent so a
        // re-run on the same archive does nothing once the row's
        // identity fields are populated.
        DefaultJob {
            name: "retention.sessions_backfill",
            schedule: "0 * * * *",
            command: "cortex-ops sessions-backfill",
            enabled: true,
        },
        // phase11w — Tool-call digest summariser. Buckets old
        // tool_call envelopes by (repo, year_week, tool) and
        // purges originals after the digest persists. Sits at
        // 06:30 UTC, 30 min after `turn_digest` (06:00) so the
        // two summarisers do not contend for classifier budget on
        // the same tick. Default ON.
        DefaultJob {
            name: "retention.tool_call_digest",
            schedule: "30 6 * * 0",
            command: "cortex-ops tool-call-digest --apply --purge-originals --budget-cents 500",
            enabled: true,
        },
        // Phase11o §2.5 — nightly tier demotion of consolidations.
        // Walks `cortex_consolidations`, demotes vectors between
        // `cortex.consolidation.fp32` → `.pq` → `cortex.cold.binary`
        // per the 0-7d / 7-90d / 90-365d schedule, hard-purges the
        // >365d tail. Default 03:00 to match the spec-19 retention
        // sweep window; operators tune via
        // `[cortex.consolidation] prune_at` in `cortex.toml`, which
        // the bin path translates to a 5-field cron expression
        // before seeding.
        DefaultJob {
            name: "retention.consolidation_prune",
            schedule: "0 3 * * *",
            command: "cortex-ops consolidation-prune",
            enabled: true,
        },
        // Phase12b — bulk Parquet archive purge. Walks
        // `${CORTEX_HOME}/events/**/*.parquet`, deletes every file
        // whose newest envelope is older than the 365-day retention
        // window. Replaces the per-event `/v1/admin/forget` path
        // operators were avoiding by reaching for `rm -rf`. Default
        // 03:15 UTC — 15 minutes after `retention.consolidation_prune`
        // so the consolidation tier is already demoted when the bulk
        // purge runs, and there is no minute-level contention with
        // the other 03:00 sweep. Operators tune cadence + retention
        // via the existing cron-edit surface; the §3 seed_defaults
        // reconciler preserves operator-tuned schedules.
        //
        // The shipped command embeds the 365-day cutoff as the
        // relative shorthand `--before 365d`. The `retention-archive-
        // purge` binary resolves `now - 365d` itself at run time
        // (parse_cutoff: RFC-3339 OR Nd/Nw/Nh duration); the command
        // literal stays static so the reconciler's drift-detection
        // works without false positives every day. NOTE: prior to the
        // duration-shorthand support the binary only accepted RFC-3339
        // and failed this literal with exit 2, which the run loop
        // mislabelled as `lock_held` (phase0 §2.4 fix).
        DefaultJob {
            name: "retention.archive_purge",
            schedule: "15 3 * * *",
            command: "cortex-ops retention-archive-purge --before 365d",
            enabled: true,
        },
        // phase0 §5 — coverage watchdog. Runs every 15 min so a blind
        // archive watcher, a stalled ingest flush, or a sweep /
        // consolidation that stopped running surfaces as a non-zero
        // cron `last_status` (1 warn / 2 critical) instead of failing
        // silently. Cheap (one HTTP probe + two local reads), no Opus
        // spend, no deletion.
        DefaultJob {
            name: "health.watchdog",
            schedule: "*/15 * * * *",
            command: "cortex-ops watchdog --json",
            enabled: true,
        },
    ]
}

/// Translate a 5-field cron expression (`m h dom mon dow`) into the
/// 7-field form the `cron` crate expects (`s m h dom mon dow year`).
/// Returns `Err` when the input is malformed.
///
/// phase11v §4.2 — the `cron` crate (0.15) rejects raw `0` in the
/// day-of-week position. Standard 5-field cron syntax allows
/// `0`–`6` for Sun–Sat (with `7` aliased to Sun on some
/// implementations). We translate the numeric form into the
/// crate's accepted three-letter form so `0 6 * * 0` no longer
/// silently disables `turn_digest` by failing the parse and
/// falling through to the daemon's `next = now` fallback.
pub fn parse_schedule(expr: &str) -> Result<Schedule, String> {
    let trimmed = expr.trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let normalised = match parts.len() {
        5 => {
            let dow = normalise_dow_field(parts[4])?;
            format!(
                "0 {} {} {} {} {} *",
                parts[0], parts[1], parts[2], parts[3], dow
            )
        }
        6 => {
            // 6-field form: `s m h dom mon dow`. DOW is field 5.
            let dow = normalise_dow_field(parts[5])?;
            format!(
                "{} {} {} {} {} {} *",
                parts[0], parts[1], parts[2], parts[3], parts[4], dow
            )
        }
        7 => {
            // 7-field form: `s m h dom mon dow year`. DOW is field 5.
            let dow = normalise_dow_field(parts[5])?;
            format!(
                "{} {} {} {} {} {} {}",
                parts[0], parts[1], parts[2], parts[3], parts[4], dow, parts[6]
            )
        }
        _ => return Err(format!("expected 5/6/7 cron fields, got {}", parts.len())),
    };
    Schedule::from_str(&normalised).map_err(|e| e.to_string())
}

/// Translate the day-of-week field. Accepts `*`, comma lists, ranges,
/// step expressions, and numeric `0`-`7`. `0` and `7` both map to
/// `SUN`; `1`-`6` map to MON-SAT.
fn normalise_dow_field(field: &str) -> Result<String, String> {
    if field == "*" {
        return Ok("*".to_string());
    }
    let mut out = String::new();
    for (i, segment) in field.split(',').enumerate() {
        if i > 0 {
            out.push(',');
        }
        let (range_part, step_part) = match segment.split_once('/') {
            Some((r, s)) => (r, Some(s)),
            None => (segment, None),
        };
        let translated = if let Some((lo, hi)) = range_part.split_once('-') {
            format!("{}-{}", translate_dow_token(lo)?, translate_dow_token(hi)?)
        } else {
            translate_dow_token(range_part)?.to_string()
        };
        out.push_str(&translated);
        if let Some(s) = step_part {
            out.push('/');
            out.push_str(s);
        }
    }
    Ok(out)
}

fn translate_dow_token(tok: &str) -> Result<&str, String> {
    match tok.trim() {
        "*" => Ok("*"),
        "0" | "7" | "SUN" | "Sun" | "sun" => Ok("SUN"),
        "1" | "MON" | "Mon" | "mon" => Ok("MON"),
        "2" | "TUE" | "Tue" | "tue" => Ok("TUE"),
        "3" | "WED" | "Wed" | "wed" => Ok("WED"),
        "4" | "THU" | "Thu" | "thu" => Ok("THU"),
        "5" | "FRI" | "Fri" | "fri" => Ok("FRI"),
        "6" | "SAT" | "Sat" | "sat" => Ok("SAT"),
        other => Err(format!("invalid day-of-week token: {other}")),
    }
}

/// Compute the next firing strictly after `from`.
///
/// phase11v §4.2 — guards against `Schedule::after(&from).next()`
/// returning a slot equal to `from` on schedules whose first
/// matching instant is exactly `from`. We re-iterate while the
/// returned timestamp is `<= from` so the contract — "strictly
/// greater than `from`" — holds for every valid 5-/6-/7-field cron
/// expression. The walk is bounded at 8 steps so a malformed
/// schedule that yields constant timestamps cannot loop forever.
pub fn next_after(expr: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let s = parse_schedule(expr).ok()?;
    let mut iter = s.after(&from);
    for _ in 0..8 {
        let candidate = iter.next()?;
        if candidate > from {
            return Some(candidate);
        }
    }
    None
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
    if outcome.status == "failed" && new_streak >= 2 && should_warn(job, now) {
        let mut warnings = scheduler.warnings.lock().await;
        warnings.push(RepeatedFailureWarning {
            name: job.name.clone(),
            recent_failures: recent_failure_stamps(job, now),
            last_error: outcome.last_error.clone(),
        });
        drop(warnings);
        let _ = metadata.touch_cron_warning(&job.name, now);
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
    let mut report = TickReport {
        due: due.len() as u32,
        ..TickReport::default()
    };
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
    fn sibling_bin_path_prepends_current_exe_dir() {
        let path = sibling_bin_path().expect("current_exe must resolve under cargo test");
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let path_str = path.to_string_lossy();
        let separator = if cfg!(windows) { ';' } else { ':' };
        let first = path_str.split(separator).next().unwrap();
        assert_eq!(
            std::path::Path::new(first),
            exe_dir,
            "sibling_bin_path must prepend the running exe's dir, got `{path_str}`"
        );
        if let Some(existing) = std::env::var_os("PATH") {
            if !existing.is_empty() {
                assert!(
                    path_str.contains(&*existing.to_string_lossy()),
                    "sibling_bin_path must preserve the parent PATH tail"
                );
            }
        }
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

    /// phase11v §4.1 — when `from` lands exactly on a matching
    /// slot, the helper MUST advance past that slot. The previous
    /// implementation could return the slot itself, which made the
    /// daemon re-fire the same job every tick (the `turn_digest`
    /// 30-s loop the user observed in production).
    ///
    /// Note on cron-crate semantics: when "0" is used as the DOW
    /// field of a 5-field expression, our `parse_schedule` adapter
    /// renders it as `"0 0 6 * * 0 *"`. The cron crate maps DOW = 0
    /// → Sunday. We pin a known Sunday in 2026 (May 3) to drive
    /// the regression; the test fails on the pre-phase11v behaviour
    /// where `Schedule::after(&from).next()` could return the same
    /// slot.
    #[test]
    fn next_after_strictly_advances_when_from_equals_a_slot() {
        // Tuesday 03:00 UTC — exactly the slot the daily 03:00 schedule fires on.
        let from = Utc.with_ymd_and_hms(2026, 5, 5, 3, 0, 0).unwrap();
        let next = next_after("0 3 * * *", from).unwrap();
        assert!(
            next > from,
            "next_after must return a strictly-later instant; got {next}"
        );
        // The next 03:00 slot is Wednesday.
        assert_eq!(
            next.format("%Y-%m-%d %H:%M").to_string(),
            "2026-05-06 03:00"
        );
    }

    /// phase11v §4.3 — drive every shipped retention schedule across
    /// 365 daily `now` values; the helper must NEVER return a slot
    /// that is `<= from`. Catches the regression class above for
    /// every cadence the daemon seeds.
    #[test]
    fn next_after_strictly_advances_across_a_year_for_every_default_schedule() {
        let schedules = [
            "0 3 * * *",
            "0 4 * * *",
            "30 4 * * 1",
            "0 5 * * *",
            "30 5 * * *",
            "45 5 * * *",
            "0 6 * * 0",
            "0 7 * * 0",
            "0 2 * * *",
        ];
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        for schedule in schedules {
            for day in 0..365 {
                let from = start + Duration::days(day);
                let next = next_after(schedule, from)
                    .unwrap_or_else(|| panic!("schedule={schedule}  from={from}  yielded None"));
                assert!(
                    next > from,
                    "schedule={schedule}  from={from}  next={next} (must be > from)"
                );
            }
        }
    }

    /// phase11v §3.3 — when the default for `enabled` flips from
    /// `false` to `true` after a row was already seeded, the next
    /// `seed_defaults` call MUST reconcile the existing row.
    #[test]
    fn seed_defaults_reconciles_drift_when_default_flips_to_enabled() {
        let s = store();
        // First seed: pretend the live default is `enabled = false`
        // by INSERTing the row directly with the old value, then
        // calling seed_defaults to drive the reconcile path.
        apply_phase9k_schema(s.conn()).unwrap();
        s.conn()
            .execute(
                "INSERT INTO cron_jobs (name, schedule, command, enabled, next_run_at)
                      VALUES ('retention.memory_consolidate', '0 7 * * 0',
                              'cortex-ops memory-consolidate --apply', 0,
                              '2026-05-10T06:00:00+00:00')",
                [],
            )
            .unwrap();
        seed_defaults(&s, anchor()).unwrap();
        let row = s
            .get_cron_job("retention.memory_consolidate")
            .unwrap()
            .expect("row present");
        assert!(
            row.enabled,
            "phase11v §3.1 — drifted default must reconcile to enabled=1"
        );
    }

    /// phase12c §1.3 — pre-flip `tool_call_digest` rows must
    /// reconcile their `command` to include `--purge-originals` when
    /// the operator has not deliberately edited them.
    #[test]
    fn seed_defaults_reconciles_tool_call_digest_command_drift() {
        let s = store();
        apply_phase9k_schema(s.conn()).unwrap();
        // Pretend the row was seeded with the pre-flip command (the
        // shape that shipped before phase12c — no `--purge-originals`).
        s.conn()
            .execute(
                "INSERT INTO cron_jobs (name, schedule, command, enabled, next_run_at)
                      VALUES ('retention.tool_call_digest', '30 6 * * 0',
                              'cortex-ops tool-call-digest --apply --budget-cents 500', 1,
                              '2026-05-10T06:30:00+00:00')",
                [],
            )
            .unwrap();
        seed_defaults(&s, anchor()).unwrap();
        let row = s
            .get_cron_job("retention.tool_call_digest")
            .unwrap()
            .expect("row present");
        assert!(
            row.command.contains("--purge-originals"),
            "phase12c §1.2 — drifted command must reconcile to include --purge-originals, got `{}`",
            row.command
        );
        assert!(row.enabled, "row must stay enabled");
    }

    /// 2026-05-19 — pre-fix deployments shipped
    /// `retention.consolidator_nightly` with the bare
    /// `cortex-consolidator nightly` command. The bin defaults
    /// `--dry-run=true` so every tick was a no-op and the
    /// dashboard `cortex_consolidations` panel stayed at zero.
    /// The reconciler MUST advance the row to the post-fix
    /// `--dry-run=false` command.
    #[test]
    fn seed_defaults_reconciles_consolidator_nightly_command_drift() {
        let s = store();
        apply_phase9k_schema(s.conn()).unwrap();
        s.conn()
            .execute(
                "INSERT INTO cron_jobs (name, schedule, command, enabled, next_run_at)
                      VALUES ('retention.consolidator_nightly', '0 2 * * *',
                              'cortex-consolidator nightly', 1,
                              '2026-05-20T02:00:00+00:00')",
                [],
            )
            .unwrap();
        seed_defaults(&s, anchor()).unwrap();
        let row = s
            .get_cron_job("retention.consolidator_nightly")
            .unwrap()
            .expect("row present");
        assert!(
            row.command.contains("--dry-run=false"),
            "consolidator_nightly cron MUST carry --dry-run=false, got `{}`",
            row.command
        );
        assert!(row.enabled, "row must stay enabled after reconcile");
    }

    /// phase11v §3.4 — when an operator deliberately disabled a
    /// row (signal: `failure_streak > 0` OR `last_warning_at`
    /// stamped), the reconciler MUST leave the row alone.
    #[test]
    fn seed_defaults_does_not_overwrite_operator_disabled_rows() {
        let s = store();
        apply_phase9k_schema(s.conn()).unwrap();
        // Row exists with operator-disabled signal: failure_streak=2.
        s.conn()
            .execute(
                "INSERT INTO cron_jobs (name, schedule, command, enabled, next_run_at, failure_streak)
                      VALUES ('retention.memory_consolidate', '0 7 * * 0',
                              'cortex-ops memory-consolidate --apply', 0,
                              '2026-05-10T06:00:00+00:00', 2)",
                [],
            )
            .unwrap();
        seed_defaults(&s, anchor()).unwrap();
        let row = s
            .get_cron_job("retention.memory_consolidate")
            .unwrap()
            .expect("row present");
        assert!(
            !row.enabled,
            "operator-disabled row must NOT be re-enabled by seed reconcile"
        );
        assert_eq!(row.failure_streak, 2);
    }

    #[test]
    fn seed_defaults_inserts_ten_jobs_idempotently() {
        let s = store();
        // phase11w — count bumped from 10 → 11 with the addition of
        // `retention.tool_call_digest`.
        // phase12b — 11 → 12 with the addition of `retention.archive_purge`.
        // 2026-05-20 — 12 → 13 with `retention.sessions_backfill`.
        // phase0 §5 — 13 → 14 with `health.watchdog`.
        assert_eq!(seed_defaults(&s, anchor()).unwrap(), 14);
        // Re-seed: zero new inserts.
        assert_eq!(seed_defaults(&s, anchor()).unwrap(), 0);
        let jobs = s.list_cron_jobs().unwrap();
        assert_eq!(jobs.len(), 14);
        let watchdog = jobs
            .iter()
            .find(|j| j.name == "health.watchdog")
            .expect("health.watchdog must seed");
        assert!(watchdog.enabled, "watchdog defaults enabled");
        assert_eq!(watchdog.schedule, "*/15 * * * *");
        assert_eq!(watchdog.command, "cortex-ops watchdog --json");
        let backfill = jobs
            .iter()
            .find(|j| j.name == "retention.sessions_backfill")
            .expect("sessions_backfill must seed");
        assert!(backfill.enabled, "sessions_backfill defaults enabled");
        assert_eq!(backfill.schedule, "0 * * * *");
        assert_eq!(backfill.command, "cortex-ops sessions-backfill");
        let consolidate = jobs
            .iter()
            .find(|j| j.name == "retention.memory_consolidate")
            .unwrap();
        assert!(
            consolidate.enabled,
            "phase11p §3.2 — memory_consolidate defaults enabled"
        );
        let sweep = jobs.iter().find(|j| j.name == "retention.sweep").unwrap();
        assert!(sweep.enabled);
        let prune = jobs
            .iter()
            .find(|j| j.name == "retention.consolidation_prune")
            .expect("phase11o consolidation_prune must seed");
        assert!(prune.enabled, "consolidation_prune defaults enabled");
        assert_eq!(prune.schedule, "0 3 * * *");
        assert_eq!(prune.command, "cortex-ops consolidation-prune");
        let nightly = jobs
            .iter()
            .find(|j| j.name == "retention.consolidator_nightly")
            .expect("phase11p §3.1 — consolidator_nightly must seed");
        assert!(nightly.enabled, "consolidator_nightly defaults enabled");
        assert_eq!(nightly.schedule, "0 2 * * *");
        assert_eq!(
            nightly.command,
            "cortex-consolidator nightly --dry-run=false"
        );
    }

    #[test]
    fn consolidator_nightly_runs_before_consolidation_prune() {
        // Phase11p §3.1 — the prune sweep at 03:00 must observe the
        // consolidator's 02:00 output. Pin the slot ordering so a
        // future schedule edit can't silently invert them.
        let jobs = default_jobs();
        let nightly = jobs
            .iter()
            .find(|j| j.name == "retention.consolidator_nightly")
            .expect("seeded above");
        let prune = jobs
            .iter()
            .find(|j| j.name == "retention.consolidation_prune")
            .expect("seeded above");
        // Both schedules use the same `m h * * *` shape; compare
        // (hour, minute) pairs directly.
        let nightly_hh: u32 = nightly
            .schedule
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let prune_hh: u32 = prune
            .schedule
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            nightly_hh < prune_hh,
            "consolidator_nightly hour ({nightly_hh}) must precede consolidation_prune hour ({prune_hh})"
        );
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
        assert!(
            next > anchor().to_rfc3339(),
            "next_run_at must advance: got {next}"
        );
    }

    #[tokio::test]
    async fn tick_skips_disabled_jobs() {
        let store = store();
        let past = (anchor() - Duration::seconds(5)).to_rfc3339();
        store
            .upsert_cron_job_if_absent("retention.disabled", "0 3 * * *", "echo hi", false, &past)
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
        tick(&scheduler, &runner, &store, anchor() + Duration::seconds(1))
            .await
            .unwrap();
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
            tick(&scheduler, &runner, &store, anchor() + Duration::minutes(i))
                .await
                .unwrap();
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
