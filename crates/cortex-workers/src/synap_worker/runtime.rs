//! Phase14h §1.3 — generic [`run_forever`] / [`run_pool`] drivers.
//!
//! These functions own the loop shape every Synap worker used
//! to copy-paste:
//!
//! 1. Check the back-pressure gate. If paused, sleep
//!    [`SynapWorker::backpressure_sleep`] and continue.
//! 2. Call [`SynapWorker::run_once`]. On success, reset the
//!    consecutive-error counter and call `on_run_once_ok`.
//! 3. On error, bump the consecutive counter, call
//!    `on_run_once_err`, sleep the back-off, and trip the
//!    supervisor when the counter reaches
//!    [`SynapWorker::max_consume_errors`].
//! 4. When `run_once` returned `0`, idle-sleep
//!    [`SynapWorker::idle_duration`] before the next poll.
//! 5. Exit cleanly when `shutdown.load(Relaxed)` is `true`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::synap_worker::trait_def::{BackpressureGate, SynapWorker};

/// Error returned by [`run_forever`] / [`run_pool`].
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// Supervisor tripped: consecutive [`SynapWorker::run_once`]
    /// errors hit the worker's threshold. The runtime returns
    /// this so the bin's `main` propagates a non-zero exit and
    /// Docker restarts the container fresh.
    #[error(
        "{worker}: consume loop stuck — {consecutive} consecutive errors (threshold {threshold}); last error: {last}"
    )]
    Supervisor {
        /// Worker name.
        worker: &'static str,
        /// Observed consecutive-error count.
        consecutive: u32,
        /// Configured threshold.
        threshold: u32,
        /// Last error message.
        last: String,
    },

    /// Supervisor tripped: the back-pressure gate stayed
    /// [`BackpressureGate::Paused`] continuously for longer than
    /// [`SynapWorker::max_backpressure_pause`]. Phase28 §1.4 —
    /// a pause the worker cannot clear on its own (the 2026-06-27
    /// graph-worker stall) must surface as a restart request, not
    /// an infinite silent park.
    #[error(
        "{worker}: backpressure paused continuously for {paused_secs}s (threshold {threshold_secs}s); requesting restart"
    )]
    StalledPause {
        /// Worker name.
        worker: &'static str,
        /// How long the gate has been continuously paused.
        paused_secs: u64,
        /// Configured threshold.
        threshold_secs: u64,
    },
}

/// Drive the shared loop for `worker` until `shutdown` flips.
///
/// Returns `Ok(())` on clean shutdown, or
/// [`RunError::Supervisor`] when the supervisor trips.
pub async fn run_forever<W: SynapWorker>(
    worker: Arc<W>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), RunError> {
    tracing::info!(worker = worker.worker_name(), "synap worker started");
    let consecutive = AtomicU32::new(0);
    // Phase28 §1.4 — sustained-pause supervisor. Tracks how long the
    // gate has been CONTINUOUSLY paused; any Active observation resets
    // it. Fires `RunError::StalledPause` past
    // `max_backpressure_pause()` so a pause the worker cannot clear on
    // its own becomes a process restart instead of a silent stall.
    let mut paused_since: Option<std::time::Instant> = None;
    while !shutdown.load(Ordering::Relaxed) {
        if matches!(worker.backpressure(), BackpressureGate::Paused) {
            worker.on_backpressure_pause();
            let started = *paused_since.get_or_insert_with(std::time::Instant::now);
            let cap = worker.max_backpressure_pause();
            if !cap.is_zero() && started.elapsed() >= cap {
                let paused_secs = started.elapsed().as_secs();
                let threshold_secs = cap.as_secs();
                tracing::error!(
                    worker = worker.worker_name(),
                    paused_secs,
                    threshold_secs,
                    "supervisor: backpressure paused past threshold; requesting restart"
                );
                return Err(RunError::StalledPause {
                    worker: worker.worker_name(),
                    paused_secs,
                    threshold_secs,
                });
            }
            tokio::time::sleep(worker.backpressure_sleep()).await;
            continue;
        }
        paused_since = None;
        match worker.run_once().await {
            Ok(handled) => {
                consecutive.store(0, Ordering::Relaxed);
                worker.on_run_once_success_reset();
                worker.on_run_once_ok(handled);
                if handled == 0 {
                    tokio::time::sleep(worker.idle_duration()).await;
                }
            }
            Err(err) => {
                let n = consecutive.fetch_add(1, Ordering::Relaxed) + 1;
                worker.on_run_once_err(&err, n);
                let threshold = worker.max_consume_errors();
                tracing::warn!(
                    worker = worker.worker_name(),
                    error = %err,
                    consecutive = n,
                    threshold,
                    "run_once failed; backing off"
                );
                if threshold > 0 && n >= threshold {
                    tracing::error!(
                        worker = worker.worker_name(),
                        consecutive = n,
                        threshold,
                        last_error = %err,
                        "supervisor: consecutive consume errors hit threshold; requesting restart"
                    );
                    return Err(RunError::Supervisor {
                        worker: worker.worker_name(),
                        consecutive: n,
                        threshold,
                        last: err.to_string(),
                    });
                }
                tokio::time::sleep(worker.error_backoff()).await;
            }
        }
    }
    tracing::info!(worker = worker.worker_name(), "synap worker stopped");
    Ok(())
}

