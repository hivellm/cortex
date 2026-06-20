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

/// Confidence tier for a projected edge (phase27a).
///
/// Distinguishes deterministic, AST-proven edges from analyzer/LLM-
/// inferred ones so the query graph lane can weight by trust and the
/// dashboard can flag uncertain edges. Stored on the edge as the
/// `confidence` prop (this enum, snake_case) plus a `confidence_score`
/// prop (float in `[0.0, 1.0]`), alongside the provenance triple. An
/// edge without these props is treated as unknown confidence — additive
/// and back-compatible with pre-phase27a edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeConfidence {
    /// Deterministically proven by an AST / tree-sitter parse
    /// (`defines`, `imports`, `calls`, `returns`, `inherits`). Default
    /// score 1.0.
    Extracted,
    /// Derived by a heuristic analyzer or an LLM (`relates_to`, `about`,
    /// `answered_by`, `cites`, `mentions_file`). Sub-1.0 score.
    Inferred,
    /// Low-confidence match, kept but flagged for review.
    Ambiguous,
}

impl EdgeConfidence {
    /// Canonical lowercase string stamped into the `confidence` edge prop.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeConfidence::Extracted => "extracted",
            EdgeConfidence::Inferred => "inferred",
            EdgeConfidence::Ambiguous => "ambiguous",
        }
    }

    /// Default numeric score used when a caller stamps a tier without an
    /// explicit score. `Extracted` is always certain (1.0); the inferred
    /// tiers carry representative mid-band values callers may override.
    #[must_use]
    pub fn default_score(self) -> f32 {
        match self {
            EdgeConfidence::Extracted => 1.0,
            EdgeConfidence::Inferred => 0.7,
            EdgeConfidence::Ambiguous => 0.4,
        }
    }
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

impl EdgeOp {
    /// Phase27a — stamp a [`EdgeConfidence`] tier (and optional explicit
    /// score) onto the edge as the `confidence` / `confidence_score`
    /// props, alongside the provenance triple. When `score` is `None`
    /// the tier's [`EdgeConfidence::default_score`] is used. Additive and
    /// idempotent: consumers that don't know about confidence ignore the
    /// extra props, and edges written before this change read back as
    /// unknown confidence.
    #[must_use]
    pub fn with_confidence(mut self, tier: EdgeConfidence, score: Option<f32>) -> Self {
        let score = score.unwrap_or_else(|| tier.default_score());
        // Round in f64 space to 3 decimals so the persisted score is a
        // clean value (e.g. 0.55, not 0.550000011920929 from the f32→f64
        // widening) when it lands on the Nexus edge / dashboard.
        let score = ((score as f64) * 1000.0).round() / 1000.0;
        self.props.insert(
            "confidence".to_string(),
            serde_json::Value::String(tier.as_str().to_string()),
        );
        if let Some(n) = serde_json::Number::from_f64(score) {
            self.props
                .insert("confidence_score".to_string(), serde_json::Value::Number(n));
        }
        self
    }
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
        let mut r = GraphWriteReport {
            nodes_upserted: 3,
            edges_upserted: 2,
            ..GraphWriteReport::default()
        };
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

    // ---------- Phase27a — edge confidence ----------

