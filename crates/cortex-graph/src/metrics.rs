//! In-process counters / histograms that back the `cortex.graph.*`
//! metric family in `docs/specs/07-graph-writer.md` §Observability.
//!
//! Light-weight atomic-counter implementation matching the embedder's
//! [`cortex_embedder::Metrics`] style. A Prometheus / OpenTelemetry
//! exporter can read these values directly or wrap them in a registry —
//! that wiring lives outside this crate.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Graph-writer metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.graph.nodes.upserted` — total node upserts written,
    /// keyed by node label.
    pub nodes_upserted: Mutex<BTreeMap<String, u64>>,
    /// `cortex.graph.edges.upserted` — total edge upserts written,
    /// keyed by edge type.
    pub edges_upserted: Mutex<BTreeMap<String, u64>>,
    /// `cortex.graph.dedup.hits{kind=node}`.
    pub dedup_hits_nodes: AtomicU64,
    /// `cortex.graph.dedup.hits{kind=edge}`.
    pub dedup_hits_edges: AtomicU64,
    /// `cortex.graph.tx.latency_ms` — histogram observations (ms).
    pub tx_latency_ms: Mutex<Vec<u32>>,
    /// `cortex.graph.tx.size` — histogram observations (ops per tx).
    pub tx_size: Mutex<Vec<u32>>,
    /// `cortex.graph.errors` — counter, keyed by error category.
    pub errors: Mutex<BTreeMap<String, u64>>,
    /// `cortex.graph.orphans` — counter, keyed by parent label.
    pub orphans: Mutex<BTreeMap<String, u64>>,
    /// `cortex.graph.backpressure.active` — 0 / 1 gauge.
    pub backpressure_active: AtomicU64,
}

impl Metrics {
    /// Create a fresh registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the node-upsert counter for `label` by `n`.
    pub fn incr_nodes_upserted(&self, label: &str, n: u64) {
        if let Ok(mut map) = self.nodes_upserted.lock() {
            *map.entry(label.to_string()).or_insert(0) += n;
        }
    }

    /// Increment the edge-upsert counter for `edge_type` by `n`.
    pub fn incr_edges_upserted(&self, edge_type: &str, n: u64) {
        if let Ok(mut map) = self.edges_upserted.lock() {
            *map.entry(edge_type.to_string()).or_insert(0) += n;
        }
    }

    /// Record `n` node-dedup hits.
    pub fn incr_dedup_hits_nodes(&self, n: u64) {
        self.dedup_hits_nodes.fetch_add(n, Ordering::Relaxed);
    }

    /// Record `n` edge-dedup hits.
    pub fn incr_dedup_hits_edges(&self, n: u64) {
        self.dedup_hits_edges.fetch_add(n, Ordering::Relaxed);
    }

    /// Record a transaction latency observation in milliseconds.
    pub fn observe_tx_latency(&self, ms: u32) {
        if let Ok(mut g) = self.tx_latency_ms.lock() {
            g.push(ms);
        }
    }

    /// Record a transaction-size observation (ops per tx).
    pub fn observe_tx_size(&self, ops: u32) {
        if let Ok(mut g) = self.tx_size.lock() {
            g.push(ops);
        }
    }

    /// Record an error observation in `category`.
    pub fn incr_errors(&self, category: &str) {
        if let Ok(mut map) = self.errors.lock() {
            *map.entry(category.to_string()).or_insert(0) += 1;
        }
    }

    /// Record an orphan-fabrication observation for `parent_label`.
    pub fn incr_orphans(&self, parent_label: &str) {
        if let Ok(mut map) = self.orphans.lock() {
            *map.entry(parent_label.to_string()).or_insert(0) += 1;
        }
    }

    /// Flip the backpressure gauge.
    pub fn set_backpressure(&self, active: bool) {
        self.backpressure_active
            .store(u64::from(active), Ordering::Relaxed);
    }
}
