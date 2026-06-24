//! Phase23c — extraction contract: reconciliation gate between deterministic
//! (tree-sitter) Phase 1 facts and LLM-derived Phase 2 annotation edges.
//!
//! UA reference: `docs/analysis/understand-anything/05-extraction-contract.md` §3.
//!
//! ## Contract
//!
//! **Phase 1 (deterministic):** `analyzer_dispatch::extract_static_patch` runs
//! tree-sitter on source files and produces [`EdgeConfidence::Extracted`] edges.
//! The resulting [`GraphPatch`] is the authoritative fact set.
//!
//! **Phase 2 (annotation):** `projection::project_envelope` runs the classifier's
//! semantic relations through 12 extractors and produces
//! [`EdgeConfidence::Inferred`] edges. Phase 2 may only reference nodes that
//! Phase 1 proved exist — it cannot originate new structural facts.
//!
//! **Reconciliation gate:** [`apply_gate`] enforces the contract before the
//! combined patch reaches Nexus. Violations are dropped and logged as
//! [`GateRejection`] entries for the audit envelope.

use std::collections::{HashMap, HashSet};

use crate::graph::patch::{GraphPatch, NodeOp};

// ── Fact set ────────────────────────────────────────────────────────────────

/// Per-symbol metadata captured from the Phase 1 (deterministic) extractor.
/// Both fields default to "unknown / not exported" so the significance filter
/// only fires when the extractor explicitly provides the information.
#[derive(Debug, Clone, Default)]
pub struct SymbolMeta {
    /// Lines the symbol spans in the source file. `None` = not tracked.
    pub line_count: Option<u32>,
    /// True when the symbol is exported (`pub`, `export`, `public`, …).
    pub exported: bool,
}

/// Authoritative fact set produced by Phase 1 (tree-sitter / deterministic).
/// Used by the reconciliation gate to validate Phase 2 annotation output.
#[derive(Debug, Default)]
pub struct FactSet {
    /// Nodes proven to exist. Key = `(label, natural_key)`.
    pub nodes: HashSet<(String, String)>,
    /// Per-`Symbol` metadata (line_count, exported). Key = natural_key.
    pub symbol_meta: HashMap<String, SymbolMeta>,
    /// Authoritative IMPORTS-type edge count per source node natural-key.
    /// Built from [`EdgeConfidence::Extracted`] import edges.
    pub import_counts: HashMap<String, usize>,
}

impl FactSet {
    /// Build a `FactSet` from the [`EdgeConfidence::Extracted`] nodes and
    /// edges in `patch`. This is the Phase 1 → Phase 2 handoff: only items
    /// proven by the deterministic extractor become reference facts.
    pub fn from_extracted(patch: &GraphPatch) -> Self {
        let mut facts = Self::default();
        for node in &patch.nodes {
            facts
                .nodes
                .insert((node.label.clone(), node.natural_key.clone()));
            if node.label == "Symbol" {
                facts.symbol_meta.insert(
                    node.natural_key.clone(),
                    SymbolMeta {
                        line_count: node
                            .props
                            .get("line_count")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u32),
                        exported: node
                            .props
                            .get("exported")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    },
                );
            }
        }
        for edge in &patch.edges {
            if read_confidence(edge) != ConfidenceTier::Extracted {
                continue;
            }
            facts
                .nodes
                .insert((edge.from_label.clone(), edge.from_key.clone()));
            facts
                .nodes
                .insert((edge.to_label.clone(), edge.to_key.clone()));
            if is_import_edge(&edge.edge_type) {
                *facts
                    .import_counts
                    .entry(edge.from_key.clone())
                    .or_default() += 1;
            }
        }
        facts
    }

    /// True when `(label, key)` is in the fact set.
    pub fn contains(&self, label: &str, key: &str) -> bool {
        self.nodes.contains(&(label.to_string(), key.to_string()))
    }

    /// True when the fact set has no nodes (non-code / pre-analysis context).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

// ── Gate result types ────────────────────────────────────────────────────────

