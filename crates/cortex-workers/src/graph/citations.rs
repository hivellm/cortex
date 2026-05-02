//! Phase11k §4 — `:CITES` + `:DERIVED_FROM` edges from Decision /
//! Knowledge / Learning / Consolidation payload bodies.
//!
//! These payloads carry markdown prose (`body` / `summary_markdown`)
//! that mention other artifacts, decisions, sections, and symbols.
//! Running the §3 markdown analyzer over the body recovers every
//! `[link](path)` and backtick-token mention; this module rewrites
//! those edges so the *source* is the payload-owning node (Decision,
//! Knowledge, …) and the edge type is [`EdgeType::Cites`].
//!
//! `:DERIVED_FROM` is a separate, structured channel —
//! [`ConsolidationPayload::source_event_ids`] gets one edge per id so
//! the curated layer is navigable back to the raw events.

use cortex_core::events::{
    ConsolidationPayload, DecisionPayload, KnowledgePayload, LearningPayload,
};

use super::analyzer::{CodeEdge, EdgeType, NodeRef, ResolutionTarget};
use super::markdown::MarkdownAnalyzer;

/// Identifier + label for the payload-owning node a citation edge
/// anchors at. Lets `extract_payload_citations` stay generic over the
/// four payload variants.
#[derive(Debug, Clone)]
pub struct PayloadCitationSource {
    /// Nexus node label (`Decision`, `Knowledge`, `Learning`,
    /// `Consolidation`).
    pub label: &'static str,
    /// Natural key — e.g. `DEC-0042` for a Decision, the
    /// `consolidation_id` for a Consolidation.
    pub natural_key: String,
}

impl PayloadCitationSource {
    fn into_node(&self) -> NodeRef {
        NodeRef {
            label: self.label.to_string(),
            natural_key: self.natural_key.clone(),
        }
    }
}

/// Run the markdown analyzer over `body` and rewrite every produced
/// edge so its source is `owner` and its edge type is
/// [`EdgeType::Cites`]. The original `to_target` is preserved so the
/// resolver can still dispatch the citation against the workspace.
///
/// `repo` and `synthetic_path` are passed through to the markdown
/// analyzer so its byte-line tracking + relative-path resolution
/// remain accurate. `synthetic_path` should be the
/// `<entity>/<id>.md` path the body would live at if it were a real
/// file (e.g. `decisions/DEC-0042.md`) — it is never written to disk
/// but anchors the relative-path math the markdown analyzer uses.
pub fn extract_payload_citations(
    body: &str,
    owner: &PayloadCitationSource,
    repo: &str,
    synthetic_path: &str,
) -> Vec<CodeEdge> {
    let raw = MarkdownAnalyzer::new().extract(body, repo, synthetic_path);
    let owner_node = owner.into_node();
    let mut out: Vec<CodeEdge> = Vec::with_capacity(raw.len());
    for edge in raw {
        // Drop section-level scaffolding edges — the payload body is
        // not a real markdown file with its own DocSection tree. We
        // distinguish via `kind` because explicit `[text](file.rs)`
        // links also produce `Documents` edges and those legitimately
        // become `Cites`.
        if matches!(edge.kind, "section_root" | "section_contains" | "fenced_path") {
            continue;
        }
        if edge.edge_type == EdgeType::Contains {
            continue;
        }
        out.push(CodeEdge {
            from_node: owner_node.clone(),
            edge_type: EdgeType::Cites,
            to_target: edge.to_target,
            source_line: edge.source_line,
            kind: edge.kind,
        });
    }
    out
}

/// Convenience entry point for [`DecisionPayload`] — uses
/// `decisions/<decision_id>.md` as the synthetic path so relative
/// links inside the decision resolve against the conventional
/// `.rulebook/decisions/` directory.
pub fn citations_for_decision(payload: &DecisionPayload, repo: &str) -> Vec<CodeEdge> {
    let owner = PayloadCitationSource {
        label: "Decision",
        natural_key: payload.decision_id.clone(),
    };
    let synthetic = format!(".rulebook/decisions/{}.md", payload.decision_id);
    extract_payload_citations(&payload.body, &owner, repo, &synthetic)
}

/// Knowledge entries live in `.rulebook/knowledge/`.
pub fn citations_for_knowledge(payload: &KnowledgePayload, repo: &str) -> Vec<CodeEdge> {
    let owner = PayloadCitationSource {
        label: "Knowledge",
        natural_key: payload.knowledge_id.clone(),
    };
    let synthetic = payload
        .source_path
        .clone()
        .unwrap_or_else(|| format!(".rulebook/knowledge/{}.md", payload.knowledge_id));
    extract_payload_citations(&payload.body, &owner, repo, &synthetic)
}

/// Learning entries live in `.rulebook/learnings/`.
pub fn citations_for_learning(payload: &LearningPayload, repo: &str) -> Vec<CodeEdge> {
    let owner = PayloadCitationSource {
        label: "Learning",
        natural_key: payload.learning_id.clone(),
    };
    let synthetic = payload
        .source_path
        .clone()
        .unwrap_or_else(|| format!(".rulebook/learnings/{}.md", payload.learning_id));
    extract_payload_citations(&payload.body, &owner, repo, &synthetic)
}

