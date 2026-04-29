//! In-process counters / histograms that back the `cortex.graph.*`
//! metric family in `docs/specs/07-graph-writer.md` §Observability.
//!
//! Light-weight atomic-counter implementation matching the embedder's
//! [`crate::embedder::Metrics`] style. A Prometheus / OpenTelemetry
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
    /// `cortex_graph_edges_dropped{edge_type}` — count of edges that
    /// the writer attempted to MERGE but Nexus 1.15 silently dropped
    /// (returned `count(*) == 0` from `MATCH (a),(b) MERGE (a)-[r]->(b)
    /// RETURN count(*)`). Spec-07 §Post-write verification: every batch
    /// reconciles attempted-vs-persisted edge counts and increments
    /// this counter for the shortfall. A non-zero rate flags either a
    /// missing-endpoint cross-batch race or a remote schema drift.
    pub edges_dropped: Mutex<BTreeMap<String, u64>>,
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

    /// Record one edge that the writer attempted but Nexus dropped.
    /// Drives `cortex_graph_edges_dropped{edge_type=...}`.
    pub fn incr_edges_dropped(&self, edge_type: &str) {
        if let Ok(mut map) = self.edges_dropped.lock() {
            *map.entry(edge_type.to_string()).or_insert(0) += 1;
        }
    }

    /// Snapshot the edges_dropped map (test / dashboard helper).
    pub fn edges_dropped_snapshot(&self) -> BTreeMap<String, u64> {
        self.edges_dropped
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_starts_zeroed() {
        let m = Metrics::new();
        assert!(m.nodes_upserted.lock().unwrap().is_empty());
        assert!(m.edges_upserted.lock().unwrap().is_empty());
        assert_eq!(m.dedup_hits_nodes.load(Ordering::Relaxed), 0);
        assert_eq!(m.dedup_hits_edges.load(Ordering::Relaxed), 0);
        assert!(m.tx_latency_ms.lock().unwrap().is_empty());
        assert!(m.tx_size.lock().unwrap().is_empty());
        assert!(m.errors.lock().unwrap().is_empty());
        assert!(m.orphans.lock().unwrap().is_empty());
        assert_eq!(m.backpressure_active.load(Ordering::Relaxed), 0);
        assert!(m.edges_dropped_snapshot().is_empty());
    }

    #[test]
    fn incr_nodes_upserted_accumulates_per_label() {
        let m = Metrics::new();
        m.incr_nodes_upserted("Turn", 2);
        m.incr_nodes_upserted("Turn", 3);
        m.incr_nodes_upserted("Artifact", 1);
        let map = m.nodes_upserted.lock().unwrap();
        assert_eq!(map.get("Turn"), Some(&5));
        assert_eq!(map.get("Artifact"), Some(&1));
    }

    #[test]
    fn incr_edges_upserted_accumulates_per_type() {
        let m = Metrics::new();
        m.incr_edges_upserted("HAS_TURN", 4);
        m.incr_edges_upserted("HAS_TURN", 1);
        let map = m.edges_upserted.lock().unwrap();
        assert_eq!(map.get("HAS_TURN"), Some(&5));
    }

    #[test]
    fn dedup_counters_are_atomic() {
        let m = Metrics::new();
        m.incr_dedup_hits_nodes(7);
        m.incr_dedup_hits_nodes(3);
        m.incr_dedup_hits_edges(1);
        assert_eq!(m.dedup_hits_nodes.load(Ordering::Relaxed), 10);
        assert_eq!(m.dedup_hits_edges.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn observe_latency_and_size_appends() {
        let m = Metrics::new();
        m.observe_tx_latency(11);
        m.observe_tx_latency(22);
        m.observe_tx_size(50);
        assert_eq!(m.tx_latency_ms.lock().unwrap().as_slice(), &[11, 22]);
        assert_eq!(m.tx_size.lock().unwrap().as_slice(), &[50]);
    }

    #[test]
    fn errors_and_orphans_keyed_counters() {
        let m = Metrics::new();
        m.incr_errors("transient");
        m.incr_errors("transient");
        m.incr_errors("permanent");
        m.incr_orphans("Turn");
        assert_eq!(m.errors.lock().unwrap().get("transient"), Some(&2));
        assert_eq!(m.errors.lock().unwrap().get("permanent"), Some(&1));
        assert_eq!(m.orphans.lock().unwrap().get("Turn"), Some(&1));
    }

    #[test]
    fn set_backpressure_round_trips() {
        let m = Metrics::new();
        m.set_backpressure(true);
        assert_eq!(m.backpressure_active.load(Ordering::Relaxed), 1);
        m.set_backpressure(false);
        assert_eq!(m.backpressure_active.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn edges_dropped_keyed_per_type_and_snapshot_clones() {
        let m = Metrics::new();
        m.incr_edges_dropped("IN_REPO");
        m.incr_edges_dropped("IN_REPO");
        m.incr_edges_dropped("HAS_TURN");
        let snap = m.edges_dropped_snapshot();
        assert_eq!(snap.get("IN_REPO"), Some(&2));
        assert_eq!(snap.get("HAS_TURN"), Some(&1));
        // Snapshot is a clone — mutating the source doesn't affect it.
        m.incr_edges_dropped("IN_REPO");
        assert_eq!(snap.get("IN_REPO"), Some(&2));
    }
}