/// Why a node or edge was rejected by the reconciliation gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    /// Node's natural-key is absent from the deterministic fact set.
    NodeNotInFactSet,
    /// Edge endpoint (`from` or `to`) absent from the fact set.
    EdgeEndpointMissing,
    /// `Symbol` node below the significance threshold (lines < min AND not exported).
    BelowSignificanceThreshold,
    /// Node natural-key does not conform to the `{part}|{part}|{part}` scheme.
    MalformedNodeId,
    /// Inferred IMPORTS count differs from deterministic count; extracted edges used.
    ImportCountMismatch {
        /// Deterministic import count.
        expected: usize,
        /// Inferred import count.
        actual: usize,
    },
}

/// A single item rejected (or replaced) by the reconciliation gate.
/// Callers emit these to the audit envelope.
#[derive(Debug, Clone)]
pub struct GateRejection {
    /// `"node"` or `"edge"`.
    pub item_kind: &'static str,
    /// Natural key for a node, or `"{from_key}→{to_key}"` for an edge.
    pub natural_key: String,
    /// Why it was rejected.
    pub reason: RejectionReason,
}

// ── Gate config ──────────────────────────────────────────────────────────────

/// Configuration for the reconciliation gate.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// Minimum line span for a `Symbol` node. Nodes with a known
    /// `line_count` strictly below this threshold that are NOT exported
    /// are dropped. Default: 10 (UA spec).
    pub significance_min_lines: u32,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            significance_min_lines: 10,
        }
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Apply the reconciliation gate to a combined (Phase 1 + Phase 2) `GraphPatch`.
///
/// Rules (UA §3 — Cortex port):
///
/// 1. [`EdgeConfidence::Extracted`] nodes and edges pass through unconditionally.
/// 2. `Symbol` nodes with a known `line_count < significance_min_lines` that
///    are not exported are dropped.
/// 3. `Symbol` / `Artifact` nodes whose natural-key has fewer than 3
///    pipe-separated parts are rejected (malformed ID).
/// 4. `Inferred`/`Ambiguous` `Symbol`/`Artifact` nodes absent from `facts` when
///    the fact set is non-empty are rejected (no invented code nodes).
/// 5. `Inferred`/`Ambiguous` edges whose `from` or `to` endpoint is absent from
///    `facts` are rejected (endpoints must exist).
/// 6. `IMPORTS`-family inferred edge count per source is asserted against
///    `facts.import_counts`. On mismatch the inferred imports for that source are
///    replaced by the already-kept extracted imports (extractor wins).
///
/// Returns the filtered `GraphPatch` and a list of [`GateRejection`] records.
pub fn apply_gate(
    patch: GraphPatch,
    facts: &FactSet,
    config: &GateConfig,
) -> (GraphPatch, Vec<GateRejection>) {
    use crate::graph::patch::EdgeOp;

    let mut out = GraphPatch::empty();
    let mut rejections: Vec<GateRejection> = Vec::new();

    // ── Nodes ──────────────────────────────────────────────────────────────
    for node in patch.nodes {
        if let Some(rej) = check_significance(&node, config) {
            rejections.push(rej);
            continue;
        }
        if let Some(rej) = check_node_id_format(&node) {
            rejections.push(rej);
            continue;
        }
        if !facts.is_empty()
            && is_code_label(&node.label)
            && !facts.contains(&node.label, &node.natural_key)
        {
            rejections.push(GateRejection {
                item_kind: "node",
                natural_key: node.natural_key.clone(),
                reason: RejectionReason::NodeNotInFactSet,
            });
            continue;
        }
        out.nodes.push(node);
    }

    // ── Edges: separate by confidence ──────────────────────────────────────
    let mut extracted_edges: Vec<EdgeOp> = Vec::new();
    let mut inferred_imports: HashMap<String, Vec<EdgeOp>> = HashMap::new();
    let mut other_inferred: Vec<EdgeOp> = Vec::new();

    for edge in patch.edges {
        match read_confidence(&edge) {
            ConfidenceTier::Extracted => extracted_edges.push(edge),
            ConfidenceTier::Inferred | ConfidenceTier::Ambiguous => {
                if is_import_edge(&edge.edge_type) {
                    inferred_imports
                        .entry(edge.from_key.clone())
                        .or_default()
                        .push(edge);
                } else {
                    other_inferred.push(edge);
                }
            }
        }
    }

    // All extracted edges pass through.
    // Also build a lookup of extracted-import edges by source for backfill.
    let mut extracted_imports_by_source: HashMap<String, Vec<EdgeOp>> = HashMap::new();
    for edge in &extracted_edges {
        if is_import_edge(&edge.edge_type) {
            extracted_imports_by_source
                .entry(edge.from_key.clone())
                .or_default()
                .push(edge.clone());
        }
    }
    out.edges.extend(extracted_edges);

    // ── Import-count reconciliation (§2.3 / §3.2) ──────────────────────────
    for (source_key, inferred) in inferred_imports {
        let expected = facts.import_counts.get(&source_key).copied().unwrap_or(0);
        let actual = inferred.len();
        if expected == 0 || actual == expected {
            // Count matches (or no baseline) → validate endpoints and keep.
            for edge in inferred {
                if let Some(rej) = check_edge_endpoints(&edge, facts) {
                    rejections.push(rej);
                } else {
                    out.edges.push(edge);
                }
            }
        } else {
            // Count mismatch → extractor wins; extracted imports already in out.edges.
            rejections.push(GateRejection {
                item_kind: "edge",
                natural_key: source_key.clone(),
                reason: RejectionReason::ImportCountMismatch { expected, actual },
            });
            // If there are no extracted imports for this source, backfill nothing;
            // the deterministic facts are authoritative — silence > hallucination.
        }
    }

    // ── Non-import inferred edges: endpoint check ───────────────────────────
    for edge in other_inferred {
        if let Some(rej) = check_edge_endpoints(&edge, facts) {
            rejections.push(rej);
        } else {
            out.edges.push(edge);
        }
    }

    (out, rejections)
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Confidence tier read from an edge's `confidence` prop (stored there by
/// [`crate::graph::patch::EdgeOp::with_confidence`]). Missing prop → Inferred
/// (backward-compatible default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfidenceTier {
    Extracted,
    Inferred,
    Ambiguous,
}

