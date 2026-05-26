//! Phase15b §2.2 — `IMPORTS` edge extractor.
//!
//! Surfaces import-statement relationships from code-bearing
//! envelopes. The classifier (Sonnet path) sees the redacted
//! payload (including code snippets the ToolCall input / output
//! / Artifact body carries) and stamps any extracted imports as
//! `relations: Vec<ExtractedRelation>` entries with label
//! `IMPORTS` (case-insensitive).
//!
//! ## Scope
//!
//! - Applied to envelope kinds that carry code: `Kind::ToolCall`
//!   (Edit/Write/Read tools whose `input` is code) AND
//!   `Kind::Artifact` (the artifact body IS code, surfaced via
//!   the bootstrap walker per spec 09 §File walker). Other kinds
//!   (Turn / Decision / etc.) carry their own semantic edges via
//!   §2.5 / §2.10 / §2.11 so a single `IMPORTS` relation never
//!   lands twice in the patch.
//! - Case-insensitive label match — the classifier's canonical
//!   vocab does not pin `IMPORTS`, so Sonnet emitting `Imports`
//!   / `imports` / `IMPORTS` all converge on the same edge type.
//! - Endpoint resolution mirrors §2.1: `"this_event"` anchors at
//!   `(<kind-label>, event_id)`; other identifiers resolve via
//!   the classifier's `entities` table. Unknown identifiers or
//!   unknown entity_types drop silently.
//! - The richer parser-based IMPORTS extraction (tree-sitter
//!   `use_declaration` / `import_statement` / `import_from_statement`
//!   nodes) lives behind the existing analyzer pipeline at
//!   `crates/cortex-workers/src/graph/analyzer/` — that path
//!   stamps edges with a different `analyzer_version` so the
//!   two sources coexist without dedupe conflicts under the
//!   `(from, to, kind)` unique constraint.

use cortex_core::events::Kind;

use super::{extract_via_classifier_relations, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "IMPORTS";

/// Extract `IMPORTS` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for envelope kinds that do not carry code (Turn /
/// Decision / etc.) or for code-bearing envelopes whose classifier
/// output carries no `IMPORTS` relation.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if !matches!(env.kind, Kind::ToolCall | Kind::Artifact) {
        return Vec::new();
    }
    extract_via_classifier_relations(env, ctx, EDGE_TYPE)
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use crate::classifier::{ExtractedEntity, ExtractedRelation};

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn with_relations(
        event_id: &str,
        kind: Kind,
        entities: Vec<ExtractedEntity>,
        relations: Vec<ExtractedRelation>,
    ) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, kind);
        env.classifier.entities = entities;
        env.classifier.relations = relations;
        env
    }

    fn entity(entity_type: &str, identifier: &str) -> ExtractedEntity {
        ExtractedEntity {
            entity_type: entity_type.into(),
            identifier: identifier.into(),
            label: None,
        }
    }

    fn rel(from: &str, relation: &str, to: &str) -> ExtractedRelation {
        ExtractedRelation {
            from: from.into(),
            relation: relation.into(),
            to: to.into(),
        }
    }

    #[test]
    fn extract_returns_empty_for_non_code_bearing_kinds() {
        for k in [
            Kind::Turn,
            Kind::Decision,
            Kind::Analysis,
            Kind::LawViolation,
            Kind::Memory,
            Kind::Knowledge,
            Kind::Learning,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no IMPORTS edges"
            );
        }
    }

    #[test]
    fn extract_emits_one_edge_per_imports_relation_on_tool_call() {
        let env = with_relations(
            "01HXTC01",
            Kind::ToolCall,
            vec![entity("concept", "serde::Serialize")],
            vec![rel("this_event", "IMPORTS", "serde::Serialize")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "IMPORTS");
        assert_eq!(edges[0].from_label, "ToolCall");
        assert_eq!(edges[0].from_key, "01HXTC01");
        assert_eq!(edges[0].to_label, "Concept");
        assert_eq!(edges[0].to_key, "serde::Serialize");
    }

    #[test]
    fn extract_emits_one_edge_per_imports_relation_on_artifact() {
        let env = with_relations(
            "01HXART01",
            Kind::Artifact,
            vec![entity("artifact", "cortex|src/util.rs|h")],
            vec![rel("this_event", "imports", "cortex|src/util.rs|h")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_label, "Artifact");
        assert_eq!(edges[0].from_key, "01HXART01");
        assert_eq!(edges[0].to_label, "Artifact");
    }

    #[test]
    fn extract_matches_relation_label_case_insensitively() {
        for label in ["IMPORTS", "Imports", "imports", "ImPoRtS"] {
            let env = with_relations(
                "01HXTC02",
                Kind::ToolCall,
                vec![entity("concept", "tokio::spawn")],
                vec![rel("this_event", label, "tokio::spawn")],
            );
            assert_eq!(
                extract(&env, &ctx()).len(),
                1,
                "relation `{label}` must match"
            );
        }
    }

    #[test]
    fn extract_drops_relation_with_unknown_endpoint_identifier() {
        let env = with_relations(
            "01HXTC03",
            Kind::ToolCall,
            Vec::new(),
            vec![rel("this_event", "IMPORTS", "phantom-mod")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let env = with_relations(
            "01HXTC04",
            Kind::ToolCall,
            vec![entity("concept", "std::io")],
            vec![rel("this_event", "IMPORTS", "std::io")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        let props = &edges[0].props;
        assert_eq!(
            props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTC04")
        );
        assert_eq!(
            props.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase15b-v1")
        );
        assert_eq!(
            props.get("created_at_ms").and_then(|v| v.as_i64()),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn extract_skips_non_imports_relations_in_same_envelope() {
        let env = with_relations(
            "01HXTC05",
            Kind::ToolCall,
            vec![entity("concept", "serde::Serialize")],
            vec![
                rel("this_event", "REFERENCES", "serde::Serialize"),
                rel("this_event", "IMPORTS", "serde::Serialize"),
                rel("this_event", "DEFINES", "serde::Serialize"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1, "only the IMPORTS relation must surface");
        assert_eq!(edges[0].edge_type, "IMPORTS");
    }

    #[test]
    fn extract_emits_multiple_edges_when_envelope_imports_many_modules() {
        let env = with_relations(
            "01HXTC06",
            Kind::ToolCall,
            vec![
                entity("concept", "serde::Serialize"),
                entity("concept", "tokio::sync"),
                entity("concept", "std::collections"),
            ],
            vec![
                rel("this_event", "IMPORTS", "serde::Serialize"),
                rel("this_event", "IMPORTS", "tokio::sync"),
                rel("this_event", "IMPORTS", "std::collections"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 3);
        let to_keys: Vec<&str> = edges.iter().map(|e| e.to_key.as_str()).collect();
        assert!(to_keys.contains(&"serde::Serialize"));
        assert!(to_keys.contains(&"tokio::sync"));
        assert!(to_keys.contains(&"std::collections"));
    }
}
