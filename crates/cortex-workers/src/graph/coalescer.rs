//! Patch coalescer — deduplicates node upserts within a single
//! micro-batch so Nexus is not asked to `MERGE` the same `(label, key)`
//! pair twice.
//!
//! Per spec 07 §Concurrency:
//!
//! > Patch coalescer: deduplicates node/edge upserts within a micro-batch
//! > (same `TOUCHED(ToolCall, Artifact)` seen twice in one window is
//! > written once). Cuts Nexus work by ~40% on bootstrap traffic where
//! > many events touch the same file.
//!
//! Per the §Acceptance criteria: nodes are deduped across events in a
//! batch; edges are **not** deduped — 100 events touching the same
//! Artifact still produce 100 `TOUCHED` edges. The implementation here
//! enforces exactly that contract: it walks all input patches, keeps a
//! `(label, natural_key)` seen-set so each unique node is emitted once,
//! and forwards every edge unchanged. When the same node is observed
//! twice, the property bag of the first occurrence wins; later
//! occurrences merge their props on top via [`BTreeMap::extend`], so
//! later-arriving values overwrite earlier ones for the same key. This
//! matches Cypher `SET n += row.props` semantics on Nexus.

use std::collections::{BTreeMap, BTreeSet};

use super::patch::{EdgeOp, GraphPatch, NodeOp};

/// Counters returned alongside a coalesced patch.
#[derive(Debug, Clone, Default)]
pub struct CoalesceStats {
    /// Number of node upserts the coalescer dropped as duplicates.
    pub node_dedup_hits: u32,
    /// Number of edge upserts the coalescer dropped as duplicates.
    /// Today the coalescer leaves edges alone; this counter exists for
    /// callers and metrics so the read surface is stable when edge
    /// dedup is added behind a flag.
    pub edge_dedup_hits: u32,
}

/// Stateful coalescer reused across coalesce calls within one window.
///
/// `seen_nodes` is repopulated on every [`coalesce`] call from the input
/// patches; the type holds it as a struct field so that windows that
/// span multiple coalesce passes (future feature) can reuse the
/// allocation by calling [`PatchCoalescer::reset`] between passes.
#[derive(Debug, Default)]
pub struct PatchCoalescer {
    /// Seen `(label, natural_key)` pairs in the current window.
    pub seen_nodes: BTreeSet<(String, String)>,
    /// Seen edge tuples in the current window. Populated only when edge
    /// dedup is enabled by a future caller; left empty by [`coalesce`].
    pub seen_edges: BTreeSet<(String, String, String, String, String)>,
}

impl PatchCoalescer {
    /// Create a fresh coalescer with empty seen-sets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the seen-sets between batches while keeping the allocation.
    pub fn reset(&mut self) {
        self.seen_nodes.clear();
        self.seen_edges.clear();
    }
}

/// Merge `patches` into a single [`GraphPatch`] alongside coalesce stats.
///
/// Nodes are deduplicated by `(label, natural_key)` — duplicate
/// occurrences increment `node_dedup_hits` and have their property bag
/// merged on top of the first occurrence. Edges are forwarded unchanged.
pub fn coalesce(patches: Vec<GraphPatch>) -> (GraphPatch, CoalesceStats) {
    let mut node_index: BTreeMap<(String, String), NodeOp> = BTreeMap::new();
    let mut node_order: Vec<(String, String)> = Vec::new();
    let mut edges: Vec<EdgeOp> = Vec::new();
    let mut stats = CoalesceStats::default();

    for patch in patches {
        for node in patch.nodes {
            let key = (node.label.clone(), node.natural_key.clone());
            match node_index.get_mut(&key) {
                Some(existing) => {
                    existing.props.extend(node.props);
                    stats.node_dedup_hits += 1;
                }
                None => {
                    node_order.push(key.clone());
                    node_index.insert(key, node);
                }
            }
        }
        edges.extend(patch.edges);
    }

    let nodes: Vec<NodeOp> = node_order
        .into_iter()
        .filter_map(|key| node_index.remove(&key))
        .collect();

    (GraphPatch { nodes, edges }, stats)
}