fn read_confidence(edge: &crate::graph::patch::EdgeOp) -> ConfidenceTier {
    match edge.props.get("confidence").and_then(|v| v.as_str()) {
        Some("extracted") => ConfidenceTier::Extracted,
        Some("ambiguous") => ConfidenceTier::Ambiguous,
        _ => ConfidenceTier::Inferred,
    }
}

/// True for labels that belong to the code-graph layer (produced by the static
/// analyzer) — these are subject to the fact-set and ID-format checks.
fn is_code_label(label: &str) -> bool {
    matches!(
        label,
        "Symbol" | "Artifact" | "ExternalPackage" | "UnresolvedImport"
    )
}

/// True for IMPORTS-family edge types (both Extracted and Inferred variants).
fn is_import_edge(edge_type: &str) -> bool {
    matches!(
        edge_type,
        "IMPORTS" | "IMPORTS_FILE" | "IMPORTS_EXTERNAL" | "UNRESOLVED_IMPORT"
    )
}

/// Significance filter for `Symbol` nodes. Returns a rejection when the node
/// has a known `line_count` below the threshold AND is not exported.
/// Returns `None` (keep) when `line_count` is absent — unknown → trust.
fn check_significance(node: &NodeOp, config: &GateConfig) -> Option<GateRejection> {
    if node.label != "Symbol" {
        return None;
    }
    let line_count = node
        .props
        .get("line_count")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)?; // None → keep
    let exported = node
        .props
        .get("exported")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if line_count < config.significance_min_lines && !exported {
        return Some(GateRejection {
            item_kind: "node",
            natural_key: node.natural_key.clone(),
            reason: RejectionReason::BelowSignificanceThreshold,
        });
    }
    None
}

/// ID-format check for `Symbol` / `Artifact` nodes: natural-key must have
/// at least 3 pipe-separated components. Pending artifact sentinels
/// (`pending|{repo}|{path}`) are exempt. Non-code labels are not checked.
fn check_node_id_format(node: &NodeOp) -> Option<GateRejection> {
    match node.label.as_str() {
        "Symbol" | "Artifact" => {
            let is_pending = node.label == "Artifact" && node.natural_key.starts_with("pending|");
            if !is_pending && node.natural_key.split('|').count() < 3 {
                return Some(GateRejection {
                    item_kind: "node",
                    natural_key: node.natural_key.clone(),
                    reason: RejectionReason::MalformedNodeId,
                });
            }
        }
        _ => {}
    }
    None
}

