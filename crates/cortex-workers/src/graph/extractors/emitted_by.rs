//! Phase15b §2.7 — `EMITTED_BY` edge extractor.
//!
//! Surfaces a producer node per envelope so the graph carries the
//! "who created this" backbone the spec calls for. The producer is
//! payload-derived rather than envelope-derived because the
//! [`EnrichedEvent`] surface does not carry the canonical
//! `Envelope.tool` adapter identifier — it strips that field during
//! the embedder enrichment step. Surfacing the producer from the
//! payload is sufficient for every kind we care about today:
//!
//! - `Kind::ToolCall` → `Tool[tool_name]`
//! - `Kind::Consolidation` → `Model[model]`
//! - `Kind::TopicCard` → `Model[synthesis_model]`
//!
//! Other kinds (Turn / Decision / Analysis / Memory / Knowledge /
//! Learning / LawViolation / AgentCall / Artifact) don't carry a
//! payload-level producer field; they surface only via the
//! envelope's adapter identity which the EnrichedEvent strips.
//! Adding `tool` to EnrichedEvent (closing the gap for every kind)
//! is a wider refactor that the proposal documents as a future
//! follow-up — it's not in the scope of this extractor commit.
//!
//! ## Edge shape
//!
//! `<KindLabel>[event_id] -EMITTED_BY-> <ProducerLabel>[id]` with
//! a `producer_kind` property pinning the discriminator (`tool` /
//! `model`) so dashboards can group without re-deriving the label.

use std::collections::BTreeMap;

use cortex_core::events::{Kind, ToolCall as ToolCallPayload};
use serde_json::Value;

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "EMITTED_BY";

/// Extract one or zero `EMITTED_BY` edges from the envelope.
/// Returns the empty vec when the envelope's kind does not carry
/// a payload-level producer, or when the producer field is
/// missing / empty.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    let (producer_label, producer_key, producer_kind) = match env.kind {
        Kind::ToolCall => match producer_from_tool_call(&env.redacted_payload) {
            Some(name) => ("Tool", name, "tool"),
            None => return Vec::new(),
        },
        Kind::Consolidation => match producer_from_typed_field(&env.redacted_payload, "model") {
            Some(model) => ("Model", model, "model"),
            None => return Vec::new(),
        },
        Kind::TopicCard => {
            match producer_from_typed_field(&env.redacted_payload, "synthesis_model") {
                Some(model) => ("Model", model, "model"),
                None => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };

    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    props.insert(
        "producer_kind".to_string(),
        Value::String(producer_kind.to_string()),
    );
    let edge = Edge {
        edge_type: EDGE_TYPE.to_string(),
        from_label: super::kind_to_label(env.kind).to_string(),
        from_key: env.event_id.clone(),
        to_label: producer_label.to_string(),
        to_key: producer_key,
        props,
    };
    vec![stamp_provenance(edge, env, ctx)]
}

/// Deserialise the ToolCall payload and pull `tool_name`.
fn producer_from_tool_call(payload: &Value) -> Option<String> {
    let tc: ToolCallPayload = serde_json::from_value(payload.clone()).ok()?;
    let trimmed = tc.tool_name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Read a non-empty trimmed string from a top-level payload field.
/// Used for the consolidation / topic-card variants where the
/// payload IS the typed struct but we only need one field.
fn producer_from_typed_field(payload: &Value, field: &str) -> Option<String> {
    let raw = payload.get(field)?.as_str()?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use serde_json::json;

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn tool_call_envelope(event_id: &str, tool_name: &str) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::ToolCall);
        env.redacted_payload = json!({
            "tool_name": tool_name,
            "input": {},
            "outcome": "success"
        });
        env
    }

    fn consolidation_envelope(event_id: &str, model: &str) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Consolidation);
        env.redacted_payload = json!({
            "consolidation_id": "cons-x",
            "grain": "session",
            "scope": {"session_id": "01HSESS"},
            "title": "t",
            "summary_markdown": "x".repeat(250),
            "source_event_count": 1,
            "model": model,
            "depth": "shallow",
            "temporal_span": {"start": "2026-05-26T00:00:00Z", "end": "2026-05-26T00:01:00Z"}
        });
        env
    }

    fn topic_card_envelope(event_id: &str, synthesis_model: &str) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::TopicCard);
        env.redacted_payload = json!({
            "topic_card_id": "topic-x",
            "topic_slug": "x",
            "repos": ["cortex"],
            "revision": 1,
            "synthesis_markdown": "x".repeat(250),
            "evidence": [{"kind": "turn", "id": "01HXTURN", "cited_at_rev": 1}],
            "confidence": 0.8,
            "last_rev_at": "2026-05-26T22:00:00Z",
            "events_since_last_rev": 0,
            "synthesis_model": synthesis_model,
            "synthesis_cost_cents": 0
        });
        env
    }

    #[test]
    fn extract_returns_empty_for_kinds_without_payload_producer() {
        for k in [
            Kind::Turn,
            Kind::Decision,
            Kind::Analysis,
            Kind::Memory,
            Kind::Knowledge,
            Kind::Learning,
            Kind::Artifact,
            Kind::AgentCall,
            Kind::LawViolation,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} has no payload-level producer field"
            );
        }
    }

    #[test]
    fn extract_emits_tool_call_to_tool_edge() {
        let env = tool_call_envelope("01HXTC01", "Bash");
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "EMITTED_BY");
        assert_eq!(edges[0].from_label, "ToolCall");
        assert_eq!(edges[0].from_key, "01HXTC01");
        assert_eq!(edges[0].to_label, "Tool");
        assert_eq!(edges[0].to_key, "Bash");
        assert_eq!(
            edges[0].props.get("producer_kind").and_then(|v| v.as_str()),
            Some("tool")
        );
    }

    #[test]
    fn extract_emits_consolidation_to_model_edge() {
        let env = consolidation_envelope("01HXCONS01", "claude-haiku-4-5");
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_label, "Consolidation");
        assert_eq!(edges[0].to_label, "Model");
        assert_eq!(edges[0].to_key, "claude-haiku-4-5");
        assert_eq!(
            edges[0].props.get("producer_kind").and_then(|v| v.as_str()),
            Some("model")
        );
    }

    #[test]
    fn extract_emits_topic_card_to_model_edge() {
        let env = topic_card_envelope("01HXTOPIC01", "claude-opus-4-7");
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_label, "TopicCard");
        assert_eq!(edges[0].to_label, "Model");
        assert_eq!(edges[0].to_key, "claude-opus-4-7");
    }

    #[test]
    fn extract_returns_empty_when_tool_call_tool_name_is_empty() {
        let env = tool_call_envelope("01HXTC02", "");
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_returns_empty_when_consolidation_payload_is_malformed() {
        let mut env = fixture_envelope("01HXCONS02", Kind::Consolidation);
        env.redacted_payload = json!({"junk": true});
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_emitted_edge() {
        let env = tool_call_envelope("01HXTC03", "Read");
        let edges = extract(&env, &ctx());
        let props = &edges[0].props;
        assert_eq!(
            props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTC03")
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
    fn extract_trims_whitespace_from_producer_key() {
        let env = tool_call_envelope("01HXTC04", "  Bash  ");
        let edges = extract(&env, &ctx());
        assert_eq!(edges[0].to_key, "Bash");
    }
}
