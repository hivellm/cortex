//! Phase15b §3.1 — projection pipeline.
//!
//! Runs every per-kind edge extractor over a single
//! [`EnrichedEvent`] and folds the result into a [`GraphPatch`]
//! the existing coalescer / writer chain knows how to batch to
//! Nexus. The pipeline is intentionally a pure function over
//! `(env, ctx)` so re-projecting the same envelope is a no-op
//! under Nexus's `(from, to, kind)` unique constraint — see
//! §3.2 invariant `same envelope twice → byte-identical patch`.
//!
//! ## Dispatch table
//!
//! The extractor list is a fixed `&[(name, fn)]` table; adding a
//! new extractor means appending one row + the per-extractor
//! module. The table is iterated in declaration order; the
//! `(from, to, kind)` triple is what dedupes — declaration order
//! is informational only.
//!
//! ## Mapper coexistence
//!
//! [`super::mapper::map_event_to_patch`] keeps emitting the
//! identity-only patches (`HAS_TURN` / `HAS_TOOL_CALL` /
//! `TOUCHED` / `IN_REPO` / `REMEMBERS`). The projection pipeline
//! returns ONLY the §2 semantic edges so the two streams compose
//! at the writer (the patch coalescer already merges multi-source
//! patches without conflict). Future work can fold the mapper's
//! identity path into the same dispatch table once every
//! identity edge has an extractor counterpart; not in this
//! phase.

use super::extractors::{
    about, answered_by, calls, cites, contradicts, defines, emitted_by, imports, mentions_file,
    relates_to, returns, supersedes, Edge, ExtractCtx,
};
use super::patch::GraphPatch;
use crate::embedder::EnrichedEvent;

/// Function-pointer type for an extractor (mirrors the contract
/// pinned in [`super::extractors`]).
type ExtractorFn = fn(&EnrichedEvent, &ExtractCtx) -> Vec<Edge>;

/// Dispatch table — one row per registered extractor. Adding a
/// new edge kind appends one row and the matching module
/// declaration in [`super::extractors`].
const EXTRACTORS: &[(&str, ExtractorFn)] = &[
    ("CALLS", calls::extract),
    ("IMPORTS", imports::extract),
    ("DEFINES", defines::extract),
    ("RETURNS", returns::extract),
    ("SUPERSEDES", supersedes::extract),
    ("CONTRADICTS", contradicts::extract),
    ("EMITTED_BY", emitted_by::extract),
    ("ABOUT", about::extract),
    ("ANSWERED_BY", answered_by::extract),
    ("CITES", cites::extract),
    ("MENTIONS_FILE", mentions_file::extract),
    ("RELATES_TO", relates_to::extract),
];

/// Project one [`EnrichedEvent`] into a [`GraphPatch`] carrying
/// every §2 semantic edge the registered extractors produce.
/// Returns an empty patch when no extractor matches the
/// envelope's kind / payload.
///
/// The function does NOT touch [`super::mapper`]'s identity-only
/// patch path; the writer chain merges both streams via the
/// existing coalescer.
pub fn project_envelope(env: &EnrichedEvent, ctx: &ExtractCtx) -> GraphPatch {
    let mut patch = GraphPatch::empty();
    for (_name, extractor) in EXTRACTORS {
        let edges = extractor(env, ctx);
        patch.edges.extend(edges);
    }
    patch
}

