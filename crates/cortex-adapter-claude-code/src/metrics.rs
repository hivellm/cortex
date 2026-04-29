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
    /// `cortex.adapter.publisher.rejected{reason}` — events the
    /// ingestion side returned in the 202 response body's `errors`
    /// list. Tracked per error class so silent schema drift surfaces
    /// in the metrics.
    pub publisher_rejected: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.publisher.accepted_total` — envelopes the
    /// ingestion side reported as accepted in the 202 body. Bumps per
    /// successful batch only after we parse the response shape.
    pub publisher_accepted: AtomicU64,
    /// `cortex.adapter.pre_thinking.bundle_bytes` — histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.adapter.laws.blocks{law_id}`.
    pub law_blocks: Mutex<BTreeMap<String, u64>>,
    /// `cortex.adapter.overflow.wal_bytes` — gauge sample.
    pub wal_bytes: AtomicU64,
    /// Phase8a — Unix-epoch ms of the most recent successful publish.
    /// `0` until the first envelope ships. `/healthz` reads this to
    /// detect publisher stall and downgrade to `degraded`.
    pub last_publish_ok_ts_ms: AtomicU64,
    /// Phase8a — current publisher queue depth (in-memory bounded
    /// channel between hook callbacks and the HTTP publisher).
    pub publisher_queue_depth: AtomicU64,
    /// Phase8a — `1` while the IPC pipe (named pipe / Unix socket)
    /// is bound + accepting connections, `0` when the listener has
    /// torn down.
    pub ipc_pipe_alive: AtomicU64,
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
    /// Increment `publisher.rejected{reason}` — bumped per envelope
    /// the ingestion 202 response body lists in `errors[]`.
    pub fn incr_publisher_rejected(&self, reason: &str) {
        if let Ok(mut m) = self.publisher_rejected.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }
    /// Add `n` to the `publisher.accepted_total` counter.
    pub fn add_publisher_accepted(&self, n: u64) {
        self.publisher_accepted.fetch_add(n, Ordering::Relaxed);
    }
    /// Read `publisher.accepted_total`.
    pub fn publisher_accepted(&self) -> u64 {
        self.publisher_accepted.load(Ordering::Relaxed)
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

    /// Phase8a — stamp `last_publish_ok_ts_ms` to the current
    /// Unix-epoch ms. Called from the publisher's success path.
    pub fn record_publish_ok_now(&self) {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.last_publish_ok_ts_ms.store(now_ms, Ordering::Relaxed);
    }
    /// Phase8a — read the most recent successful-publish timestamp.
    /// `0` means the publisher has never succeeded since boot.
    pub fn last_publish_ok_ts_ms(&self) -> u64 {
        self.last_publish_ok_ts_ms.load(Ordering::Relaxed)
    }
    /// Phase8a — set the current queue depth gauge.
    pub fn set_publisher_queue_depth(&self, depth: u64) {
        self.publisher_queue_depth.store(depth, Ordering::Relaxed);
    }
    /// Phase8a — read the queue depth gauge.
    pub fn publisher_queue_depth(&self) -> u64 {
        self.publisher_queue_depth.load(Ordering::Relaxed)
    }
    /// Phase8a — flip the IPC-pipe-alive flag.
    pub fn set_ipc_pipe_alive(&self, alive: bool) {
        self.ipc_pipe_alive
            .store(if alive { 1 } else { 0 }, Ordering::Relaxed);
    }
    /// Phase8a — read the IPC-pipe-alive flag.
    pub fn ipc_pipe_alive(&self) -> bool {
        self.ipc_pipe_alive.load(Ordering::Relaxed) == 1
    }
}
