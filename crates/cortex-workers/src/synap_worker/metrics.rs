//! Phase14h §3 — shared worker metrics.
//!
//! Two centralised counters:
//!
//! - `cortex_synap_worker_lag{worker}` — last observed lag
//!   (current_offset - last_acked_offset) per worker. Updated
//!   on every successful [`crate::synap_worker::SynapWorker::run_once`].
//! - `cortex_synap_worker_dead_letter_total{worker, reason}` —
//!   monotonic counter family keyed by the fixed
//!   [`crate::synap_worker::dead_letter::DeadLetterReason`]
//!   taxonomy.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::dead_letter::DeadLetterReason;

/// Process-wide shared metrics registry for all Synap workers.
///
/// One instance per worker — the `worker_name` is baked in so
/// callers do not have to repeat the label per call. The
/// doctor (`cortex-ops doctor synap-workers`) walks the
/// per-worker handles to render the cross-worker table.
#[derive(Debug)]
pub struct WorkerMetrics {
    worker: String,
    lag: AtomicU64,
    dead_letter_total: Mutex<BTreeMap<String, u64>>,
}

impl WorkerMetrics {
    /// Fresh registry for `worker_name`.
    pub fn new(worker_name: impl Into<String>) -> Self {
        Self {
            worker: worker_name.into(),
            lag: AtomicU64::new(0),
            dead_letter_total: Mutex::new(BTreeMap::new()),
        }
    }

    /// The `{worker}` label this registry was built with.
    pub fn worker(&self) -> &str {
        &self.worker
    }

    /// Stamp the most recent lag observation. Lag is
    /// `current_offset - last_acked_offset` and is bounded at
    /// `u64::MAX`; callers pass `0` when the lag cannot be
    /// observed (e.g. the consumer has no offset tracker).
    pub fn set_lag(&self, lag: u64) {
        self.lag.store(lag, Ordering::Relaxed);
    }

    /// Last lag observation.
    pub fn lag(&self) -> u64 {
        self.lag.load(Ordering::Relaxed)
    }

    /// Bump the dead-letter counter for `reason`. Poisoned
    /// mutex is recovered via `into_inner`; this is best-effort
    /// telemetry.
    pub fn incr_dead_letter(&self, reason: DeadLetterReason) {
        let mut guard = match self.dead_letter_total.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *guard.entry(reason.as_str().to_string()).or_insert(0) += 1;
    }

    /// Snapshot of the dead-letter counter family. Returns a
    /// stable-order map keyed by reason label.
    pub fn dead_letter_snapshot(&self) -> BTreeMap<String, u64> {
        match self.dead_letter_total.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// Sum across all reasons. Convenience for the doctor's
    /// single-cell summary.
    pub fn dead_letter_total(&self) -> u64 {
        self.dead_letter_snapshot().values().copied().sum()
    }

    /// Render the Prometheus text format for this registry.
    pub fn render_prom(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let worker = &self.worker;
        out.push_str("# TYPE cortex_synap_worker_lag gauge\n");
        let _ = writeln!(
            out,
            "cortex_synap_worker_lag{{worker=\"{worker}\"}} {}",
            self.lag()
        );
        out.push_str("# TYPE cortex_synap_worker_dead_letter_total counter\n");
        for (reason, n) in self.dead_letter_snapshot() {
            let _ = writeln!(
                out,
                "cortex_synap_worker_dead_letter_total{{worker=\"{worker}\",reason=\"{reason}\"}} {n}"
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_round_trips() {
        let m = WorkerMetrics::new("embedder");
        assert_eq!(m.lag(), 0);
        m.set_lag(42);
        assert_eq!(m.lag(), 42);
        m.set_lag(0);
        assert_eq!(m.lag(), 0);
    }

    #[test]
    fn dead_letter_counter_aggregates_per_reason_and_totals() {
        let m = WorkerMetrics::new("fulltext");
        m.incr_dead_letter(DeadLetterReason::DeserializeFailed);
        m.incr_dead_letter(DeadLetterReason::DeserializeFailed);
        m.incr_dead_letter(DeadLetterReason::PublishFailed);
        let snap = m.dead_letter_snapshot();
        assert_eq!(snap.get("deserialize_failed").copied(), Some(2));
        assert_eq!(snap.get("publish_failed").copied(), Some(1));
        assert_eq!(m.dead_letter_total(), 3);
    }

    #[test]
    fn render_prom_includes_worker_label_and_each_reason() {
        let m = WorkerMetrics::new("graph");
        m.set_lag(7);
        m.incr_dead_letter(DeadLetterReason::PermanentHandlerError);
        let body = m.render_prom();
        assert!(body.contains("cortex_synap_worker_lag{worker=\"graph\"} 7"));
        assert!(body.contains(
            "cortex_synap_worker_dead_letter_total{worker=\"graph\",reason=\"permanent_handler_error\"} 1"
        ));
    }
}