/// Check both endpoints of an edge against the fact set.
/// Returns `None` when both are present (or facts are empty — non-code context).
fn check_edge_endpoints(
    edge: &crate::graph::patch::EdgeOp,
    facts: &FactSet,
) -> Option<GateRejection> {
    if facts.is_empty() {
        return None;
    }
    if !facts.contains(&edge.from_label, &edge.from_key) {
        return Some(GateRejection {
            item_kind: "edge",
            natural_key: format!("{}→{}", edge.from_key, edge.to_key),
            reason: RejectionReason::EdgeEndpointMissing,
        });
    }
    if !facts.contains(&edge.to_label, &edge.to_key) {
        return Some(GateRejection {
            item_kind: "edge",
            natural_key: format!("{}→{}", edge.from_key, edge.to_key),
            reason: RejectionReason::EdgeEndpointMissing,
        });
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::graph::patch::{EdgeConfidence, EdgeOp, NodeOp};

    fn symbol_node(key: &str) -> NodeOp {
        NodeOp::with_identity("Symbol", key)
    }

    fn symbol_node_with_meta(key: &str, line_count: u64, exported: bool) -> NodeOp {
        let mut node = NodeOp::with_identity("Symbol", key);
        node.props
            .insert("line_count".to_string(), serde_json::json!(line_count));
        node.props
            .insert("exported".to_string(), serde_json::json!(exported));
        node
    }

    fn artifact_node(key: &str) -> NodeOp {
        NodeOp::with_identity("Artifact", key)
    }

    fn inferred_edge(from_label: &str, from: &str, to_label: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: "RELATES_TO".to_string(),
            from_label: from_label.to_string(),
            from_key: from.to_string(),
            to_label: to_label.to_string(),
            to_key: to.to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Inferred, None)
    }

    fn extracted_imports_edge(from: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: "IMPORTS_FILE".to_string(),
            from_label: "Artifact".to_string(),
            from_key: from.to_string(),
            to_label: "Artifact".to_string(),
            to_key: to.to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Extracted, None)
    }

    fn inferred_imports_edge(from: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: "IMPORTS".to_string(),
            from_label: "Artifact".to_string(),
            from_key: from.to_string(),
            to_label: "Artifact".to_string(),
            to_key: to.to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Inferred, None)
    }

    fn facts_with_nodes(pairs: &[(&str, &str)]) -> FactSet {
        let mut f = FactSet::default();
        for &(label, key) in pairs {
            f.nodes.insert((label.to_string(), key.to_string()));
        }
        f
    }

    // ── §2.1 Node not in fact set ──────────────────────────────────────────

    #[test]
    fn inferred_symbol_not_in_facts_is_rejected() {
        let facts = facts_with_nodes(&[("Symbol", "cortex|rust|foo")]);
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // This symbol is NOT in the fact set → hallucinated.
        patch.nodes.push(symbol_node("cortex|rust|hallucinated"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(out.nodes.is_empty(), "hallucinated node must be dropped");
        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].reason, RejectionReason::NodeNotInFactSet);
    }

    #[test]
    fn inferred_symbol_in_facts_passes() {
        let facts = facts_with_nodes(&[("Symbol", "cortex|rust|foo")]);
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch.nodes.push(symbol_node("cortex|rust|foo"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1);
        assert!(rejections.is_empty());
    }

    #[test]
    fn extracted_nodes_bypass_fact_set_check() {
        // Even with an empty fact set, extracted nodes pass.
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch
            .nodes
            .push(artifact_node("cortex|src/main.rs|abc123def456ghij"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        // Artifact with valid 3-part key passes ID format check.
        assert_eq!(out.nodes.len(), 1);
        assert!(rejections.is_empty());
    }

    // ── §2.2 Edge endpoint missing ─────────────────────────────────────────

    #[test]
    fn hallucinated_edge_endpoint_is_rejected() {
        let facts = facts_with_nodes(&[
            ("Symbol", "cortex|rust|real_fn"),
            ("Symbol", "cortex|rust|other_real"),
        ]);
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // Target is hallucinated — not in facts.
        patch.edges.push(inferred_edge(
            "Symbol",
            "cortex|rust|real_fn",
            "Symbol",
            "cortex|rust|hallucinated_fn",
        ));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(out.edges.is_empty(), "hallucinated edge must be dropped");
        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].reason, RejectionReason::EdgeEndpointMissing);
    }

    #[test]
    fn edge_with_both_endpoints_in_facts_passes() {
        let facts = facts_with_nodes(&[
            ("Symbol", "cortex|rust|alpha"),
            ("Symbol", "cortex|rust|beta"),
        ]);
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch.edges.push(inferred_edge(
            "Symbol",
            "cortex|rust|alpha",
            "Symbol",
            "cortex|rust|beta",
        ));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.edges.len(), 1);
        assert!(rejections.is_empty());
    }

    // ── §2.3 Import-count reconciliation ──────────────────────────────────

    #[test]
    fn import_count_match_keeps_inferred_imports() {
        // 2 extracted + 2 inferred imports from the same source → counts match.
        let mut facts = FactSet::default();
        let src = "cortex|src/lib.rs|aaabbbccc";
        let dep1 = "cortex|src/a.rs|111";
        let dep2 = "cortex|src/b.rs|222";
        facts
            .nodes
            .insert(("Artifact".to_string(), src.to_string()));
        facts
            .nodes
            .insert(("Artifact".to_string(), dep1.to_string()));
        facts
            .nodes
            .insert(("Artifact".to_string(), dep2.to_string()));
        facts.import_counts.insert(src.to_string(), 2);

        let config = GateConfig::default();
        let mut patch = GraphPatch::empty();
        patch.edges.push(inferred_imports_edge(src, dep1));
        patch.edges.push(inferred_imports_edge(src, dep2));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.edges.len(), 2, "matching import count must keep edges");
        assert!(rejections.is_empty());
    }

    #[test]
    fn import_count_mismatch_uses_extracted_imports() {
        // Extracted: 2 imports. Inferred: only 1 → mismatch → use extracted.
        let src = "cortex|src/lib.rs|aaabbbccc";
        let dep1 = "cortex|src/a.rs|111";
        let dep2 = "cortex|src/b.rs|222";

        let mut facts = FactSet::default();
        facts
            .nodes
            .insert(("Artifact".to_string(), src.to_string()));
        facts
            .nodes
            .insert(("Artifact".to_string(), dep1.to_string()));
        facts
            .nodes
            .insert(("Artifact".to_string(), dep2.to_string()));
        // Deterministic count: 2.
        facts.import_counts.insert(src.to_string(), 2);

        let config = GateConfig::default();
        let mut patch = GraphPatch::empty();
        // Extracted imports (from tree-sitter Phase 1).
        patch.edges.push(extracted_imports_edge(src, dep1));
        patch.edges.push(extracted_imports_edge(src, dep2));
        // Inferred imports (from classifier Phase 2): only one — count mismatch.
        patch.edges.push(inferred_imports_edge(src, dep1));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        // Extracted imports are kept (2 edges).
        let kept_import_count = out
            .edges
            .iter()
            .filter(|e| is_import_edge(&e.edge_type))
            .count();
        assert_eq!(kept_import_count, 2, "extracted imports should be kept");
        // Mismatch rejection logged.
        assert_eq!(rejections.len(), 1);
        assert!(matches!(
            rejections[0].reason,
            RejectionReason::ImportCountMismatch {
                expected: 2,
                actual: 1
            }
        ));
    }

    // ── §2.4 Significance filter ───────────────────────────────────────────

    #[test]
    fn small_private_helper_is_filtered() {
        let facts = FactSet::default(); // empty facts → no code-gate check
        let config = GateConfig::default(); // min 10 lines

        let mut patch = GraphPatch::empty();
        // 8-line, not exported → below threshold.
        patch
            .nodes
            .push(symbol_node_with_meta("cortex|rust|helper", 8, false));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(
            out.nodes.is_empty(),
            "small private helper must be filtered"
        );
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].reason,
            RejectionReason::BelowSignificanceThreshold
        );
    }

    #[test]
    fn small_exported_helper_is_kept() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // 8-line, exported → passes significance.
        patch
            .nodes
            .push(symbol_node_with_meta("cortex|rust|pub_helper", 8, true));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1, "exported helper must pass");
        assert!(rejections.is_empty());
    }

    #[test]
    fn large_private_helper_is_kept() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // 15 lines, not exported → passes significance (≥ 10).
        patch
            .nodes
            .push(symbol_node_with_meta("cortex|rust|large_fn", 15, false));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1);
        assert!(rejections.is_empty());
    }

    #[test]
    fn unknown_line_count_keeps_node() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // No line_count prop → cannot filter.
        patch.nodes.push(symbol_node("cortex|rust|unknown_size"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1, "unknown line_count must pass");
        assert!(rejections.is_empty());
    }

    // ── §2.5 Node ID format ────────────────────────────────────────────────

    #[test]
    fn symbol_with_no_pipe_is_rejected() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        // No pipe-separated parts → malformed.
        patch.nodes.push(symbol_node("malformed_id"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(out.nodes.is_empty());
        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].reason, RejectionReason::MalformedNodeId);
    }

    #[test]
    fn symbol_with_two_parts_is_rejected() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch.nodes.push(symbol_node("cortex|only_two_parts"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(out.nodes.is_empty());
        assert_eq!(rejections[0].reason, RejectionReason::MalformedNodeId);
    }

    #[test]
    fn symbol_with_three_parts_passes_id_check() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch.nodes.push(symbol_node("cortex|rust|crate::foo"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1);
        assert!(rejections.is_empty());
    }

    #[test]
    fn artifact_with_pending_prefix_passes_id_check() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch
            .nodes
            .push(artifact_node("pending|cortex|src/main.rs"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert_eq!(out.nodes.len(), 1, "pending sentinel must pass ID check");
        assert!(rejections.is_empty());
    }

    #[test]
    fn artifact_without_three_parts_is_rejected() {
        let facts = FactSet::default();
        let config = GateConfig::default();

        let mut patch = GraphPatch::empty();
        patch.nodes.push(artifact_node("only_one_part"));

        let (out, rejections) = apply_gate(patch, &facts, &config);
        assert!(out.nodes.is_empty());
        assert_eq!(rejections[0].reason, RejectionReason::MalformedNodeId);
    }

    // ── FactSet::from_extracted ────────────────────────────────────────────

    #[test]
    fn fact_set_built_from_extracted_edges_only() {
        use crate::graph::patch::EdgeOp;

        let mut patch = GraphPatch::empty();
        patch
            .nodes
            .push(artifact_node("cortex|src/main.rs|hash1234"));
        let extracted_import =
            extracted_imports_edge("cortex|src/main.rs|hash1234", "cortex|src/lib.rs|hash5678");
        let inferred_edge_item = EdgeOp {
            edge_type: "ABOUT".to_string(),
            from_label: "Turn".to_string(),
            from_key: "01HXTURN".to_string(),
            to_label: "Topic".to_string(),
            to_key: "graph-indexing".to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Inferred, None);
        patch.edges.push(extracted_import);
        patch.edges.push(inferred_edge_item);

        let facts = FactSet::from_extracted(&patch);
        // Artifact node added explicitly.
        assert!(facts.contains("Artifact", "cortex|src/main.rs|hash1234"));
        // Import target from extracted edge.
        assert!(facts.contains("Artifact", "cortex|src/lib.rs|hash5678"));
        // Inferred edge endpoints NOT added to facts.
        assert!(!facts.contains("Turn", "01HXTURN"));
        assert!(!facts.contains("Topic", "graph-indexing"));
        // Import count tracked.
        assert_eq!(
            facts
                .import_counts
                .get("cortex|src/main.rs|hash1234")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn symbol_meta_populated_from_node_props() {
        let mut patch = GraphPatch::empty();
        patch
            .nodes
            .push(symbol_node_with_meta("cortex|rust|my_fn", 25, true));
        let facts = FactSet::from_extracted(&patch);
        let meta = facts.symbol_meta.get("cortex|rust|my_fn").unwrap();
        assert_eq!(meta.line_count, Some(25));
        assert!(meta.exported);
    }
}