    #[test]
    fn edge_confidence_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&EdgeConfidence::Extracted).unwrap(),
            "\"extracted\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeConfidence::Inferred).unwrap(),
            "\"inferred\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeConfidence::Ambiguous).unwrap(),
            "\"ambiguous\""
        );
    }

    #[test]
    fn edge_confidence_default_score_per_tier() {
        assert_eq!(EdgeConfidence::Extracted.default_score(), 1.0);
        assert!(EdgeConfidence::Inferred.default_score() < 1.0);
        assert!(EdgeConfidence::Ambiguous.default_score() < EdgeConfidence::Inferred.default_score());
        assert_eq!(EdgeConfidence::Extracted.as_str(), "extracted");
    }

    #[test]
    fn with_confidence_stamps_props_using_default_score() {
        let e = edge("CALLS").with_confidence(EdgeConfidence::Extracted, None);
        assert_eq!(
            e.props.get("confidence"),
            Some(&serde_json::Value::String("extracted".into()))
        );
        assert_eq!(
            e.props.get("confidence_score").and_then(|v| v.as_f64()),
            Some(1.0)
        );
    }

    #[test]
    fn with_confidence_honors_explicit_score() {
        let e = edge("RELATES_TO").with_confidence(EdgeConfidence::Inferred, Some(0.55));
        assert_eq!(
            e.props.get("confidence"),
            Some(&serde_json::Value::String("inferred".into()))
        );
        assert_eq!(
            e.props.get("confidence_score").and_then(|v| v.as_f64()),
            Some(0.55)
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

/// Filter for bulk edge deletions issued by the stale-edge sweeper
/// (phase11k §5.3).
///
/// All fields are optional; the resulting Cypher predicate is the
/// AND-conjunction of every `Some` field. An all-`None` filter matches
/// every edge in the graph — callers must populate at least one field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgeDeleteFilter {
    /// Restrict deletion to edges stamped with this `analyzer_version`
    /// property. Used to retire output from a superseded analyzer run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzer_version: Option<String>,
    /// Restrict deletion to edges whose `source_event_id` is in this list.
    /// The sweeper passes the set of stale event ids identified by the
    /// hash-change guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_ids: Option<Vec<String>>,
    /// Restrict deletion to edges of these relationship types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_types: Option<Vec<String>>,
    /// Restrict deletion to edges whose `created_at_ms` is older than
    /// this Unix-epoch-ms threshold (exclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_than_ms: Option<i64>,
    /// Restrict deletion to edges whose source (from) node carries this
    /// label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_label: Option<String>,
    /// Restrict deletion to edges whose target (to) node natural key
    /// starts with this prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_natural_key_prefix: Option<String>,
}

impl EdgeDeleteFilter {
    /// Return `true` if at least one field is `Some`. The sweeper must
    /// never issue a delete with an all-`None` filter to avoid
    /// accidentally wiping the entire graph.
    pub fn is_non_empty(&self) -> bool {
        self.analyzer_version.is_some()
            || self.source_event_ids.is_some()
            || self.edge_types.is_some()
            || self.older_than_ms.is_some()
            || self.from_label.is_some()
            || self.to_natural_key_prefix.is_some()
    }

    /// Build the WHERE clause body (without the `WHERE` keyword) that
    /// corresponds to this filter. Returns `None` when the filter is empty.
    pub fn to_cypher_predicate(&self) -> Option<String> {
        let mut clauses: Vec<String> = Vec::new();

        if let Some(ver) = &self.analyzer_version {
            clauses.push(format!(
                "r.analyzer_version = \"{}\"",
                escape_cypher_str(ver)
            ));
        }
        if let Some(ids) = &self.source_event_ids {
            if !ids.is_empty() {
                let list: Vec<String> = ids
                    .iter()
                    .map(|id| format!("\"{}\"", escape_cypher_str(id)))
                    .collect();
                clauses.push(format!("r.source_event_id IN [{}]", list.join(", ")));
            }
        }
        if let Some(types) = &self.edge_types {
            if !types.is_empty() {
                let list: Vec<String> = types
                    .iter()
                    .map(|t| format!("\"{}\"", escape_cypher_str(t)))
                    .collect();
                clauses.push(format!("type(r) IN [{}]", list.join(", ")));
            }
        }
        if let Some(ms) = self.older_than_ms {
            clauses.push(format!("r.created_at_ms < {ms}"));
        }
        if let Some(lbl) = &self.from_label {
            clauses.push(format!("\"{}\" IN labels(a)", escape_cypher_str(lbl)));
        }
        if let Some(prefix) = &self.to_natural_key_prefix {
            clauses.push(format!(
                "b.natural_key STARTS WITH \"{}\"",
                escape_cypher_str(prefix)
            ));
        }

        if clauses.is_empty() {
            None
        } else {
            Some(clauses.join(" AND "))
        }
    }
}

/// Minimal string escaping for values embedded in a Cypher predicate.
/// Only handles double-quotes and backslashes; all other characters are
/// passed through as-is. Non-ASCII characters are replaced with `?` to
/// stay consistent with `cypher_string_literal` in `nexus_client.rs`.
fn escape_cypher_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x80 => out.push(c),
            _ => out.push('?'),
        }
    }
    out
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
