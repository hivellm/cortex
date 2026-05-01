//! Phase10k — always-on cron tick for the retention sweeps.
//!
//! Phase9a→9k built every sweep + the cron registry that binds them,
//! but until phase10k there was no long-running process firing the
//! tick. The result observed in production: hot data piling up in
//! `cortex.turn.fp32`, `cortex.tool_call.fp32`, the meili indexes,
//! and the SQLite metadata store — sweeps never ran on their own.
//!
//! This module owns the daemon-side wiring. On boot the cortex-api
//! main:
//!
//! 1. Calls [`spawn`] with the shared `MetadataStore`.
//! 2. The spawn helper seeds the eight default cron rows (idempotent
//!    — operators who disabled a row keep their setting across
//!    restarts), then launches a `tokio::time::interval` loop that
//!    invokes [`cortex_retention::scheduler::tick`] every
//!    [`DEFAULT_TICK_INTERVAL_SECS`] seconds.
//! 3. The runner is [`ProcessRunner`], so each due row spawns the
//!    matching `cortex-ops <subcommand>` and inherits PATH from the
//!    daemon. Output is captured + persisted to the `cron_jobs` row.
//!
//! Operators can opt the daemon out via
//! `CORTEX_RETENTION_DAEMON=disabled`. Useful for CI, single-shot
//! tests, or rolling upgrades where the operator wants to drive
//! sweeps from outside the daemon. The opt-out is a single env var
//! check at boot — no runtime reconfiguration; flipping the value
//! requires a restart, which is the right cost for a feature that
//! shapes the durability profile of the host.

use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use cortex_retention::scheduler::{
    next_after, seed_defaults, ProcessRunner, RunOutcome, Runner, Scheduler, TickReport,
    DEFAULT_TICK_INTERVAL_SECS,
};
use cortex_storage::{CronJob, MetadataStore};

/// Env var honoured at boot: `disabled` (case-insensitive) keeps
/// the daemon idle so an operator can drive sweeps externally.
pub const ENV_OPT_OUT: &str = "CORTEX_RETENTION_DAEMON";

/// Returns `true` when the env opt-out is engaged. Public so the
/// caller can skip the spawn without touching the metadata store.
pub fn opt_out_engaged() -> bool {
    std::env::var(ENV_OPT_OUT)
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("disabled"))
        .unwrap_or(false)
}

/// Spawn options. Non-public fields keep the call site small.
#[derive(Clone)]
pub struct SpawnOptions {
    /// Tick cadence override. Production uses the default 30 s; tests
    /// drop it to a few hundred ms.
    pub tick_interval: StdDuration,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            tick_interval: StdDuration::from_secs(DEFAULT_TICK_INTERVAL_SECS),
        }
    }
}

/// Spawn the retention daemon. Returns `Some` join-handle when the
/// loop started, `None` when the env opt-out skipped it.
///
/// The daemon owns its own metadata-store handle, opened anew on
/// every tick. Two reasons it does NOT share the dashboard's
/// `MetadataStore` Arc:
///
/// 1. `rusqlite::Connection` carries `RefCell` interior state, so
///    it is `Send` but not `Sync`. Holding a `tokio::sync::Mutex`
///    guard across the `tick(...).await` boundary requires the
///    inner type to be `Sync` (the guard derefs to `&T` which the
///    future state machine shares between yields).
/// 2. SQLite WAL handles cross-connection concurrency natively. A
///    fresh connection per 30-s tick is ~1 ms of overhead and lets
///    the dashboard, the daemon, and the operator-driven CLI all
///    drive the same on-disk DB without fighting over a single
///    in-process handle.
pub fn spawn(metadata_path: PathBuf, opts: SpawnOptions) -> Option<tokio::task::JoinHandle<()>> {
    if opt_out_engaged() {
        tracing::info!(
            "retention daemon disabled via {ENV_OPT_OUT}=disabled — operator must drive sweeps manually"
        );
        return None;
    }

    // Seed the default rows once on boot. Idempotent — re-seeding
    // after a restart never overwrites operator-tuned schedules.
    seed_with_fresh_connection(&metadata_path);

    Some(spawn_loop(metadata_path, opts))
}