/// Spawn [`SynapWorker::pool_size`] copies of [`run_forever`]
/// onto the current Tokio runtime and join them.
///
/// Every copy shares the inner `Arc<W>`, so per-worker metrics
/// and back-pressure state are naturally pooled. Returns
/// `Ok(())` once every copy has exited; returns the first
/// [`RunError`] observed if any copy tripped the supervisor.
pub async fn run_pool<W: SynapWorker>(
    worker: Arc<W>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), RunError> {
    let count = worker.pool_size().max(1);
    let mut handles = Vec::with_capacity(count);
    for idx in 0..count {
        let this = worker.clone();
        let shut = shutdown.clone();
        handles.push(tokio::spawn(async move {
            tracing::debug!(worker = this.worker_name(), idx, "pool worker starting");
            run_forever(this, shut).await
        }));
    }
    let mut first_err: Option<RunError> = None;
    for h in handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
            Err(join_err) => {
                tracing::warn!(error = %join_err, "worker join failed");
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct CountingWorker {
        runs: AtomicU64,
        backpressure_calls: AtomicU64,
        ok_calls: AtomicU64,
        err_calls: AtomicU64,
        plan: Mutex<Vec<Result<usize, String>>>,
        pause_after: AtomicU64,
        threshold: AtomicU32,
        gate: Mutex<BackpressureGate>,
        /// Phase28 §1.4 — sustained-pause cap in ms; `0` disables
        /// (mirrors the trait's `Duration::ZERO` contract).
        max_pause_ms: AtomicU64,
    }

    impl CountingWorker {
        fn with_plan(plan: Vec<Result<usize, String>>) -> Self {
            Self {
                plan: Mutex::new(plan),
                gate: Mutex::new(BackpressureGate::Active),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl SynapWorker for CountingWorker {
        fn worker_name(&self) -> &'static str {
            "counting-test"
        }
        fn pool_size(&self) -> usize {
            1
        }
        fn idle_duration(&self) -> Duration {
            Duration::from_millis(1)
        }
        fn backpressure_sleep(&self) -> Duration {
            Duration::from_millis(1)
        }
        fn error_backoff(&self) -> Duration {
            Duration::from_millis(1)
        }
        fn backpressure(&self) -> BackpressureGate {
            *self.gate.lock().unwrap()
        }
        fn max_consume_errors(&self) -> u32 {
            self.threshold.load(Ordering::Relaxed)
        }
        async fn run_once(&self) -> anyhow::Result<usize> {
            self.runs.fetch_add(1, Ordering::Relaxed);
            let mut plan = self.plan.lock().unwrap();
            let step = if plan.is_empty() {
                Ok(0)
            } else {
                plan.remove(0)
            };
            drop(plan);
            let pause_after = self.pause_after.load(Ordering::Relaxed);
            if pause_after > 0 && self.runs.load(Ordering::Relaxed) >= pause_after {
                *self.gate.lock().unwrap() = BackpressureGate::Paused;
            }
            match step {
                Ok(n) => Ok(n),
                Err(msg) => Err(anyhow::anyhow!(msg)),
            }
        }
        fn on_run_once_ok(&self, _handled: usize) {
            self.ok_calls.fetch_add(1, Ordering::Relaxed);
        }
        fn on_run_once_err(&self, _err: &anyhow::Error, _consecutive: u32) {
            self.err_calls.fetch_add(1, Ordering::Relaxed);
        }
        fn on_backpressure_pause(&self) {
            self.backpressure_calls.fetch_add(1, Ordering::Relaxed);
        }
        fn max_backpressure_pause(&self) -> Duration {
            Duration::from_millis(self.max_pause_ms.load(Ordering::Relaxed))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn happy_path_runs_until_shutdown() {
        let worker = Arc::new(CountingWorker::with_plan(vec![Ok(3), Ok(0), Ok(2)]));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shut_clone = shutdown.clone();
        let worker_clone = worker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            shut_clone.store(true, Ordering::Relaxed);
        });
        run_forever(worker_clone, shutdown).await.unwrap();
        assert!(worker.runs.load(Ordering::Relaxed) >= 3);
        assert!(worker.ok_calls.load(Ordering::Relaxed) >= 3);
        assert_eq!(worker.err_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transient_errors_reset_after_success() {
        let plan = vec![
            Err("boom".into()),
            Err("boom".into()),
            Ok(1),
            Err("boom".into()),
        ];
        let worker = Arc::new(CountingWorker::with_plan(plan));
        worker.threshold.store(10, Ordering::Relaxed);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shut_clone = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            shut_clone.store(true, Ordering::Relaxed);
        });
        run_forever(worker.clone(), shutdown).await.unwrap();
        assert!(worker.err_calls.load(Ordering::Relaxed) >= 3);
        assert!(worker.ok_calls.load(Ordering::Relaxed) >= 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_trips_after_threshold() {
        let plan = vec![
            Err("a".into()),
            Err("b".into()),
            Err("c".into()),
            Err("d".into()),
        ];
        let worker = Arc::new(CountingWorker::with_plan(plan));
        worker.threshold.store(3, Ordering::Relaxed);
        let shutdown = Arc::new(AtomicBool::new(false));
        let err = run_forever(worker.clone(), shutdown).await.unwrap_err();
        match err {
            RunError::Supervisor {
                worker: name,
                consecutive,
                threshold,
                last,
            } => {
                assert_eq!(name, "counting-test");
                assert_eq!(threshold, 3);
                assert_eq!(consecutive, 3);
                assert_eq!(last, "c");
            }
            other => panic!("expected Supervisor, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backpressure_pause_is_observed() {
        let worker = Arc::new(CountingWorker::with_plan(vec![Ok(1), Ok(0)]));
        worker.pause_after.store(1, Ordering::Relaxed);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shut_clone = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            shut_clone.store(true, Ordering::Relaxed);
        });
        run_forever(worker.clone(), shutdown).await.unwrap();
        assert!(worker.backpressure_calls.load(Ordering::Relaxed) >= 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stalled_pause_supervisor_trips_past_threshold() {
        // Phase28 §1.4 — a gate that stays Paused (the 2026-06-27
        // graph-worker stall shape: nothing can ever disarm it) must
        // surface as RunError::StalledPause instead of parking the
        // loop forever.
        let worker = Arc::new(CountingWorker::with_plan(vec![]));
        *worker.gate.lock().unwrap() = BackpressureGate::Paused;
        worker.max_pause_ms.store(20, Ordering::Relaxed);
        let shutdown = Arc::new(AtomicBool::new(false));
        let err = run_forever(worker.clone(), shutdown).await.unwrap_err();
        match err {
            RunError::StalledPause {
                worker: name,
                paused_secs: _,
                threshold_secs,
            } => {
                assert_eq!(name, "counting-test");
                assert_eq!(threshold_secs, 0, "20ms cap truncates to 0s");
            }
            other => panic!("expected StalledPause, got {other:?}"),
        }
        assert_eq!(
            worker.runs.load(Ordering::Relaxed),
            0,
            "paused loop never reaches run_once"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn active_gate_resets_the_stalled_pause_timer() {
        // The cap fires on CONTINUOUS pause only — an Active
        // observation in between resets the timer, so two pause
        // segments each below the cap never trip even when their sum
        // exceeds it.
        let worker = Arc::new(CountingWorker::with_plan(vec![]));
        *worker.gate.lock().unwrap() = BackpressureGate::Paused;
        worker.max_pause_ms.store(100, Ordering::Relaxed);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shut_clone = shutdown.clone();
        let worker_clone = worker.clone();
        tokio::spawn(async move {
            // First pause segment ~50ms < 100ms cap.
            tokio::time::sleep(Duration::from_millis(50)).await;
            *worker_clone.gate.lock().unwrap() = BackpressureGate::Active;
            // One Active loop iteration resets the timer.
            tokio::time::sleep(Duration::from_millis(10)).await;
            *worker_clone.gate.lock().unwrap() = BackpressureGate::Paused;
            // Second pause segment ~60ms < 100ms cap.
            tokio::time::sleep(Duration::from_millis(60)).await;
            shut_clone.store(true, Ordering::Relaxed);
        });
        run_forever(worker.clone(), shutdown)
            .await
            .expect("no StalledPause: neither continuous segment exceeded the cap");
        assert!(
            worker.runs.load(Ordering::Relaxed) >= 1,
            "the Active window must have reached run_once to reset the timer"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_exits_cleanly_on_first_check() {
        let worker = Arc::new(CountingWorker::with_plan(vec![]));
        let shutdown = Arc::new(AtomicBool::new(true));
        run_forever(worker.clone(), shutdown).await.unwrap();
        assert_eq!(worker.runs.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_pool_spawns_n_copies_and_joins() {
        struct PoolWorker {
            count: AtomicU64,
        }
        #[async_trait]
        impl SynapWorker for PoolWorker {
            fn worker_name(&self) -> &'static str {
                "pool-test"
            }
            fn pool_size(&self) -> usize {
                3
            }
            fn idle_duration(&self) -> Duration {
                Duration::from_millis(1)
            }
            async fn run_once(&self) -> anyhow::Result<usize> {
                self.count.fetch_add(1, Ordering::Relaxed);
                Ok(0)
            }
        }
        let worker = Arc::new(PoolWorker {
            count: AtomicU64::new(0),
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let shut_clone = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            shut_clone.store(true, Ordering::Relaxed);
        });
        run_pool(worker.clone(), shutdown).await.unwrap();
        assert!(worker.count.load(Ordering::Relaxed) >= 3);
    }
}