/// Consolidation summaries are not on-disk; the synthetic path
/// rooted at `consolidations/<id>` keeps the markdown analyzer's
/// relative-path resolution tight to the consolidation namespace.
pub fn citations_for_consolidation(
    payload: &ConsolidationPayload,
    repo: &str,
) -> Vec<CodeEdge> {
    let owner = PayloadCitationSource {
        label: "Consolidation",
        natural_key: payload.consolidation_id.clone(),
    };
    let synthetic = format!("consolidations/{}.md", payload.consolidation_id);
    extract_payload_citations(&payload.summary_markdown, &owner, repo, &synthetic)
}

/// Phase11k §4.3 — materialise one `:DERIVED_FROM` edge per source
/// event id. The target node label is left for the patch builder to
/// fill in (the source events may be Turn / ToolCall / Artifact —
/// the caller knows which is which from the source event's kind).
/// Default target label is `Turn` because the consolidator's
/// session-grain producer publishes Turn-rooted source sets; callers
/// override via the second argument when they know better.
pub fn derived_from_edges(
    payload: &ConsolidationPayload,
    target_label: &str,
) -> Vec<CodeEdge> {
    let owner_node = NodeRef {
        label: "Consolidation".into(),
        natural_key: payload.consolidation_id.clone(),
    };
    payload
        .source_event_ids
        .iter()
        .map(|sid| CodeEdge {
            from_node: owner_node.clone(),
            edge_type: EdgeType::DerivedFrom,
            to_target: ResolutionTarget::Resolved(NodeRef {
                label: target_label.to_string(),
                natural_key: sid.clone(),
            }),
            source_line: None,
            kind: "consolidation_source",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{
        ConsolidationDepth, ConsolidationGrain, ConsolidationScope, TimeSpan,
    };
    use std::collections::BTreeMap;

    fn decision_fixture(body: &str) -> DecisionPayload {
        DecisionPayload {
            decision_id: "DEC-0042".into(),
            title: "Pick X".into(),
            status: "accepted".into(),
            body: body.into(),
            supersedes: None,
            cas_ref: None,
            tags: vec![],
        }
    }

    fn consolidation_fixture(summary: &str, source_ids: Vec<String>) -> ConsolidationPayload {
        ConsolidationPayload {
            consolidation_id: "CON-0001".into(),
            grain: ConsolidationGrain::Session,
            scope: ConsolidationScope::SessionId("session-x".into()),
            title: "summary".into(),
            summary_markdown: summary.into(),
            takeaways: vec![],
            source_event_ids: source_ids,
            source_event_count: 0,
            model: "claude-haiku-4-5".into(),
            depth: ConsolidationDepth::Shallow,
            outcome_distribution: BTreeMap::new(),
            temporal_span: TimeSpan {
                start_ms: 0,
                end_ms: 0,
                duration_ms: 0,
            },
            repos: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn decision_body_link_emits_cites_edge() {
        let body = "see [the spec](../specs/07-graph-writer.md) for details";
        let edges = citations_for_decision(&decision_fixture(body), "cortex");
        assert!(edges.iter().any(|e| e.edge_type == EdgeType::Cites
            && e.from_node.label == "Decision"
            && e.from_node.natural_key == "DEC-0042"));
    }

    #[test]
    fn decision_body_mention_emits_cites_edge() {
        let body = "the `crate::workers::run_worker` is canonical";
        let edges = citations_for_decision(&decision_fixture(body), "cortex");
        assert!(edges.iter().any(|e| e.edge_type == EdgeType::Cites
            && e.kind.starts_with("mention_")));
    }

    #[test]
    fn payload_citations_drop_section_scaffolding() {
        let body = "# Title\n\n## Sub\n\nbody";
        let edges = citations_for_decision(&decision_fixture(body), "cortex");
        // Heading-only bodies have nothing else to cite.
        assert!(edges.is_empty());
    }

    #[test]
    fn derived_from_edges_one_per_source_id() {
        let payload = consolidation_fixture(
            "summary",
            vec!["evt-1".into(), "evt-2".into(), "evt-3".into()],
        );
        let edges = derived_from_edges(&payload, "Turn");
        assert_eq!(edges.len(), 3);
        for e in &edges {
            assert_eq!(e.edge_type, EdgeType::DerivedFrom);
            assert_eq!(e.from_node.natural_key, "CON-0001");
        }
    }

    #[test]
    fn consolidation_body_link_routes_through_cites() {
        let payload = consolidation_fixture(
            "see [target](../crates/foo/src/lib.rs)\n",
            vec![],
        );
        let edges = citations_for_consolidation(&payload, "cortex");
        assert!(edges.iter().any(|e| e.edge_type == EdgeType::Cites));
    }
}
