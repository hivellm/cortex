//! Phase15b §2.11 — `MENTIONS_FILE` edge extractor.
//!
//! Surfaces file-path references from Turn envelopes so graph
//! queries can answer "every Turn that mentioned `src/lib.rs`".
//! The classifier already tags file paths with `entity_type =
//! "artifact"`; this extractor walks `classifier.entities` for
//! artifact-typed entries and emits one edge per file.
//!
//! ## Scope
//!
//! - `Kind::Turn` only — other kinds touch files through more
//!   specific edges (ToolCall via `TOUCHED` per the mapper,
//!   Artifact via identity-only `IN_REPO`, etc.).
//! - Edge shape: `Turn[event_id] -MENTIONS_FILE-> Artifact[identifier]`.
//!   Artifact natural keys follow the mapper's
//!   `repo|path|content_hash` convention; the classifier may emit
//!   just the path — the edge still lands and the downstream
//!   writer's `MERGE` against the existing Artifact node anchors
//!   on the natural key.
//! - Idempotent re-projection: same `(Turn, Artifact, MENTIONS_FILE)`
//!   triple → no-op under the unique constraint.

use cortex_core::events::Kind;

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "MENTIONS_FILE";

/// Extract `MENTIONS_FILE` edges from a [`EnrichedEvent`]. Returns
/// the empty vec for any envelope kind other than `Turn`, or for
/// Turns whose classifier-extracted entities carry no artifact.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::Turn {
        return Vec::new();
    }
    env.classifier
        .entities
        .iter()
        .filter(|e| e.entity_type == "artifact")
        .map(|e| e.identifier.trim().to_string())
        .filter(|id| !id.is_empty())
        .map(|id| {
            let edge = Edge {
                edge_type: EDGE_TYPE.to_string(),
                from_label: "Turn".to_string(),
                from_key: env.event_id.clone(),
                to_label: "Artifact".to_string(),
                to_key: id,
                props: std::collections::BTreeMap::new(),
            };
            stamp_provenance(edge, env, ctx)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use crate::classifier::ExtractedEntity;

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

    fn turn_with_entities(event_id: &str, entities: Vec<ExtractedEntity>) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Turn);
        env.classifier.entities = entities;
        env
    }

    #[test]
    fn extract_returns_empty_for_non_turn_kinds() {
        for k in [
            Kind::ToolCall,
            Kind::Decision,
            Kind::Analysis,
            Kind::Memory,
            Kind::Artifact,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(extract(&env, &ctx()).is_empty());
        }
    }

    #[test]
    fn extract_emits_one_edge_per_artifact_entity() {
        let env = turn_with_entities(
            "01HXTURN01",
            vec![
                entity("artifact", "cortex|src/lib.rs|sha256:abc"),
                entity("artifact", "cortex|src/main.rs|sha256:def"),
                entity("concept", "tokio::spawn"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 2);
        let to_keys: Vec<&str> = edges.iter().map(|e| e.to_key.as_str()).collect();
        assert!(to_keys.contains(&"cortex|src/lib.rs|sha256:abc"));
        assert!(to_keys.contains(&"cortex|src/main.rs|sha256:def"));
        for e in &edges {
            assert_eq!(e.edge_type, "MENTIONS_FILE");
            assert_eq!(e.from_label, "Turn");
            assert_eq!(e.to_label, "Artifact");
        }
    }

    #[test]
    fn extract_ignores_non_artifact_entities() {
        let env = turn_with_entities(
            "01HXTURN02",
            vec![
                entity("concept", "tokio::spawn"),
                entity("decision", "DEC-0042"),
                entity("topic", "retention"),
            ],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_skips_whitespace_only_artifact_ids() {
        let env = turn_with_entities(
            "01HXTURN03",
            vec![
                entity("artifact", "  "),
                entity("artifact", "cortex|src/x.rs|h"),
            ],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_key, "cortex|src/x.rs|h");
    }

    #[test]
    fn extract_trims_artifact_id_whitespace() {
        let env = turn_with_entities(
            "01HXTURN04",
            vec![entity("artifact", "  cortex|src/y.rs|h  ")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges[0].to_key, "cortex|src/y.rs|h");
    }

    #[test]
    fn extract_stamps_provenance_triple() {
        let env = turn_with_entities("01HXTURN05", vec![entity("artifact", "cortex|src/z.rs|h")]);
        let edges = extract(&env, &ctx());
        let p = &edges[0].props;
        assert_eq!(
            p.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTURN05")
        );
        assert_eq!(
            p.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase15b-v1")
        );
    }

    #[test]
    fn extract_returns_empty_when_entities_array_empty() {
        let env = turn_with_entities("01HXTURN06", vec![]);
        assert!(extract(&env, &ctx()).is_empty());
    }
}
