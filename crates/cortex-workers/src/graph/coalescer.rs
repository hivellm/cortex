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

#[cfg(test)]
use super::patch::ConflictPolicy;
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
///
/// Phase11k §5.4 added a per-session `(content_hash, analyzer_version)`
/// dedup so a search-and-replace burst against the same artifact does
/// not re-emit identical static-extraction patches. The session
/// table lives on the coalescer struct (not the free `coalesce`
/// function) so the worker can carry it across batches without the
/// caller plumbing a third arg through.
#[derive(Debug, Default)]
pub struct PatchCoalescer {
    /// Seen `(label, natural_key)` pairs in the current window.
    pub seen_nodes: BTreeSet<(String, String)>,
    /// Seen edge tuples in the current window. Populated only when edge
    /// dedup is enabled by a future caller; left empty by [`coalesce`].
    pub seen_edges: BTreeSet<(String, String, String, String, String)>,
    /// Phase11k §5.4 — per-session dedup on `(content_hash,
    /// analyzer_version)`. The key is `(repo, path, content_hash,
    /// analyzer_version)` so two different paths that share a hash
    /// (rare but possible in deduplicated archives) do not
    /// false-collapse, and a bumped version forces re-emission.
    /// Populated by [`Self::observe_static_emission`] from the
    /// graph worker's batch loop.
    pub seen_static_emissions: BTreeSet<(String, String, String, String)>,
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
        self.seen_static_emissions.clear();
    }

    /// Phase11k §5.4 — record a `(repo, path, content_hash,
    /// analyzer_version)` tuple as already-emitted within this
    /// session. Returns `true` when the tuple was unseen (callers
    /// should proceed with the static extraction) and `false` when
    /// it has already been observed (callers should skip re-emission
    /// to dedup a search-and-replace burst).
    pub fn observe_static_emission(
        &mut self,
        repo: &str,
        path: &str,
        content_hash: &str,
        analyzer_version: &str,
    ) -> bool {
        let key = (
            repo.to_string(),
            path.to_string(),
            content_hash.to_string(),
            analyzer_version.to_string(),
        );
        self.seen_static_emissions.insert(key)
    }

    /// Number of `(content_hash, analyzer_version)` tuples observed
    /// so far in this session. Useful for tests + metric pull.
    pub fn static_emission_count(&self) -> usize {
        self.seen_static_emissions.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(label: &str, key: &str, props: Vec<(&str, serde_json::Value)>) -> NodeOp {
        NodeOp {
            label: label.into(),
            natural_key: key.into(),
            external_id: Some(key.into()),
            conflict_policy: ConflictPolicy::default(),
            props: props.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    fn edge(t: &str, from: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: t.into(),
            from_label: "A".into(),
            from_key: from.into(),
            to_label: "B".into(),
            to_key: to.into(),
            props: BTreeMap::new(),
        }
    }

    #[test]
    fn coalesce_dedupes_nodes_by_label_and_key() {
        let p1 = GraphPatch {
            nodes: vec![node("Turn", "t-1", vec![("a", json!(1))])],
            edges: vec![],
        };
        let p2 = GraphPatch {
            nodes: vec![
                node("Turn", "t-1", vec![("b", json!(2))]),
                node("Turn", "t-2", vec![]),
            ],
            edges: vec![],
        };
        let (out, stats) = coalesce(vec![p1, p2]);
        // Two unique nodes (t-1 and t-2); 1 dedup hit.
        assert_eq!(out.nodes.len(), 2);
        assert_eq!(stats.node_dedup_hits, 1);
        // Order preserved by first-seen.
        assert_eq!(out.nodes[0].natural_key, "t-1");
        assert_eq!(out.nodes[1].natural_key, "t-2");
        // Props merged: t-1 has both `a` and `b`.
        assert_eq!(out.nodes[0].props.get("a"), Some(&json!(1)));
        assert_eq!(out.nodes[0].props.get("b"), Some(&json!(2)));
    }

    #[test]
    fn coalesce_does_not_dedupe_edges() {
        let p = GraphPatch {
            nodes: vec![],
            edges: vec![
                edge("TOUCHED", "tc-1", "art-1"),
                edge("TOUCHED", "tc-1", "art-1"),
                edge("TOUCHED", "tc-1", "art-1"),
            ],
        };
        let (out, stats) = coalesce(vec![p]);
        // All three edges preserved; edge_dedup_hits stays at 0.
        assert_eq!(out.edges.len(), 3);
        assert_eq!(stats.edge_dedup_hits, 0);
    }

    #[test]
    fn coalesce_empty_input_yields_empty_patch() {
        let (out, stats) = coalesce(Vec::new());
        assert!(out.is_empty());
        assert_eq!(stats.node_dedup_hits, 0);
        assert_eq!(stats.edge_dedup_hits, 0);
    }

    #[test]
    fn coalesce_preserves_first_seen_order_across_patches() {
        let p1 = GraphPatch {
            nodes: vec![node("X", "1", vec![]), node("X", "2", vec![])],
            edges: vec![],
        };
        let p2 = GraphPatch {
            nodes: vec![node("X", "3", vec![]), node("X", "1", vec![])],
            edges: vec![],
        };
        let (out, stats) = coalesce(vec![p1, p2]);
        let keys: Vec<&str> = out.nodes.iter().map(|n| n.natural_key.as_str()).collect();
        assert_eq!(keys, vec!["1", "2", "3"]);
        assert_eq!(stats.node_dedup_hits, 1);
    }

    #[test]
    fn later_props_overwrite_earlier_for_same_key() {
        let p1 = GraphPatch {
            nodes: vec![node("X", "1", vec![("k", json!("first"))])],
            edges: vec![],
        };
        let p2 = GraphPatch {
            nodes: vec![node("X", "1", vec![("k", json!("second"))])],
            edges: vec![],
        };
        let (out, _) = coalesce(vec![p1, p2]);
        assert_eq!(out.nodes[0].props.get("k"), Some(&json!("second")));
    }

    #[test]
    fn coalescer_new_and_reset() {
        let mut c = PatchCoalescer::new();
        c.seen_nodes.insert(("X".into(), "1".into()));
        c.seen_edges
            .insert(("a".into(), "b".into(), "c".into(), "d".into(), "e".into()));
        c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1");
        assert_eq!(c.seen_nodes.len(), 1);
        assert_eq!(c.seen_edges.len(), 1);
        assert_eq!(c.static_emission_count(), 1);
        c.reset();
        assert!(c.seen_nodes.is_empty());
        assert!(c.seen_edges.is_empty());
        assert_eq!(c.static_emission_count(), 0);
    }

    // ---------- Phase11k §5.4 — per-session static-emission dedup ----------

    #[test]
    fn observe_static_emission_first_call_returns_true() {
        let mut c = PatchCoalescer::new();
        assert!(c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1"));
        assert_eq!(c.static_emission_count(), 1);
    }

    #[test]
    fn observe_static_emission_repeat_returns_false() {
        let mut c = PatchCoalescer::new();
        c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1");
        // Identical tuple — must dedup.
        assert!(!c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1"));
        assert_eq!(c.static_emission_count(), 1);
    }

    #[test]
    fn observe_static_emission_different_hash_is_unseen() {
        let mut c = PatchCoalescer::new();
        c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1");
        // Same path + version but new content_hash — search-and-
        // replace burst landed a real change; must NOT dedup.
        assert!(c.observe_static_emission("cortex", "src/lib.rs", "sha256:def", "phase11l.1"));
    }

    #[test]
    fn observe_static_emission_version_bump_forces_re_emission() {
        let mut c = PatchCoalescer::new();
        c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11k.1");
        // Same content_hash but a bumped analyzer_version — must
        // re-emit so the bumped extraction logic actually lands.
        assert!(c.observe_static_emission("cortex", "src/lib.rs", "sha256:abc", "phase11l.1"));
    }

    #[test]
    fn observe_static_emission_distinct_paths_do_not_collapse() {
        let mut c = PatchCoalescer::new();
        c.observe_static_emission("cortex", "src/a.rs", "sha256:abc", "phase11l.1");
        // Different path — even if some other quirk produced the
        // same hash, the tuple key keeps them apart.
        assert!(c.observe_static_emission("cortex", "src/b.rs", "sha256:abc", "phase11l.1"));
        assert_eq!(c.static_emission_count(), 2);
    }
}