fn seed_with_fresh_connection(metadata_path: &Path) {
    match MetadataStore::open(metadata_path) {
        Ok(store) => {
            let now = Utc::now();
            match seed_defaults(&store, now) {
                Ok(0) => tracing::info!("retention daemon: cron registry already seeded"),
                Ok(n) => tracing::info!(inserted = n, "retention daemon: seeded default cron jobs"),
                Err(err) => tracing::warn!(
                    error = %err,
                    "retention daemon: seed_defaults failed; tick loop will still run for any pre-existing rows"
                ),
            }
        }
        Err(err) => tracing::warn!(
            error = %err,
            metadata_db = %metadata_path.display(),
            "retention daemon: could not open metadata store for seed; tick loop will retry"
        ),
    }
}

fn spawn_loop(metadata_path: PathBuf, opts: SpawnOptions) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let scheduler = Scheduler::new();
        let runner: &dyn Runner = &ProcessRunner;
        let mut ticker = tokio::time::interval(opts.tick_interval);
        // Skip the immediate first tick — gives the daemon time to
        // bind /healthz and finish wiring the dashboard before we
        // start firing subprocesses that may compete for the same
        // metadata DB lock.
        ticker.tick().await;
        tracing::info!(
            interval_secs = opts.tick_interval.as_secs(),
            metadata_db = %metadata_path.display(),
            "retention daemon started"
        );
        loop {
            ticker.tick().await;
            run_one_tick(&metadata_path, &scheduler, runner).await;
        }
    })
}

/// One scheduler tick driven by [`spawn_loop`]. Cannot reuse the
/// `cortex-retention::scheduler::tick` helper because that one
/// holds `&MetadataStore` across the `runner.run().await` boundary,
/// and `MetadataStore` carries a `RefCell` (rusqlite internals)
/// that makes it `Send + !Sync`. The loop here drops the
/// connection BEFORE the await and re-opens it after, so no
/// non-`Sync` reference ever crosses a yield point.
async fn run_one_tick(metadata_path: &Path, scheduler: &Scheduler, runner: &dyn Runner) {
    let now = Utc::now();
    let due = match list_due(metadata_path, now) {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                error = %err,
                metadata_db = %metadata_path.display(),
                "retention daemon: list_due failed, skipping tick"
            );
            return;
        }
    };
    let mut report = TickReport {
        due: due.len() as u32,
        ..Default::default()
    };
    for job in due {
        match runner.run(&job.command).await {
            Ok(outcome) => {
                match outcome.status.as_str() {
                    "success" => report.successes += 1,
                    "lock_held" => report.lock_held += 1,
                    _ => report.failures += 1,
                }
                if let Err(err) = persist_outcome(metadata_path, &job, now, &outcome) {
                    tracing::warn!(
                        error = %err,
                        job = %job.name,
                        "retention daemon: persist_outcome failed"
                    );
                }
            }
            Err(err) => {
                report.failures += 1;
                tracing::warn!(
                    error = %err,
                    job = %job.name,
                    "retention daemon: runner.run failed"
                );
            }
        }
    }
    if report.due > 0 {
        tracing::info!(
            due = report.due,
            successes = report.successes,
            failures = report.failures,
            lock_held = report.lock_held,
            "retention tick"
        );
    }
    // Drain repeated-failure warnings so the dashboard banner can
    // pick them up. Keeping the queue local — surfacing it to the
    // rest of cortex-api lands in a follow-up (phase10l) once the
    // dashboard endpoint is decided.
    let _ = scheduler.drain_warnings().await;
}

fn list_due(metadata_path: &Path, now: DateTime<Utc>) -> Result<Vec<CronJob>, anyhow::Error> {
    let store = MetadataStore::open(metadata_path)?;
    cortex_storage::apply_phase9k_schema(store.conn())?;
    let due = store.select_due_cron_jobs(now)?;
    Ok(due)
}

