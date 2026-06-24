//! Phase15b §2.12 — `RELATES_TO` edge extractor.
//!
//! Fallback semantic-similarity edge between Turn envelopes. The
//! task spec calls for `cosine ≥ 0.85` between Turn embeddings,
//! but [`EnrichedEvent`] does not carry the embedding vector (the
//! embedder writes it to Vectorizer side-by-side with the
//! envelope but the worker pipeline that hosts this extractor
//! does not have the result handy). Two pragmatic surface
//! options coexist behind this extractor:
//!
//! 1. **Classifier-relations RELATES_TO label** — Sonnet emits a
//!    `RELATES_TO` relation when it spots a semantic link the
//!    other extractors missed (e.g. two Turn envelopes both
//!    discussing the same concept without explicit citation).
//!    This is the primary signal today.
//! 2. **Future cosine pass** — a follow-up commit will wire the
//!    Vectorizer KNN result into `ExtractCtx` so a Turn whose
//!    nearest-neighbour scores `>= 0.85` gets an edge to that
//!    neighbour. Until that lands the cosine path is dormant;
//!    the extractor only emits classifier-driven edges.
//!
//! ## Scope
//!
//! - `Kind::Turn` only — the cosine fallback is Turn-on-Turn per
//!   the spec; Analysis / Decision / etc. have their own
//!   semantic edges via §2.6 / §2.10 / §2.8.
//! - Edge shape: `Turn[event_id] -RELATES_TO-> <TargetLabel>[id]`.
//!   The shared resolver maps the classifier's `to` identifier
//!   through the entity table; Turn-to-Turn semantic links land
//!   when Sonnet emits a `turn`-typed entity for the related
//!   Turn id.
//! - Edge prop `source` = `"classifier"` today so a future
//!   `"cosine"` source coexists under the unique constraint
//!   (different `(source)` does NOT split the dedupe; the
//!   `(from, to, kind)` triple is the constraint key, the prop
//!   is informational).

use cortex_core::events::Kind;
use serde_json::Value;

use super::{resolve_endpoint, stamp_provenance, Edge, ExtractCtx, ANCHOR_THIS_EVENT};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "RELATES_TO";

/// Extract `RELATES_TO` edges from a [`EnrichedEvent`]. Returns
/// the empty vec for any envelope kind other than `Turn` or for
/// Turns whose classifier output carries no `RELATES_TO` relation.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::Turn {
        return Vec::new();
    }
    let mut out: Vec<Edge> = Vec::new();
    for rel in &env.classifier.relations {
        if !rel.relation.eq_ignore_ascii_case(EDGE_TYPE) {
            continue;
        }
        if rel.from != ANCHOR_THIS_EVENT {
            continue;
        }
        let Some((to_label, to_key)) = resolve_endpoint(env, rel.to.as_str()) else {
            continue;
        };
        let mut props = std::collections::BTreeMap::new();
        props.insert(
            "source".to_string(),
            Value::String("classifier".to_string()),
        );
        let edge = Edge {
            edge_type: EDGE_TYPE.to_string(),
            from_label: "Turn".to_string(),
            from_key: env.event_id.clone(),
            to_label,
            to_key,
            props,
            ..Default::default()
        };
        out.push(stamp_provenance(edge, env, ctx));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use crate::classifier::{ExtractedEntity, ExtractedRelation};

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

    fn turn_with(
        event_id: &str,
        entities: Vec<ExtractedEntity>,
        relations: Vec<ExtractedRelation>,
    ) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Turn);
        env.classifier.entities = entities;
        env.classifier.relations = relations;
        env
    }

    #[test]
    fn extract_returns_empty_for_non_turn_kinds() {
        for k in [
            Kind::ToolCall,
            Kind::Decision,
            Kind::Analysis,
            Kind::Memory,
            Kind::TopicCard,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(extract(&env, &ctx()).is_empty());
        }
    }

    #[test]
    fn extract_emits_turn_to_turn_edge_from_classifier_relation() {
        let env = turn_with(
            "01HXTURN01",
            vec![entity("turn", "01HXTURN02")],
            vec![rel("this_event", "RELATES_TO", "01HXTURN02")],
        );
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "RELATES_TO");
        assert_eq!(edges[0].from_label, "Turn");
        assert_eq!(edges[0].from_key, "01HXTURN01");
        assert_eq!(edges[0].to_label, "Turn");
        assert_eq!(edges[0].to_key, "01HXTURN02");
        assert_eq!(
            edges[0].props.get("source").and_then(|v| v.as_str()),
            Some("classifier")
        );
    }

    #[test]
    fn extract_matches_relation_label_case_insensitively() {
        for label in ["RELATES_TO", "relates_to", "Relates_To", "RElaTES_to"] {
            let env = turn_with(
                "01HXTURN02",
                vec![entity("turn", "01HXTURN99")],
                vec![rel("this_event", label, "01HXTURN99")],
            );
            assert_eq!(
                extract(&env, &ctx()).len(),
                1,
                "relation `{label}` must match"
            );
        }
    }

    #[test]
    fn extract_drops_relation_with_unknown_endpoint() {
        let env = turn_with(
            "01HXTURN03",
            Vec::new(),
            vec![rel("this_event", "RELATES_TO", "phantom-turn")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_ignores_non_relates_to_relations() {
        let env = turn_with(
            "01HXTURN04",
            vec![entity("turn", "01HXTURN99")],
            vec![
                rel("this_event", "REFERENCES", "01HXTURN99"),
                rel("this_event", "CALLS", "01HXTURN99"),
            ],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_and_source_prop() {
        let env = turn_with(
            "01HXTURN05",
            vec![entity("turn", "01HXTURN99")],
            vec![rel("this_event", "RELATES_TO", "01HXTURN99")],
        );
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
        assert_eq!(p.get("source").and_then(|v| v.as_str()), Some("classifier"));
    }

    #[test]
    fn extract_ignores_relations_with_non_this_event_from() {
        let env = turn_with(
            "01HXTURN06",
            vec![entity("turn", "01HXTURN_A"), entity("turn", "01HXTURN_B")],
            vec![rel("01HXTURN_A", "RELATES_TO", "01HXTURN_B")],
        );
        assert!(extract(&env, &ctx()).is_empty());
    }
}
