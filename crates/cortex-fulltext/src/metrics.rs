//! In-process counters / histograms backing the `cortex.fulltext.*`
//! metric family in `docs/specs/08-fulltext-indexer.md` §Observability.
//!
//! Light-weight atomic-counter implementation matching the
//! cortex-graph and cortex-embedder style. Any Prometheus /
//! OpenTelemetry exporter can read these values directly.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Full-text-indexer metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.fulltext.documents.total` — total docs upserted, keyed
    /// by index name.
    pub documents_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.fulltext.batch.size` — histogram observations (docs
    /// per batch).
    pub batch_size: Mutex<Vec<u32>>,
    /// `cortex.fulltext.upsert.latency_ms` — histogram observations
    /// keyed by index.
    pub upsert_latency_ms: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.fulltext.dedup.hits` — total dedup short-circuits.
    pub dedup_hits: AtomicU64,
    /// `cortex.fulltext.task_failures` — Meili task-await failures
    /// keyed by reason.
    pub task_failures: Mutex<BTreeMap<String, u64>>,
    /// `cortex.fulltext.errors` — counter keyed by HTTP status / category.
    pub errors: Mutex<BTreeMap<String, u64>>,
    /// `cortex.fulltext.skipped_empty` — events dropped because body
    /// selection produced an empty string.
    pub skipped_empty: AtomicU64,
    /// `cortex.fulltext.truncated` — docs whose `body` was truncated.
    pub truncated: AtomicU64,
    /// `cortex.fulltext.settings_bump` — count of `ensure_index` calls
    /// that pushed a fresh settings version.
    pub settings_bump: AtomicU64,
    /// `cortex.fulltext.backpressure.active` — 0 / 1 gauge.
    pub backpressure_active: AtomicU64,
    /// `cortex_fulltext_routed_total` — count of envelopes routed to
    /// each index, keyed by index name. Lets the operator confirm the
    /// spec-08 routing matrix is producing the expected distribution
    /// (every kind / topic combination should land in a known bucket;
    /// `cortex-misc` should stay non-zero but small).
    pub routed_total: Mutex<BTreeMap<String, u64>>,
}

impl Metrics {
    /// Create a fresh registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the document-upsert counter for `index` by `n`.
    pub fn incr_documents(&self, index: &str, n: u64) {
        if let Ok(mut map) = self.documents_total.lock() {
            *map.entry(index.to_string()).or_insert(0) += n;
        }
    }

    /// Record a batch-size observation.
    pub fn observe_batch_size(&self, n: u32) {
        if let Ok(mut g) = self.batch_size.lock() {
            g.push(n);
        }
    }

    /// Record a Meili upsert-latency observation in milliseconds.
    pub fn observe_upsert_latency(&self, index: &str, ms: u32) {
        if let Ok(mut map) = self.upsert_latency_ms.lock() {
            map.entry(index.to_string()).or_default().push(ms);
        }
    }

    /// Record `n` dedup short-circuits.
    pub fn incr_dedup_hits(&self, n: u64) {
        self.dedup_hits.fetch_add(n, Ordering::Relaxed);
    }

    /// Record a Meili task-await failure under `reason`.
    pub fn incr_task_failure(&self, reason: &str) {
        if let Ok(mut map) = self.task_failures.lock() {
            *map.entry(reason.to_string()).or_insert(0) += 1;
        }
    }

    /// Record an error observation in `category`.
    pub fn incr_errors(&self, category: &str) {
        if let Ok(mut map) = self.errors.lock() {
            *map.entry(category.to_string()).or_insert(0) += 1;
        }
    }

    /// Increment the skipped-empty counter.
    pub fn incr_skipped_empty(&self) {
        self.skipped_empty.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the truncated-doc counter.
    pub fn incr_truncated(&self) {
        self.truncated.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the settings-bump counter.
    pub fn incr_settings_bump(&self) {
        self.settings_bump.fetch_add(1, Ordering::Relaxed);
    }

    /// Flip the backpressure gauge.
    pub fn set_backpressure(&self, active: bool) {
        self.backpressure_active
            .store(u64::from(active), Ordering::Relaxed);
    }

    /// Record one envelope routed into `index`. Drives the
    /// `cortex_fulltext_routed_total{index=...}` counter.
    pub fn incr_routed(&self, index: &str) {
        if let Ok(mut map) = self.routed_total.lock() {
            *map.entry(index.to_string()).or_insert(0) += 1;
        }
    }

    /// Snapshot the routed-total counter (test / dashboard helper).
    pub fn routed_snapshot(&self) -> BTreeMap<String, u64> {
        self.routed_total
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}
