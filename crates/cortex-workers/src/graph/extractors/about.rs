//! Phase15b §2.8 — `ABOUT` edge extractor.
//!
//! Surfaces topic linkage from Turn and Decision envelopes — both
//! kinds carry `classifier.topics` (multi-label vocab the
//! classifier stamps) so we project one edge per topic so graph
//! queries can ask "every Turn discussing topic X" or "every
//! Decision tagged X" without scanning the payload bodies.
//!
//! ## Scope
//!
//! - `Kind::Turn` + `Kind::Decision` only — other kinds also
//!   carry topics but their semantic anchor is more specific
//!   (TopicCard has its own `topic_slug`, Consolidation has
//!   `topics` already projected into the Meili index, etc.).
//! - Topic label maps to graph node `Topic[topic-slug]` so the
//!   `cortex_topic_neighbors` MCP walk can pivot from the topic
//!   node into every connected envelope.
//! - Empty / whitespace-only topic strings drop silently.

use cortex_core::events::Kind;

use super::{kind_to_label, stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "ABOUT";

/// Extract `ABOUT` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for any envelope kind other than `Turn` or
/// `Decision`, or for envelopes whose classifier output carries
/// no topics.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if !matches!(env.kind, Kind::Turn | Kind::Decision) {
        return Vec::new();
    }
    let from_label = kind_to_label(env.kind).to_string();
    env.classifier
        .topics
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|topic| {
            let edge = Edge {
                edge_type: EDGE_TYPE.to_string(),
                from_label: from_label.clone(),
                from_key: env.event_id.clone(),
                to_label: "Topic".to_string(),
                to_key: topic,
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

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn with_topics(event_id: &str, kind: Kind, topics: Vec<&str>) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, kind);
        env.classifier.topics = topics.into_iter().map(String::from).collect();
        env
    }

    #[test]
    fn extract_returns_empty_for_kinds_outside_turn_or_decision() {
        for k in [
            Kind::ToolCall,
            Kind::Memory,
            Kind::Analysis,
            Kind::Consolidation,
            Kind::TopicCard,
        ] {
            let env = with_topics("01HXEVT01", k, vec!["topic-a"]);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no ABOUT edges"
            );
        }
    }

    #[test]
    fn extract_emits_one_edge_per_topic_on_turn() {
        let env = with_topics("01HXTURN01", Kind::Turn, vec!["retention", "phase15b"]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 2);
        let topics: Vec<&str> = edges.iter().map(|e| e.to_key.as_str()).collect();
        assert!(topics.contains(&"retention"));
        assert!(topics.contains(&"phase15b"));
        assert_eq!(edges[0].from_label, "Turn");
        assert_eq!(edges[0].to_label, "Topic");
    }

    #[test]
    fn extract_emits_edges_for_decision_kind() {
        let env = with_topics("01HXDEC01", Kind::Decision, vec!["infra"]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_label, "Decision");
    }

    #[test]
    fn extract_skips_whitespace_only_topics() {
        let env = with_topics("01HXTURN02", Kind::Turn, vec!["", "   ", "real"]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_key, "real");
    }

    #[test]
    fn extract_trims_topic_whitespace() {
        let env = with_topics("01HXTURN03", Kind::Turn, vec!["  retention  "]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges[0].to_key, "retention");
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let env = with_topics("01HXTURN04", Kind::Turn, vec!["x"]);
        let edges = extract(&env, &ctx());
        let p = &edges[0].props;
        assert_eq!(
            p.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTURN04")
        );
        assert_eq!(
            p.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase15b-v1")
        );
    }

    #[test]
    fn extract_returns_empty_when_topics_array_is_empty() {
        let env = with_topics("01HXTURN05", Kind::Turn, vec![]);
        assert!(extract(&env, &ctx()).is_empty());
    }
}
