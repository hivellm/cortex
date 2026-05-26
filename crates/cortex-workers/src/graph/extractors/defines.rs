//! Phase15b §2.3 — `DEFINES` edge extractor.
//!
//! Surfaces symbol-definition relationships from code-bearing
//! envelopes. The classifier (Sonnet path) pins `DEFINES` as one of
//! the canonical relation labels at
//! `crates/cortex-workers/src/classifier/types.rs::ExtractedRelation`
//! ("event introduces / defines an entity"), so this extractor
//! has a tighter wire shape than §2.1 / §2.2 — Sonnet emits the
//! canonical uppercase form and the case-insensitive match is
//! retained only for robustness.
//!
//! ## Scope
//!
//! - Code-bearing envelope kinds only: `Kind::ToolCall`
//!   (Edit/Write tools that define new symbols in the payload)
//!   plus `Kind::Artifact` (the bootstrap walker emits one per
//!   indexed source file; the artifact body declares every
//!   symbol the language's parser exposes). Other kinds carry
//!   their own semantic edges via §2.5 / §2.10 / §2.11.
//! - Endpoint resolution via the shared
//!   [`super::extract_via_classifier_relations`]: `"this_event"`
//!   anchors at `(<kind-label>, event_id)`; entity-table lookup
//!   resolves typed targets; unknown identifiers drop silently.
//! - Coexists with the analyzer pipeline's tree-sitter
//!   `function_item` / `struct_item` / `impl_item` extraction —
//!   that path stamps a different `analyzer_version` so the two
//!   never collide under the `(from, to, kind)` unique constraint.

use cortex_core::events::Kind;

use super::{extract_via_classifier_relations, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "DEFINES";

/// Extract `DEFINES` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for envelope kinds that do not carry code, or for
/// code-bearing envelopes whose classifier output carries no
/// `DEFINES` relation.
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
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no DEFINES edges"
            );
        }
    }

    #[test]
    fn extract_emits_defines_edge_on_tool_call_with_canonical_uppercase() {
        let env = with_relations(
            "01HXTC01",
            Kind::ToolCall,
            vec![entity("concept", "cortex_workers::graph::Extractor")],
            vec![rel(
                "this_event",
                "DEFINES",
                "cortex_workers::graph::Extractor",
            )],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "DEFINES");
        assert_eq!(edges[0].from_label, "ToolCall");
        assert_eq!(edges[0].to_label, "Concept");
    }

    #[test]
    fn extract_emits_defines_edge_on_artifact_envelope() {
        let env = with_relations(
            "01HXART01",
            Kind::Artifact,
            vec![entity("concept", "crate::Foo")],
            vec![rel("this_event", "DEFINES", "crate::Foo")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_label, "Artifact");
        assert_eq!(edges[0].from_key, "01HXART01");
    }

    #[test]
    fn extract_matches_relation_label_case_insensitively() {
        for label in ["DEFINES", "Defines", "defines", "DeFiNeS"] {
            let env = with_relations(
                "01HXTC02",
                Kind::ToolCall,
                vec![entity("concept", "crate::Bar")],
                vec![rel("this_event", label, "crate::Bar")],
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
            vec![rel("this_event", "DEFINES", "ghost::Sym")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let env = with_relations(
            "01HXTC04",
            Kind::ToolCall,
            vec![entity("concept", "crate::Baz")],
            vec![rel("this_event", "DEFINES", "crate::Baz")],
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
    fn extract_skips_non_defines_relations_in_same_envelope() {
        let env = with_relations(
            "01HXTC05",
            Kind::ToolCall,
            vec![entity("concept", "crate::Qux")],
            vec![
                rel("this_event", "REFERENCES", "crate::Qux"),
                rel("this_event", "DEFINES", "crate::Qux"),
                rel("this_event", "IMPORTS", "crate::Qux"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1, "only the DEFINES relation must surface");
        assert_eq!(edges[0].edge_type, "DEFINES");
    }

    #[test]
    fn extract_emits_multiple_edges_when_artifact_defines_many_symbols() {
        let env = with_relations(
            "01HXART02",
            Kind::Artifact,
            vec![
                entity("concept", "crate::A"),
                entity("concept", "crate::B"),
                entity("concept", "crate::C"),
            ],
            vec![
                rel("this_event", "DEFINES", "crate::A"),
                rel("this_event", "DEFINES", "crate::B"),
                rel("this_event", "DEFINES", "crate::C"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 3);
    }
}
