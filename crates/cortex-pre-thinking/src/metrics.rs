//! `cortex.prethink.*` counters / histograms (spec 12 §Observability).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::budget::TrimStep;

/// Pre-thinking metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.prethink.calls.total{intent}`.
    pub calls_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.bundle.bytes` histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.prethink.sections.count{section}` histogram.
    pub section_count: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.prethink.truncation.applied{step}`.
    pub truncation_applied: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.latency_ms` histogram.
    pub latency_ms: Mutex<Vec<u64>>,
    /// `cortex.prethink.empty_bundle`.
    pub empty_bundle: AtomicU64,
    /// `cortex.prethink.timeouts`.
    pub timeouts: AtomicU64,
    /// Phase14e — `cortex_pre_thinking_fail_open_total{reason}`.
    /// Reasons: `timeout`, `network`, `unauthorised`, `internal`,
    /// `breaker_open`. Read by the doctor + the
    /// `/v1/health/pre-thinking` endpoint to surface outage
    /// counts to the operator.
    pub fail_open_total: Mutex<BTreeMap<String, u64>>,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Increment the call counter for `intent`.
    pub fn incr_calls(&self, intent: &str) {
        if let Ok(mut m) = self.calls_total.lock() {
            *m.entry(intent.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a bundle size observation.
    pub fn observe_bundle_bytes(&self, n: u32) {
        if let Ok(mut g) = self.bundle_bytes.lock() {
            g.push(n);
        }
    }
    /// Record a section-count observation.
    pub fn observe_section_count(&self, section: &str, n: u32) {
        if let Ok(mut m) = self.section_count.lock() {
            m.entry(section.to_string()).or_default().push(n);
        }
    }
    /// Record a truncation step.
    pub fn incr_truncation_step(&self, step: TrimStep) {
        if let Ok(mut m) = self.truncation_applied.lock() {
            *m.entry(format!("{step:?}")).or_insert(0) += 1;
        }
    }
    /// Record a latency observation.
    pub fn observe_latency_ms(&self, ms: u64) {
        if let Ok(mut g) = self.latency_ms.lock() {
            g.push(ms);
        }
    }
    /// Increment the empty-bundle counter.
    pub fn incr_empty_bundle(&self) {
        self.empty_bundle.fetch_add(1, Ordering::Relaxed);
    }
    /// Increment the timeout counter.
    pub fn incr_timeouts(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Phase14e — bump the per-reason fail-open counter. Called
    /// from the pipeline on every fail-open dispatch (real
    /// upstream failure OR breaker-open short-circuit).
    pub fn incr_fail_open(&self, reason: &str) {
        if let Ok(mut m) = self.fail_open_total.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }

    /// Phase14e — snapshot the per-reason fail-open counters.
    /// Used by the doctor + the `/v1/health/pre-thinking`
    /// endpoint.
    pub fn fail_open_snapshot(&self) -> BTreeMap<String, u64> {
        self.fail_open_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }
}
