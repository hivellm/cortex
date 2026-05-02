//! Graph-patch types — the in-memory representation of a batch of node /
//! edge upserts before it goes on the wire to Nexus.
//!
//! The shape mirrors `docs/specs/07-graph-writer.md` §Inputs/Outputs and
//! §Event-to-graph mapping. A [`GraphPatch`] is what the mapper produces
//! per event and what the coalescer merges across a micro-batch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Conflict-resolution policy applied when a node upsert lands on
/// an `external_id` that already exists in the catalog. Mirrors the
/// Nexus 2.1 `ConflictPolicy` enum (`error` / `match` / `replace`)
/// the SDK's `create_node_with_external_id` accepts.
///
/// Phase11l §2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Return the existing internal id, leave properties untouched.
    /// Default for every Cortex upsert: re-running the same patch
    /// stays idempotent without overwriting whatever the graph
    /// worker / dashboard already learned about the node.
    #[default]
    Match,
    /// Reuse the existing internal id but overwrite the property
    /// bag. Used by the §5 sentinel-redirect path once the
    /// canonical content_hash arrives.
    Replace,
    /// Surface a duplicate-id error. Reserved for the future
    /// `cortex-ops graph drop` admin command's post-drop sanity
    /// check; the regular write path uses `Match`.
    Error,
}

/// Upsert entry for a single node.
///
/// `natural_key` is the primary identity Cortex stamps on every node
/// (the legacy `MERGE { natural_key: $key }` Cypher path still relies
/// on it; the §3 templates rewrite drops the `MERGE` form). Phase11l
/// §2 added [`Self::external_id`] as the canonical identity slot the
/// Nexus 2.1 `_id` route reads — it shadows `natural_key` once the
/// new templates ship and the legacy field becomes a soft fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOp {
    /// Cypher node label (e.g. `Turn`, `Artifact`, `Decision`).
    pub label: String,
    /// Natural key for the node (e.g. `turn_id`, or
    /// `repo|path|content_hash` for `Artifact`).
    pub natural_key: String,
    /// Phase11l §2.1 — Nexus 2.1 reserved `_id` slot. When `Some`,
    /// the writer issues `CREATE … {_id: external_id} ON CONFLICT
    /// <conflict_policy>` instead of the legacy `MERGE` shape. When
    /// `None`, the legacy `MERGE { natural_key }` path runs as a
    /// soft fallback. New construction sites populate this from
    /// `natural_key` via [`super::identity::external_id_for_node`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Phase11l §2.1 — conflict policy passed to the Nexus 2.1
    /// upsert. Defaults to [`ConflictPolicy::Match`] so re-running
    /// the same patch is idempotent without clobbering downstream-
    /// stamped properties.
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
    /// Property bag to set with `SET n += $props`.
    pub props: BTreeMap<String, serde_json::Value>,
}

impl NodeOp {
    /// Phase11l §2 — preferred construction path. Stamps both the
    /// legacy `natural_key` (for the soft `MERGE` fallback) AND the
    /// Nexus 2.1 `external_id` slot from the same canonical key
    /// string. `conflict_policy` defaults to
    /// [`ConflictPolicy::Match`]; callers that need `Replace` or
    /// `Error` semantics override post-construction.
    pub fn with_identity(label: impl Into<String>, natural_key: impl Into<String>) -> Self {
        let nk = natural_key.into();
        Self {
            label: label.into(),
            external_id: Some(nk.clone()),
            natural_key: nk,
            conflict_policy: ConflictPolicy::default(),
            props: BTreeMap::new(),
        }
    }
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
        NodeOp::with_identity(label, key)
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

    // ---------- Phase11l §2.4 ----------

    #[test]
    fn conflict_policy_default_is_match() {
        assert_eq!(ConflictPolicy::default(), ConflictPolicy::Match);
    }

    #[test]
    fn conflict_policy_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConflictPolicy::Match).unwrap(),
            "\"match\""
        );
        assert_eq!(
            serde_json::to_string(&ConflictPolicy::Replace).unwrap(),
            "\"replace\""
        );
        assert_eq!(
            serde_json::to_string(&ConflictPolicy::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn node_op_with_identity_populates_external_id_from_natural_key() {
        let n = NodeOp::with_identity("Artifact", "cortex|src/lib.rs|sha256:abc");
        assert_eq!(n.label, "Artifact");
        assert_eq!(n.natural_key, "cortex|src/lib.rs|sha256:abc");
        assert_eq!(
            n.external_id.as_deref(),
            Some("cortex|src/lib.rs|sha256:abc"),
            "external_id must mirror natural_key for the soft-fallback period"
        );
        assert_eq!(n.conflict_policy, ConflictPolicy::Match);
        assert!(n.props.is_empty());
    }

    #[test]
    fn node_op_serde_round_trips_with_external_id_and_policy() {
        let mut n = NodeOp::with_identity("Symbol", "cortex|rust|crate::foo::bar");
        n.props
            .insert("language".into(), serde_json::Value::String("rust".into()));
        n.conflict_policy = ConflictPolicy::Replace;
        let json = serde_json::to_string(&n).unwrap();
        assert!(
            json.contains("\"external_id\":\"cortex|rust|crate::foo::bar\""),
            "external_id must serialise; got {json}"
        );
        assert!(
            json.contains("\"conflict_policy\":\"replace\""),
            "conflict_policy must serialise; got {json}"
        );
        let parsed: NodeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.external_id.as_deref(),
            Some("cortex|rust|crate::foo::bar")
        );
        assert_eq!(parsed.conflict_policy, ConflictPolicy::Replace);
    }

    #[test]
    fn node_op_legacy_json_without_new_fields_still_deserializes() {
        // Wire shape from before phase11l §2 — `external_id` and
        // `conflict_policy` absent. The soft-fallback contract requires
        // both fields default cleanly so a half-replayed archive does
        // not block the worker.
        let legacy = r#"{
            "label":"Artifact",
            "natural_key":"cortex|src/x.rs|sha256:abc",
            "props":{}
        }"#;
        let parsed: NodeOp = serde_json::from_str(legacy).unwrap();
        assert!(parsed.external_id.is_none());
        assert_eq!(parsed.conflict_policy, ConflictPolicy::Match);
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
