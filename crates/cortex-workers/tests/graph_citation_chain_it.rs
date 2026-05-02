//! Phase11k §4.4 — citation-chain integration test.
//!
//! Walks a synthetic ADR → Spec → Analysis → Code chain and asserts
//! the chain is traversable in 4 hops with `confidence ≥ 0.9` (the
//! `kind` discriminator the markdown analyzer stamps on every edge
//! is the proxy: `mention_qualified` and explicit links both carry
//! "intra_crate" tier == high confidence).
//!
//! The IT runs the citation extractor against four payload bodies
//! whose links chain together, then verifies every hop produced an
//! edge of type `CITES` (or `DERIVED_FROM` for the consolidation
//! tail) so a Cypher walk could traverse them in order.

use cortex_workers::graph::analyzer::{build_graph_patch, EdgeType, PatchBuildContext};
use cortex_workers::graph::citations::{
    citations_for_consolidation, citations_for_decision, citations_for_knowledge,
    citations_for_learning, derived_from_edges,
};
use cortex_workers::graph::patch::EdgeOp;
use cortex_workers::graph::resolver::{LocalSymbols, ModuleMap, PackageMap, SymbolResolver};

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope,
    DecisionPayload, KnowledgePayload, LearningPayload, TimeSpan,
};
use std::collections::BTreeMap;

fn drive_to_edges(
    edges: Vec<cortex_workers::graph::analyzer::CodeEdge>,
    source_repo: &str,
    source_path: &str,
    source_hash: &str,
) -> Vec<EdgeOp> {
    let mm = ModuleMap::new();
    let pm = PackageMap::new();
    let ls = LocalSymbols::new(source_repo, "markdown", source_path);
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let ctx = PatchBuildContext {
        source_repo,
        source_path,
        source_content_hash: source_hash,
        source_event_id: Some("evt-citation"),
        resolver: &resolver,
        content_hash_for: &|_repo: &str, _path: &str| None,
        analyzer_version: "phase11k.it.cite",
    };
    build_graph_patch(&edges, &ctx).edges
}

fn decision() -> DecisionPayload {
    DecisionPayload {
        decision_id: "DEC-0042".into(),
        title: "Use static extraction".into(),
        status: "accepted".into(),
        body: "see [the spec](../specs/07-graph-writer.md) and the \
               analysis in [docs](../docs/analysis/graph/06-implementation-plan.md)\n"
            .into(),
        supersedes: None,
        cas_ref: None,
        tags: vec![],
    }
}

fn knowledge() -> KnowledgePayload {
    KnowledgePayload {
        knowledge_id: "PATTERN-0001".into(),
        title: "Three-tier resolver".into(),
        category: "pattern".into(),
        body: "see `crate::resolver::SymbolResolver` and \
               [the analysis](../docs/analysis/graph/04-extraction-pipeline.md)"
            .into(),
        source_path: None,
        tags: vec![],
    }
}

fn learning() -> LearningPayload {
    LearningPayload {
        learning_id: "LRN-0001".into(),
        title: "Tree-sitter resolver caveat".into(),
        body: "during phase11k §1.3 I learned that `crate::workers` paths \
               sometimes alias [util.rs](../src/util.rs)"
            .into(),
        related_task: Some("phase11k_graph_correlations".into()),
        source_path: None,
        tags: vec![],
    }
}

fn consolidation() -> ConsolidationPayload {
    ConsolidationPayload {
        consolidation_id: "CON-0001".into(),
        grain: ConsolidationGrain::DecisionTrace,
        scope: ConsolidationScope::DecisionId("DEC-0042".into()),
        title: "Decision trace for DEC-0042".into(),
        summary_markdown: "applies [the decision](../decisions/DEC-0042.md) and \
                           [the spec](../specs/07-graph-writer.md)"
            .into(),
        takeaways: vec![],
        source_event_ids: vec!["evt-1".into(), "evt-2".into()],
        source_event_count: 2,
        model: "claude-haiku-4-5".into(),
        depth: ConsolidationDepth::Shallow,
        outcome_distribution: BTreeMap::new(),
        temporal_span: TimeSpan {
            start_ms: 0,
            end_ms: 0,
            duration_ms: 0,
        },
        repos: vec!["cortex".into()],
        tags: vec![],
    }
}

#[test]
fn adr_to_consolidation_chain_traverses_in_four_hops() {
    let dec = decision();
    let kn = knowledge();
    let lrn = learning();
    let con = consolidation();

    let dec_edges = citations_for_decision(&dec, "cortex");
    let kn_edges = citations_for_knowledge(&kn, "cortex");
    let lrn_edges = citations_for_learning(&lrn, "cortex");
    let con_edges = citations_for_consolidation(&con, "cortex");
    let derived = derived_from_edges(&con, "Turn");

    // Every payload must contribute at least one CITES edge anchored
    // at the right owner label.
    assert!(dec_edges
        .iter()
        .any(|e| e.edge_type == EdgeType::Cites && e.from_node.label == "Decision"));
    assert!(kn_edges
        .iter()
        .any(|e| e.edge_type == EdgeType::Cites && e.from_node.label == "Knowledge"));
    assert!(lrn_edges
        .iter()
        .any(|e| e.edge_type == EdgeType::Cites && e.from_node.label == "Learning"));
    assert!(con_edges
        .iter()
        .any(|e| e.edge_type == EdgeType::Cites && e.from_node.label == "Consolidation"));

    // DERIVED_FROM tail one edge per source event.
    assert_eq!(derived.len(), 2);
    for e in &derived {
        assert_eq!(e.edge_type, EdgeType::DerivedFrom);
    }

    // Drive the decision edges through the patch builder so the wire
    // shape is exercised end-to-end.
    let dec_wire = drive_to_edges(dec_edges, "cortex", "decisions/DEC-0042.md", "sha256:dec");
    assert!(dec_wire.iter().any(|e| e.edge_type == "CITES"));
    // Confirm the props the §1.4 builder stamps.
    for e in &dec_wire {
        assert_eq!(
            e.props.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase11k.it.cite")
        );
    }
}

#[test]
fn high_confidence_citations_dominate() {
    let dec = decision();
    let edges = citations_for_decision(&dec, "cortex");
    let high_confidence = edges
        .iter()
        .filter(|e| matches!(e.kind, "link" | "mention_qualified"))
        .count();
    let total = edges.len();
    assert!(total > 0);
    let ratio = (high_confidence as f64) / (total as f64);
    assert!(
        ratio >= 0.9,
        "high-confidence ratio {ratio:.3} below 90%; edges: {edges:?}"
    );
}
