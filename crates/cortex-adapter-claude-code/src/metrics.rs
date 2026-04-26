//! `cortex.adapter.*` counters / histograms (spec 10 §Observability).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Adapter-side metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.adapter.events.total{kind}`.
    pub events_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.events.dropped{reason}`.
    pub events_dropped: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.sync.latency_ms{hook}` — histogram observations.
    pub sync_latency_ms: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.adapter.sync.timeouts{hook}`.
    pub sync_timeouts: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.publisher.errors{status}`.
    pub publisher_errors: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.pre_thinking.bundle_bytes` — histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.adapter.laws.blocks{law_id}`.
    pub law_blocks: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.overflow.wal_bytes` — gauge sample.
    pub wal_bytes: AtomicU64,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment `events_total{kind}`.
    pub fn incr_events_total(&self, kind: &str) {
        if let Ok(mut m) = self.events_total.lock() {
            *m.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `events_dropped{reason}`.
    pub fn incr_dropped(&self, reason: &str) {
        if let Ok(mut m) = self.events_dropped.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a sync hook latency in milliseconds.
    pub fn observe_sync_latency(&self, hook: &str, ms: u32) {
        if let Ok(mut m) = self.sync_latency_ms.lock() {
            m.entry(hook.to_string()).or_default().push(ms);
        }
    }
    /// Increment `sync.timeouts{hook}`.
    pub fn incr_sync_timeout(&self, hook: &str) {
        if let Ok(mut m) = self.sync_timeouts.lock() {
            *m.entry(hook.to_string()).or_insert(0) += 1;
        }
    }
    /// Increment `publisher.errors{status}`.
    pub fn incr_publisher_error(&self, status: &str) {
        if let Ok(mut m) = self.publisher_errors.lock() {
            *m.entry(status.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a pre-thinking bundle size in bytes.
    pub fn observe_bundle_bytes(&self, bytes: u32) {
        if let Ok(mut g) = self.bundle_bytes.lock() {
            g.push(bytes);
        }
    }
    /// Increment `laws.blocks{law_id}`.
    pub fn incr_law_block(&self, law_id: &str) {
        if let Ok(mut m) = self.law_blocks.lock() {
            *m.entry(law_id.to_string()).or_insert(0) += 1;
        }
    }
    /// Set the `overflow.wal_bytes` gauge.
    pub fn set_wal_bytes(&self, bytes: u64) {
        self.wal_bytes.store(bytes, Ordering::Relaxed);
    }
    /// Read the WAL gauge.
    pub fn wal_bytes(&self) -> u64 {
        self.wal_bytes.load(Ordering::Relaxed)
    }
}
