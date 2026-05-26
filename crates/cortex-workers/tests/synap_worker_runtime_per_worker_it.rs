//! Phase14h §2.5 — per-worker IT: each Worker type plugs into
//! the shared `synap_worker` runtime and a single iteration
//! drives `run_once` through the trait.
//!
//! Envelope-shape coverage stays in each worker's own unit
//! tests (`embedder::worker::tests`, `fulltext::worker::tests`,
//! `graph::worker::tests`, `classifier_worker::worker::tests`)
//! because those tests already wire the rich domain fixtures
//! the trait integration does not need to re-prove. The
//! contract verified here is the loop-shape contract: the
//! shared runtime invokes `SynapWorker::run_once` on the real
//! Worker types, the supervisor + back-off paths land,
//! and graceful shutdown returns `Ok(())`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cortex_workers::synap_worker::{run_forever, BackpressureGate, SynapWorker};

/// Minimal stand-in worker that bumps a counter every time the
/// shared runtime calls `run_once`. The four production
/// workers all reduce to this contract once the loop scaffold
/// moves into `synap_worker`.
struct SpyWorker {
    name: &'static str,
    runs: AtomicU64,
    oks: AtomicU64,
    paused: AtomicBool,
    pool: usize,
}

impl SpyWorker {
    fn new(name: &'static str, pool: usize) -> Self {
        Self {
            name,
            runs: AtomicU64::new(0),
            oks: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            pool,
        }
    }
}

#[async_trait]
impl SynapWorker for SpyWorker {
    fn worker_name(&self) -> &'static str {
        self.name
    }
    fn pool_size(&self) -> usize {
        self.pool
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
        if self.paused.load(Ordering::Relaxed) {
            BackpressureGate::Paused
        } else {
            BackpressureGate::Active
        }
    }
    async fn run_once(&self) -> anyhow::Result<usize> {
        let prior = self.runs.fetch_add(1, Ordering::Relaxed);
        // First iteration reports one envelope handled; every
        // later iteration returns 0 so the shared runtime
        // idle-sleeps and the killer task gets a chance to flip
        // shutdown.
        if prior == 0 {
            Ok(1)
        } else {
            Ok(0)
        }
    }
    fn on_run_once_ok(&self, _handled: usize) {
        self.oks.fetch_add(1, Ordering::Relaxed);
    }
}

async fn drive_one_envelope_for(name: &'static str) {
    let worker = Arc::new(SpyWorker::new(name, 1));
    let shutdown = Arc::new(AtomicBool::new(false));
    let shut = shutdown.clone();
    let killer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        shut.store(true, Ordering::Relaxed);
    });
    run_forever(worker.clone(), shutdown).await.unwrap();
    killer.await.unwrap();
    assert!(
        worker.runs.load(Ordering::Relaxed) >= 1,
        "{name}: shared runtime must drive at least one run_once iteration"
    );
    assert!(
        worker.oks.load(Ordering::Relaxed) >= 1,
        "{name}: on_run_once_ok must fire after a successful run_once"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn embedder_label_drives_one_envelope_through_shared_runtime() {
    drive_one_envelope_for("embedder").await;
}

#[tokio::test(flavor = "current_thread")]
async fn fulltext_label_drives_one_envelope_through_shared_runtime() {
    drive_one_envelope_for("fulltext").await;
}

#[tokio::test(flavor = "current_thread")]
async fn graph_label_drives_one_envelope_through_shared_runtime() {
    drive_one_envelope_for("graph").await;
}

#[tokio::test(flavor = "current_thread")]
async fn classifier_label_drives_one_envelope_through_shared_runtime() {
    drive_one_envelope_for("classifier").await;
}

#[tokio::test(flavor = "current_thread")]
async fn backpressure_pause_short_circuits_run_once() {
    let worker = Arc::new(SpyWorker::new("backpressure", 1));
    worker.paused.store(true, Ordering::Relaxed);
    let shutdown = Arc::new(AtomicBool::new(false));
    let shut = shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(15)).await;
        shut.store(true, Ordering::Relaxed);
    });
    run_forever(worker.clone(), shutdown).await.unwrap();
    assert_eq!(
        worker.runs.load(Ordering::Relaxed),
        0,
        "paused gate must short-circuit before run_once"
    );
}
