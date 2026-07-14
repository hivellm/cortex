//! Phase14h §1.2 — [`SynapWorker`] trait.

use std::time::Duration;

use async_trait::async_trait;

/// Worker-controlled gate on the run loop.
///
/// The embedder / fulltext / graph workers wire their
/// [`crate::admin_health::BackpressureState`] into this gate so
/// the shared runtime parks the loop while downstream backends
/// are saturated. The classifier worker has no backpressure
/// state and leaves the gate at its [`BackpressureGate::Active`]
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressureGate {
    /// Run the loop normally.
    #[default]
    Active,
    /// Park the loop and sleep for the configured idle interval
    /// before re-checking. Bumps the `set_backpressure(true)`
    /// flag on the worker's metrics.
    Paused,
}

/// Trait every Synap-consuming worker implements.
///
/// The four production workers (`embedder`, `fulltext`, `graph`,
/// `classifier`) implement this trait; the shared
/// [`crate::synap_worker::runtime`] drives the loop on top of it.
///
/// Defaulted hooks let a worker opt out of behaviors it does
/// not need without forcing every implementor to spell them out.
#[async_trait]
pub trait SynapWorker: Send + Sync + 'static {
    /// Stable, low-cardinality label for logs + metrics. One per
    /// worker class (`"embedder"` / `"fulltext"` / `"graph"` /
    /// `"classifier"`).
    fn worker_name(&self) -> &'static str;

    /// Spawn this many copies of [`run_forever`] in
    /// [`run_pool`]. Must be `>= 1`.
    fn pool_size(&self) -> usize;

    /// One iteration of the loop. Returns the number of
    /// envelopes handled (used to decide whether the runtime
    /// should idle-sleep before the next poll). A returned
    /// `Err(_)` advances the supervisor's consecutive-error
    /// counter and triggers the back-off sleep.
    async fn run_once(&self) -> anyhow::Result<usize>;

    /// How long the runtime sleeps after an empty
    /// [`run_once`]. Defaults to 100ms.
    fn idle_duration(&self) -> Duration {
        Duration::from_millis(100)
    }

    /// How long the runtime sleeps after a back-pressure pause.
    /// Defaults to 5s (matches the legacy per-worker constants).
    fn backpressure_sleep(&self) -> Duration {
        Duration::from_secs(5)
    }

    /// Returns the current gate before each iteration. Default
    /// is [`BackpressureGate::Active`].
    fn backpressure(&self) -> BackpressureGate {
        BackpressureGate::Active
    }

    /// Maximum consecutive [`run_once`] errors before the
    /// runtime returns an [`crate::synap_worker::runtime::RunError::Supervisor`]
    /// so the bin propagates a non-zero exit and Docker restarts
    /// the container. `0` (default) disables the supervisor and
    /// the loop backs off forever.
    fn max_consume_errors(&self) -> u32 {
        0
    }

    /// Called after a successful [`run_once`]. Default no-op.
    /// Workers stamp jobs-processed counters here.
    fn on_run_once_ok(&self, _handled: usize) {}

    /// Called after a failed [`run_once`]. Default no-op.
    /// `consecutive` is the post-increment failure count the
    /// supervisor is watching — workers mirror it into their
    /// own `*_consecutive` field for the `/healthz` probe.
    fn on_run_once_err(&self, _err: &anyhow::Error, _consecutive: u32) {}

    /// Called after a successful [`run_once`] BEFORE
    /// `on_run_once_ok` so workers can reset their consecutive
    /// counter regardless of whether the batch was empty.
    fn on_run_once_success_reset(&self) {}

    /// Called when the runtime enters the back-pressure pause
    /// branch. Default no-op. Workers set
    /// `metrics.set_backpressure(true)` here.
    fn on_backpressure_pause(&self) {}

    /// Back-off slept after a failed [`run_once`]. Defaults to
    /// 500ms (matches the legacy per-worker constant). Tests
    /// override to keep the loop snappy.
    fn error_backoff(&self) -> Duration {
        Duration::from_millis(500)
    }

    /// Maximum CONTINUOUS back-pressure pause before the runtime
    /// returns [`crate::synap_worker::runtime::RunError::StalledPause`]
    /// so the bin propagates a non-zero exit and Docker restarts the
    /// container fresh (phase28 §1.4 defense in depth: the 2026-06-27
    /// graph-worker stall showed a pause that no success could ever
    /// clear silently parks the loop forever, and the consecutive-error
    /// supervisor never sees it because the pause branch short-circuits
    /// before `run_once`). `Duration::ZERO` disables the check.
    /// Defaults to 10 minutes — an order of magnitude above every
    /// worker's half-open retry window, so it only fires when recovery
    /// probes are themselves not clearing the gauge.
    fn max_backpressure_pause(&self) -> Duration {
        Duration::from_secs(600)
    }
}
