//! Graph-patch types — the in-memory representation of a batch of node /
//! edge upserts before it goes on the wire to Nexus.
//!
//! The shape mirrors `docs/specs/07-graph-writer.md` §Inputs/Outputs and
//! §Event-to-graph mapping. A [`GraphPatch`] is what the mapper produces
//! per event and what the coalescer merges across a micro-batch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Upsert entry for a single node.
///
/// `natural_key` is the primary identity used inside Nexus `MERGE`
/// statements; `props` is the bag of properties to set / update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOp {
    /// Cypher node label (e.g. `Turn`, `Artifact`, `Decision`).
    pub label: String,
    /// Natural key for the node (e.g. `turn_id`, or
    /// `repo|path|content_hash` for `Artifact`).
    pub natural_key: String,
    /// Property bag to set with `SET n += $props`.
    pub props: BTreeMap<String, serde_json::Value>,
}

/// Upsert entry for a single edge.
///
/// Edges are stored directed (from → to), matched against the natural keys
/// of their endpoints; the endpoint nodes are expected to be upserted in
/// the same patch (or already exist).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeOp {
    /// Cypher relationship type (e.g. `HAS_TURN`, `TOUCHED`).
    pub edge_type: String,
    /// Source node label.
    pub from_label: String,
    /// Source node natural key.
    pub from_key: String,
    /// Target node label.
    pub to_label: String,
    /// Target node natural key.
    pub to_key: String,
    /// Property bag set on the relationship.
    pub props: BTreeMap<String, serde_json::Value>,
}

/// A batch of node / edge upserts that will be turned into one Cypher
/// transaction. Produced by [`super::map_event_to_patch`] per event and
/// merged by [`crate::PatchCoalescer`] across a micro-batch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphPatch {
    /// Nodes to upsert.
    pub nodes: Vec<NodeOp>,
    /// Edges to upsert.
    pub edges: Vec<EdgeOp>,
}

impl GraphPatch {
    /// Empty patch — no work to do.
    pub fn empty() -> Self {
        Self::default()
    }

    /// True when the patch has no nodes and no edges.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(label: &str, key: &str) -> NodeOp {
        NodeOp {
            label: label.into(),
            natural_key: key.into(),
            props: BTreeMap::new(),
        }
    }

    fn edge(t: &str) -> EdgeOp {
        EdgeOp {
            edge_type: t.into(),
            from_label: "A".into(),
            from_key: "a".into(),
            to_label: "B".into(),
            to_key: "b".into(),
            props: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_patch_is_empty_and_round_trips_default() {
        let p = GraphPatch::empty();
        assert!(p.is_empty());
        let d = GraphPatch::default();
        assert!(d.is_empty());
    }

    #[test]
    fn patch_with_node_or_edge_is_not_empty() {
        let mut p = GraphPatch::empty();
        p.nodes.push(node("Turn", "t1"));
        assert!(!p.is_empty());
        let mut q = GraphPatch::empty();
        q.edges.push(edge("HAS_TURN"));
        assert!(!q.is_empty());
    }

    #[test]
    fn patch_serde_round_trips() {
        let mut p = GraphPatch::empty();
        p.nodes.push(node("Artifact", "repo|src/lib.rs|sha256:abc"));
        p.edges.push(edge("IN_REPO"));
        let json = serde_json::to_string(&p).unwrap();
        let parsed: GraphPatch = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.edges.len(), 1);
        assert_eq!(parsed.nodes[0].label, "Artifact");
        assert_eq!(parsed.edges[0].edge_type, "IN_REPO");
    }

    #[test]
    fn write_report_default_is_zeroed() {
        let r = GraphWriteReport::default();
        assert_eq!(r.nodes_upserted, 0);
        assert_eq!(r.edges_upserted, 0);
        assert_eq!(r.nodes_deduped, 0);
        assert_eq!(r.edges_deduped, 0);
        assert!(r.by_label.is_empty());
        assert_eq!(r.latency_ms, 0);
    }

    #[test]
    fn write_report_serde_round_trips() {
        let mut r = GraphWriteReport::default();
        r.nodes_upserted = 3;
        r.edges_upserted = 2;
        r.by_label.insert("Turn".into(), 1);
        let json = serde_json::to_string(&r).unwrap();
        let parsed: GraphWriteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nodes_upserted, 3);
        assert_eq!(parsed.by_label.get("Turn"), Some(&1));
    }
}

/// Per-batch report returned by [`crate::GraphWriter::write_batch`].
///
/// Mirrors spec 07 §Inputs/Outputs exactly. The `by_label` map records
/// upsert counts keyed by node label so observers can see which parts
/// of the schema are changing fastest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphWriteReport {
    /// Total node upserts written to Nexus.
    pub nodes_upserted: u32,
    /// Total edge upserts written to Nexus.
    pub edges_upserted: u32,
    /// Node upserts the coalescer dropped as duplicates.
    pub nodes_deduped: u32,
    /// Edge upserts the coalescer dropped as duplicates.
    pub edges_deduped: u32,
    /// Per-label upsert counts.
    pub by_label: BTreeMap<String, u32>,
    /// Wall-clock latency of the batch in milliseconds.
    pub latency_ms: u32,
}
