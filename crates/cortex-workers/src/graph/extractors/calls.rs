//! Phase15b §2.1 — `CALLS` edge extractor.
//!
//! Surfaces function-call relationships from ToolCall envelopes.
//! The classifier (Sonnet path) emits a `relations: Vec<ExtractedRelation>`
//! field on every classified event; any relation with label
//! `CALLS` (case-insensitive) becomes a graph edge here.
//!
//! ## Scope
//!
//! - Restricted to `Kind::ToolCall` envelopes per the task spec.
//!   `Kind::Turn` / `Kind::Decision` / etc. carry their own
//!   semantic edges via other extractors (§2.5 / §2.10) so a
//!   single `CALLS` relation never lands twice in the patch.
//! - The classifier's vocab list does not pin `CALLS` (it's free-
//!   form alongside the canonical `REFERENCES` / `IMPLEMENTS` /
//!   etc.); we match case-insensitively so Sonnet emitting
//!   `Calls` / `calls` / `CALLS` all converge on the same graph
//!   edge type.
//! - `from = "this_event"` resolves to `(ToolCall, event_id)`;
//!   any other `from` / `to` resolves through the envelope's
//!   `entities` table via [`super::resolve_entity_endpoint`].
//! - Relations that reference an entity not in the same record's
//!   `entities` table are dropped (the classifier contract pins
//!   the same-record invariant); the dropped edge is a no-op
//!   under the `(from, to, kind)` unique constraint so silent
//!   skips never leak phantom nodes.

use cortex_core::events::Kind;

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "CALLS";

/// The classifier's anchor literal for "the envelope itself".
const ANCHOR_THIS_EVENT: &str = "this_event";

/// Extract `CALLS` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for any envelope kind other than `ToolCall`, or for
/// a ToolCall whose classifier output carries no `CALLS` relation.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::ToolCall {
        return Vec::new();
    }
    let mut out: Vec<Edge> = Vec::new();
    for rel in &env.classifier.relations {
        if !rel.relation.eq_ignore_ascii_case(EDGE_TYPE) {
            continue;
        }
        let from = resolve_endpoint(env, rel.from.as_str());
        let to = resolve_endpoint(env, rel.to.as_str());
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let edge = Edge {
            edge_type: EDGE_TYPE.to_string(),
            from_label: from.0,
            from_key: from.1,
            to_label: to.0,
            to_key: to.1,
            props: std::collections::BTreeMap::new(),
        };
        out.push(stamp_provenance(edge, env, ctx));
    }
    out
}

/// Resolve a relation endpoint string to `(label, natural_key)`.
/// `"this_event"` anchors at the envelope; any other identifier
/// resolves through the classifier's `entities` table. Returns
/// `None` when the identifier is unknown so the caller drops the
/// edge rather than fabricating a phantom node.
pub(crate) fn resolve_endpoint(env: &EnrichedEvent, identifier: &str) -> Option<(String, String)> {
    if identifier == ANCHOR_THIS_EVENT {
        return Some(("ToolCall".to_string(), env.event_id.clone()));
    }
    let entity = env
        .classifier
        .entities
        .iter()
        .find(|e| e.identifier == identifier)?;
    let label = entity_type_to_label(&entity.entity_type)?;
    Some((label, entity.identifier.clone()))
}

/// Map the classifier's `entity_type` controlled vocab to the
/// graph node label. Unknown entity types return `None` so the
/// extractor drops the edge rather than guessing.
pub(crate) fn entity_type_to_label(entity_type: &str) -> Option<String> {
    let label = match entity_type {
        "decision" => "Decision",
        "law" => "Law",
        "analysis" => "Analysis",
        "artifact" => "Artifact",
        "repo" => "Repo",
        "topic" => "Topic",
        "concept" => "Concept",
        "tool" => "Tool",
        "person" => "Person",
        "session" => "Session",
        "turn" => "Turn",
        _ => return None,
    };
    Some(label.to_string())
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
            Kind::LawViolation,
            Kind::Memory,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no CALLS edges"
            );
        }
    }

    #[test]
    fn extract_emits_one_edge_per_calls_relation() {
        let env = tool_call_with_relations(
            "01HXTC01",
            vec![entity("artifact", "cortex|src/lib.rs|sha256:abc")],
            vec![rel("this_event", "CALLS", "cortex|src/lib.rs|sha256:abc")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "CALLS");
        assert_eq!(edges[0].from_label, "ToolCall");
        assert_eq!(edges[0].from_key, "01HXTC01");
        assert_eq!(edges[0].to_label, "Artifact");
        assert_eq!(edges[0].to_key, "cortex|src/lib.rs|sha256:abc");
    }

    #[test]
    fn extract_matches_relation_label_case_insensitively() {
        for label in ["CALLS", "Calls", "calls", "CaLlS"] {
            let env = tool_call_with_relations(
                "01HXTC02",
                vec![entity("artifact", "cortex|x|h")],
                vec![rel("this_event", label, "cortex|x|h")],
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
            "01HXTC03",
            // No entity table — the `to` cannot resolve.
            Vec::new(),
            vec![rel("this_event", "CALLS", "phantom-id")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_drops_relation_with_unknown_entity_type() {
        let env = tool_call_with_relations(
            "01HXTC04",
            vec![entity("unknown_kind", "weird-id")],
            vec![rel("this_event", "CALLS", "weird-id")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let env = tool_call_with_relations(
            "01HXTC05",
            vec![entity("artifact", "cortex|src/main.rs|h")],
            vec![rel("this_event", "CALLS", "cortex|src/main.rs|h")],
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
    fn extract_skips_non_calls_relations_in_same_envelope() {
        let env = tool_call_with_relations(
            "01HXTC06",
            vec![entity("artifact", "cortex|x|h")],
            vec![
                rel("this_event", "REFERENCES", "cortex|x|h"),
                rel("this_event", "CALLS", "cortex|x|h"),
                rel("this_event", "DEFINES", "cortex|x|h"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1, "only the CALLS relation must surface");
        assert_eq!(edges[0].edge_type, "CALLS");
    }
}