/// Iterator over the registered edge-kind names — useful for
/// the `cortex-ops doctor graph-coverage` lookup table (§4.1)
/// and for tests that assert the registry is well-formed.
pub fn registered_edge_kinds() -> impl Iterator<Item = &'static str> {
    EXTRACTORS.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::super::extractors::tests::fixture_envelope;
    use super::*;
    use crate::classifier::{ExtractedEntity, ExtractedRelation};
    use cortex_core::events::Kind;

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
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
    fn registered_edge_kinds_lists_all_twelve_kinds() {
        let kinds: Vec<&str> = registered_edge_kinds().collect();
        assert_eq!(kinds.len(), 12);
        for expected in [
            "CALLS",
            "IMPORTS",
            "DEFINES",
            "RETURNS",
            "SUPERSEDES",
            "CONTRADICTS",
            "EMITTED_BY",
            "ABOUT",
            "ANSWERED_BY",
            "CITES",
            "MENTIONS_FILE",
            "RELATES_TO",
        ] {
            assert!(
                kinds.contains(&expected),
                "missing registered kind {expected}"
            );
        }
    }

    #[test]
    fn project_envelope_returns_empty_patch_for_bare_envelope() {
        let env = fixture_envelope("01HXEVT01", Kind::Memory);
        let patch = project_envelope(&env, &ctx());
        assert!(patch.is_empty());
    }

    #[test]
    fn project_envelope_emits_edges_from_classifier_relations_on_tool_call() {
        let mut env = fixture_envelope("01HXTC01", Kind::ToolCall);
        env.classifier.entities = vec![entity("artifact", "cortex|src/lib.rs|h")];
        env.classifier.relations = vec![
            rel("this_event", "CALLS", "cortex|src/lib.rs|h"),
            rel("this_event", "IMPORTS", "cortex|src/lib.rs|h"),
        ];
        env.redacted_payload =
            serde_json::json!({"tool_name": "Read", "input": {}, "outcome": "success"});
        let patch = project_envelope(&env, &ctx());
        let kinds: std::collections::BTreeSet<&str> =
            patch.edges.iter().map(|e| e.edge_type.as_str()).collect();
        // CALLS (§2.1) + IMPORTS (§2.2) + EMITTED_BY (§2.7, Read tool) all fire.
        assert!(kinds.contains("CALLS"));
        assert!(kinds.contains("IMPORTS"));
        assert!(kinds.contains("EMITTED_BY"));
    }

    #[test]
    fn project_envelope_is_idempotent_byte_identical_on_second_call() {
        // §3.2 invariant — re-projecting the same envelope MUST
        // produce the same patch so the writer's
        // `(from, to, kind)` unique constraint catches the
        // second emission as a no-op.
        let mut env = fixture_envelope("01HXTC02", Kind::ToolCall);
        env.classifier.entities = vec![entity("artifact", "cortex|src/main.rs|h")];
        env.classifier.relations = vec![rel("this_event", "CALLS", "cortex|src/main.rs|h")];
        env.redacted_payload =
            serde_json::json!({"tool_name": "Read", "input": {}, "outcome": "success"});
        let ctx = ctx();
        let first = project_envelope(&env, &ctx);
        let second = project_envelope(&env, &ctx);
        assert_eq!(
            first.edges.len(),
            second.edges.len(),
            "edge count must match"
        );
        let first_json = serde_json::to_string(&first).unwrap();
        let second_json = serde_json::to_string(&second).unwrap();
        assert_eq!(
            first_json, second_json,
            "patch must be byte-identical on re-projection"
        );
    }

    #[test]
    fn project_envelope_decision_emits_supersedes_when_payload_supersedes_set() {
        let mut env = fixture_envelope("01HXDEC01", Kind::Decision);
        env.redacted_payload = serde_json::json!({
            "decision_id": "DEC-0042",
            "title": "Adopt phase15b extractors",
            "status": "accepted",
            "body": "stuff",
            "supersedes": "DEC-0001",
            "tags": []
        });
        let patch = project_envelope(&env, &ctx());
        let kinds: Vec<&str> = patch.edges.iter().map(|e| e.edge_type.as_str()).collect();
        assert!(kinds.contains(&"SUPERSEDES"));
    }

    #[test]
    fn project_envelope_turn_emits_about_answered_by_mentions_cites() {
        let mut env = fixture_envelope("01HXTURN01", Kind::Turn);
        env.classifier.topics = vec!["retention".into()];
        env.classifier.entities = vec![entity("artifact", "cortex|src/x.rs|h")];
        env.redacted_payload = serde_json::json!({
            "user_message": "see DEC-0042",
            "assistant_message": "implementing DEC-0042 now",
            "tool_call_event_ids": ["01HXTC_A"]
        });
        let patch = project_envelope(&env, &ctx());
        let kinds: std::collections::BTreeSet<&str> =
            patch.edges.iter().map(|e| e.edge_type.as_str()).collect();
        assert!(kinds.contains("ABOUT"));
        assert!(kinds.contains("ANSWERED_BY"));
        assert!(kinds.contains("MENTIONS_FILE"));
        assert!(kinds.contains("CITES"));
    }

    #[test]
    fn project_envelope_never_touches_nodes_only_edges() {
        // The §2 extractors emit edges only; the mapper handles
        // nodes. Pin this so a future regression that starts
        // pushing nodes into the projection pipeline fails the
        // unit test rather than silently corrupting the patch
        // contract.
        let mut env = fixture_envelope("01HXTURN02", Kind::Turn);
        env.classifier.topics = vec!["x".into()];
        env.redacted_payload = serde_json::json!({"user_message": "x", "tool_call_event_ids": []});
        let patch = project_envelope(&env, &ctx());
        assert!(patch.nodes.is_empty(), "projection MUST not emit nodes");
        assert!(!patch.edges.is_empty(), "projection MUST emit edges");
    }
}