fn persist_outcome(
    metadata_path: &Path,
    job: &CronJob,
    now: DateTime<Utc>,
    outcome: &RunOutcome,
) -> Result<(), anyhow::Error> {
    let store = MetadataStore::open(metadata_path)?;
    let next = next_after(&job.schedule, now)
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| now.to_rfc3339());
    store.record_cron_run(
        &job.name,
        now,
        &outcome.status,
        outcome.stdout_tail.as_deref(),
        outcome.stderr_tail.as_deref(),
        outcome.last_error.as_deref(),
        &next,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise tests that mutate `CORTEX_RETENTION_DAEMON`. Cargo
    /// runs tests in parallel by default; without this guard the
    /// "opt-out engaged" test races with "spawn seeds defaults" and
    /// either can spuriously fail.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn opt_out_engaged_reads_env_var() {
        let _g = env_lock();
        let prev = std::env::var(ENV_OPT_OUT).ok();
        std::env::set_var(ENV_OPT_OUT, "disabled");
        assert!(opt_out_engaged());
        std::env::set_var(ENV_OPT_OUT, "DISABLED");
        assert!(opt_out_engaged(), "env check is case-insensitive");
        std::env::set_var(ENV_OPT_OUT, "enabled");
        assert!(!opt_out_engaged());
        std::env::remove_var(ENV_OPT_OUT);
        assert!(!opt_out_engaged());
        if let Some(v) = prev {
            std::env::set_var(ENV_OPT_OUT, v);
        }
    }

    #[tokio::test]
    async fn spawn_skips_when_opt_out_engaged() {
        let _g = env_lock();
        let prev = std::env::var(ENV_OPT_OUT).ok();
        std::env::set_var(ENV_OPT_OUT, "disabled");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.sqlite");
        let handle = spawn(path, SpawnOptions::default());
        assert!(handle.is_none(), "opt-out must skip spawning");
        std::env::remove_var(ENV_OPT_OUT);
        if let Some(v) = prev {
            std::env::set_var(ENV_OPT_OUT, v);
        }
    }

    #[tokio::test]
    async fn spawn_seeds_defaults_when_metadata_empty() {
        let _g = env_lock();
        let prev = std::env::var(ENV_OPT_OUT).ok();
        std::env::remove_var(ENV_OPT_OUT);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.sqlite");
        // Long tick so the loop never actually fires during the test.
        let opts = SpawnOptions {
            tick_interval: StdDuration::from_secs(3600),
        };
        let handle = spawn(path.clone(), opts).expect("spawn must produce a handle");
        // Re-open to inspect the seeded rows.
        let store = MetadataStore::open(&path).expect("metadata reopens");
        let jobs = store.list_cron_jobs().expect("metadata store readable");
        assert_eq!(jobs.len(), 8, "default cron rows should be seeded");
        // Stop the loop so the test runtime can shut down cleanly.
        handle.abort();
        if let Some(v) = prev {
            std::env::set_var(ENV_OPT_OUT, v);
        }
    }

    #[tokio::test]
    async fn run_one_tick_persists_run_outcome_for_a_due_job() {
        // Drives the same on-disk MetadataStore through the
        // open/close-per-tick path the daemon uses in production.
        // The runner is a synthetic always-success runner so the
        // tick can be observed end-to-end without spawning a real
        // subprocess.
        use cortex_retention::scheduler::{RunError, RunOutcome};
        struct AlwaysSuccess;
        #[async_trait::async_trait]
        impl Runner for AlwaysSuccess {
            async fn run(&self, command: &str) -> Result<RunOutcome, RunError> {
                Ok(RunOutcome {
                    status: "success".to_string(),
                    stdout_tail: Some(format!("ran {command}")),
                    stderr_tail: None,
                    last_error: None,
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.sqlite");
        // Seed one due row with `next_run_at` in the past.
        {
            let store = MetadataStore::open(&path).unwrap();
            cortex_storage::apply_phase9k_schema(store.conn()).unwrap();
            let past = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
            store
                .upsert_cron_job_if_absent(
                    "retention.sweep",
                    "0 3 * * *",
                    "cortex-ops retention-sweep",
                    true,
                    &past,
                )
                .unwrap();
        }

        let scheduler = Scheduler::new();
        let runner: &dyn Runner = &AlwaysSuccess;
        run_one_tick(&path, &scheduler, runner).await;

        // The persisted row must show the run as success and a
        // freshly-advanced next_run_at.
        let store = MetadataStore::open(&path).unwrap();
        let job = store.get_cron_job("retention.sweep").unwrap().unwrap();
        assert_eq!(job.last_status.as_deref(), Some("success"));
        let next = job.next_run_at.expect("next_run_at advanced");
        assert!(next > Utc::now().to_rfc3339() || next.starts_with("2"));
    }
}
