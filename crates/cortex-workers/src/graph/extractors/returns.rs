//! Phase15b §2.4 — `RETURNS` edge extractor.
//!
//! Surfaces return-value annotations from ToolCall envelopes.
//! When the classifier inspects a ToolCall's `output` field (and
//! the input/output pair has a clear "called X, got back type Y"
//! shape — Read returning a file body, Bash returning stdout,
//! `cargo check` returning a Result) it stamps a `RETURNS`
//! relation pointing from the ToolCall to the typed entity the
//! tool returned.
//!
//! ## Scope
//!
//! - `Kind::ToolCall` only (the spec scopes this extractor to
//!   tool-call results); Turn/Decision/etc. don't carry an
//!   output-shape concept.
//! - Same shared-helper plumbing as §2.1 / §2.2 / §2.3 —
//!   case-insensitive label match, `"this_event"` anchors at
//!   `(ToolCall, event_id)`, entity-table lookup resolves
//!   typed targets, unknown identifiers drop silently.
//! - The richer typed-return extraction (Rust `-> Result<T, E>`
//!   signatures, Python type hints, TypeScript return types)
//!   lives behind the analyzer pipeline's tree-sitter
//!   `function_signature` path; coexists via a different
//!   `analyzer_version` stamp.

use cortex_core::events::Kind;

use super::{extract_via_classifier_relations, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "RETURNS";

/// Extract `RETURNS` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for any envelope kind other than `ToolCall`, or for
/// a ToolCall whose classifier output carries no `RETURNS`
/// relation.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::ToolCall {
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

    fn tool_call_with_relations(
        event_id: &str,
        entities: Vec<ExtractedEntity>,
        relations: Vec<ExtractedRelation>,
    ) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::ToolCall);
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
    fn extract_returns_empty_for_non_tool_call_kinds() {
        for k in [
            Kind::Turn,
            Kind::Decision,
            Kind::Analysis,
            Kind::Artifact,
            Kind::Memory,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no RETURNS edges"
            );
        }
    }

    #[test]
    fn extract_emits_returns_edge_for_read_returning_file_body() {
        let env = tool_call_with_relations(
            "01HXTC01",
            vec![entity("artifact", "cortex|src/lib.rs|sha256:abc")],
            vec![rel("this_event", "RETURNS", "cortex|src/lib.rs|sha256:abc")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "RETURNS");
        assert_eq!(edges[0].from_label, "ToolCall");
        assert_eq!(edges[0].to_label, "Artifact");
    }

    #[test]
    fn extract_emits_returns_edge_for_typed_result() {
        let env = tool_call_with_relations(
            "01HXTC02",
            vec![entity("concept", "std::io::Result")],
            vec![rel("this_event", "RETURNS", "std::io::Result")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_key, "std::io::Result");
    }

    #[test]
    fn extract_matches_relation_label_case_insensitively() {
        for label in ["RETURNS", "Returns", "returns", "ReTuRnS"] {
            let env = tool_call_with_relations(
                "01HXTC03",
                vec![entity("concept", "crate::Out")],
                vec![rel("this_event", label, "crate::Out")],
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
        let env = tool_call_with_relations(
            "01HXTC04",
            Vec::new(),
            vec![rel("this_event", "RETURNS", "ghost::Result")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let env = tool_call_with_relations(
            "01HXTC05",
            vec![entity("concept", "crate::Bytes")],
            vec![rel("this_event", "RETURNS", "crate::Bytes")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        let props = &edges[0].props;
        assert_eq!(
            props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTC05")
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
    fn extract_skips_non_returns_relations_in_same_envelope() {
        let env = tool_call_with_relations(
            "01HXTC06",
            vec![entity("concept", "crate::Res")],
            vec![
                rel("this_event", "REFERENCES", "crate::Res"),
                rel("this_event", "RETURNS", "crate::Res"),
                rel("this_event", "CALLS", "crate::Res"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1, "only the RETURNS relation must surface");
        assert_eq!(edges[0].edge_type, "RETURNS");
    }
}
