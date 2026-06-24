//! Phase15b §2.9 — `ANSWERED_BY` edge extractor.
//!
//! Surfaces the Turn → ToolCall back-reference that
//! `cortex_core::events::Turn::tool_call_event_ids` already
//! carries. The mapper today emits the inverse `HAS_TOOL_CALL`
//! linkage (parent→child); promoting `ANSWERED_BY` lets graph
//! queries pivot from a Turn to every tool-call that answered it
//! without re-deriving the back-reference.
//!
//! ## Scope
//!
//! - `Kind::Turn` only — payload-derived from the
//!   `Turn::tool_call_event_ids` vec.
//! - Edge shape: `Turn[event_id] -ANSWERED_BY-> ToolCall[child_event_id]`.
//! - Empty / malformed payloads drop silently.

use cortex_core::events::{Kind, Turn as TurnPayload};

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "ANSWERED_BY";

/// Extract `ANSWERED_BY` edges from a [`EnrichedEvent`]. Returns
/// the empty vec for any envelope kind other than `Turn`, or for
/// Turns whose `tool_call_event_ids` is empty.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::Turn {
        return Vec::new();
    }
    let payload: TurnPayload = match serde_json::from_value(env.redacted_payload.clone()) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    payload
        .tool_call_event_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .map(|tc_id| {
            let edge = Edge {
                edge_type: EDGE_TYPE.to_string(),
                from_label: "Turn".to_string(),
                from_key: env.event_id.clone(),
                to_label: "ToolCall".to_string(),
                to_key: tc_id,
                props: std::collections::BTreeMap::new(),
                ..Default::default()
            };
            stamp_provenance(edge, env, ctx)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use serde_json::json;

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn turn_envelope(event_id: &str, tool_call_ids: Vec<&str>) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Turn);
        env.redacted_payload = json!({
            "user_message": "hi",
            "assistant_message": "yes",
            "tool_call_event_ids": tool_call_ids
        });
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
    fn extract_returns_empty_when_tool_call_ids_empty() {
        let env = turn_envelope("01HXTURN01", vec![]);
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_emits_one_edge_per_tool_call_id() {
        let env = turn_envelope("01HXTURN02", vec!["01HXTC01", "01HXTC02"]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].edge_type, "ANSWERED_BY");
        assert_eq!(edges[0].from_label, "Turn");
        assert_eq!(edges[0].from_key, "01HXTURN02");
        assert_eq!(edges[0].to_label, "ToolCall");
        let to_keys: Vec<&str> = edges.iter().map(|e| e.to_key.as_str()).collect();
        assert!(to_keys.contains(&"01HXTC01"));
        assert!(to_keys.contains(&"01HXTC02"));
    }

    #[test]
    fn extract_returns_empty_when_payload_is_malformed() {
        let mut env = fixture_envelope("01HXTURN03", Kind::Turn);
        env.redacted_payload = json!({"junk": true});
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_drops_whitespace_only_tool_call_ids() {
        let env = turn_envelope("01HXTURN04", vec!["", "  ", "01HXTC03"]);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_key, "01HXTC03");
    }

    #[test]
    fn extract_stamps_provenance_triple() {
        let env = turn_envelope("01HXTURN05", vec!["01HXTC04"]);
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
}
