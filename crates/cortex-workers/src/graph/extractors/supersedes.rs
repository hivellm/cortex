//! Phase15b §2.5 — `SUPERSEDES` edge extractor.
//!
//! Promotes decision supersession from a property buried in the
//! mapper's identity-only patch (`DecisionPayload.supersedes`) to
//! a first-class graph edge `Decision[new] → Decision[old]`. The
//! supersession chain is what `cortex_decision_chain` already
//! walks via Cypher — surfacing it as a typed edge lets every
//! graph-lane query traverse the chain without parsing payload
//! JSON.
//!
//! ## Scope
//!
//! - `Kind::Decision` only — the payload-derived flavour. The
//!   classifier-vocab `SUPERSEDES` label (canonical per
//!   `crates/cortex-workers/src/classifier/types.rs::ExtractedRelation`
//!   "decision-level supersession") is handled by the
//!   classifier-relations path on Turn / Analysis envelopes that
//!   discuss the chain — those land via the §2.10 `CITES`
//!   extractor when they reference a decision by id. Keeping the
//!   payload-driven path here ensures every emitted Decision
//!   contributes its supersession edge even when the classifier
//!   didn't run (static-fallback path) or failed.
//! - Edge shape: `Decision(new) -SUPERSEDES-> Decision(old)`.
//!   The `new` natural key resolves to the envelope's own
//!   `decision_id` (NOT `event_id` — the producer-stable id is
//!   what the mapper writes to the node). The `old` key is
//!   `DecisionPayload.supersedes`. A Decision whose `supersedes`
//!   is `None` returns the empty vec.
//! - Idempotent re-projection: the same `(from, to, kind)`
//!   triple hits the unique constraint as a no-op.

use cortex_core::events::{DecisionPayload, Kind};

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "SUPERSEDES";

/// Extract `SUPERSEDES` edges from a [`EnrichedEvent`]. Returns
/// the empty vec for any envelope kind other than `Decision`, or
/// for a Decision whose `supersedes` field is `None`, or for a
/// payload whose JSON shape cannot be deserialised (the mapper's
/// identity-only path still lands so the node exists; the edge
/// is the only loss).
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::Decision {
        return Vec::new();
    }
    let payload: DecisionPayload = match serde_json::from_value(env.redacted_payload.clone()) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let Some(superseded) = payload
        .supersedes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Vec::new();
    };
    let edge = Edge {
        edge_type: EDGE_TYPE.to_string(),
        from_label: "Decision".to_string(),
        from_key: payload.decision_id,
        to_label: "Decision".to_string(),
        to_key: superseded,
        props: std::collections::BTreeMap::new(),
    };
    vec![stamp_provenance(edge, env, ctx)]
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use serde_json::json;

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn decision_envelope(
        event_id: &str,
        decision_id: &str,
        supersedes: Option<&str>,
    ) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Decision);
        env.redacted_payload = json!({
            "decision_id": decision_id,
            "title": "Adopt phase15b extractors",
            "status": "accepted",
            "body": "stuff",
            "supersedes": supersedes,
            "tags": []
        });
        env
    }

    #[test]
    fn extract_returns_empty_for_non_decision_kinds() {
        for k in [
            Kind::Turn,
            Kind::ToolCall,
            Kind::Analysis,
            Kind::Memory,
            Kind::LawViolation,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no SUPERSEDES edges"
            );
        }
    }

    #[test]
    fn extract_returns_empty_when_supersedes_is_none() {
        let env = decision_envelope("01HXDEC01", "DEC-0042", None);
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_returns_empty_when_supersedes_is_whitespace() {
        let env = decision_envelope("01HXDEC02", "DEC-0042", Some("   "));
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_emits_decision_to_decision_edge_when_supersedes_set() {
        let env = decision_envelope("01HXDEC03", "DEC-0099", Some("DEC-0001"));
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "SUPERSEDES");
        assert_eq!(edges[0].from_label, "Decision");
        assert_eq!(edges[0].from_key, "DEC-0099");
        assert_eq!(edges[0].to_label, "Decision");
        assert_eq!(edges[0].to_key, "DEC-0001");
    }

    #[test]
    fn extract_stamps_provenance_triple() {
        let env = decision_envelope("01HXDEC04", "DEC-0050", Some("DEC-0001"));
        let edges = extract(&env, &ctx());
        let props = &edges[0].props;
        assert_eq!(
            props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXDEC04")
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
    fn extract_returns_empty_when_payload_shape_is_invalid() {
        let mut env = fixture_envelope("01HXDEC05", Kind::Decision);
        // Missing required fields → DecisionPayload::deserialize fails.
        env.redacted_payload = json!({"junk": true});
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_uses_producer_stable_decision_id_not_event_id_for_from() {
        // The producer's `decision_id` is stable across re-emits;
        // `event_id` changes every time the envelope re-emits.
        // The edge MUST anchor on `decision_id` so re-emission is
        // a no-op under the (from, to, kind) constraint.
        let env = decision_envelope("01HXDEC06_FIRST_EMIT", "DEC-0100", Some("DEC-0010"));
        let edges = extract(&env, &ctx());
        assert_eq!(edges[0].from_key, "DEC-0100");
        assert_ne!(edges[0].from_key, "01HXDEC06_FIRST_EMIT");
    }
}
