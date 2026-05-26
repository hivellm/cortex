//! Phase15b §2.6 — `CONTRADICTS` edge extractor.
//!
//! Surfaces the per-TopicCard contradictions detected by the
//! phase14 synthesiser as first-class graph edges between the two
//! contradicting evidence items. Today the dashboard renders the
//! contradictions inline on the topic card; promoting them to
//! edges lets graph-lane queries traverse "what else contradicts
//! Decision X" without parsing payload JSON.
//!
//! ## Scope
//!
//! - `Kind::TopicCard` only — payload-derived from
//!   [`cortex_core::events::TopicCardPayload::contradictions`].
//! - Edge shape:
//!   `Evidence[a] -CONTRADICTS-> Evidence[b]`. Endpoint labels
//!   come from the matching
//!   [`cortex_core::events::EvidenceRef::kind`] in the same
//!   payload's `evidence` array
//!   (`Consolidation` / `Decision` / `Law` / `Turn`). When
//!   evidence_a or evidence_b cannot be resolved (the §1.5
//!   payload validator should catch this; we double-check
//!   defensively) the edge is dropped silently.
//! - Status filter: only `ContradictionStatus::Open` contradictions
//!   land. `Reconciled` and `Deprecated` lifecycle states have
//!   already been resolved or stale; surfacing them would
//!   pollute graph-lane recall with closed work.
//! - Properties: edge carries `kind` (the
//!   [`cortex_core::events::ContradictionKind`] discriminator as
//!   snake_case string), `surfaced_at_rev`, and the canonical
//!   provenance triple.
//! - Idempotent re-projection: re-emitting the same TopicCard
//!   revision lands on the same `(evidence_a, evidence_b, kind)`
//!   triple → no-op under the unique constraint.

use std::collections::BTreeMap;

use cortex_core::events::{
    Contradiction, ContradictionStatus, EvidenceKind, EvidenceRef, Kind, TopicCardPayload,
};
use serde_json::Value;

use super::{stamp_provenance, Edge, ExtractCtx};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "CONTRADICTS";

/// Map [`EvidenceKind`] to the matching graph node label.
fn evidence_kind_to_label(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Consolidation => "Consolidation",
        EvidenceKind::Decision => "Decision",
        EvidenceKind::Law => "Law",
        EvidenceKind::Turn => "Turn",
    }
}

/// Map [`cortex_core::events::ContradictionKind`] to a stable
/// snake_case string for the edge's `kind` property.
fn contradiction_kind_label(kind: cortex_core::events::ContradictionKind) -> &'static str {
    use cortex_core::events::ContradictionKind;
    match kind {
        ContradictionKind::DecisionSupersession => "decision_supersession",
        ContradictionKind::LawViolationMismatch => "law_violation_mismatch",
        ContradictionKind::OutcomeDivergence => "outcome_divergence",
    }
}

/// Resolve an evidence id against the payload's `evidence` array.
/// Returns `None` when the id is not present so the caller drops
/// the edge rather than fabricating a phantom node.
fn lookup_evidence<'a>(evidence: &'a [EvidenceRef], id: &str) -> Option<&'a EvidenceRef> {
    evidence.iter().find(|e| e.id == id)
}

/// Build one [`Edge`] from a [`Contradiction`] when both endpoints
/// resolve in the payload's evidence table. Returns `None` for
/// statuses other than `Open` or when either endpoint is missing.
fn contradiction_to_edge(c: &Contradiction, evidence: &[EvidenceRef]) -> Option<Edge> {
    if c.status != ContradictionStatus::Open {
        return None;
    }
    let a = lookup_evidence(evidence, &c.evidence_a)?;
    let b = lookup_evidence(evidence, &c.evidence_b)?;
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    props.insert(
        "kind".to_string(),
        Value::String(contradiction_kind_label(c.kind).to_string()),
    );
    props.insert(
        "surfaced_at_rev".to_string(),
        Value::from(c.surfaced_at_rev),
    );
    Some(Edge {
        edge_type: EDGE_TYPE.to_string(),
        from_label: evidence_kind_to_label(a.kind).to_string(),
        from_key: a.id.clone(),
        to_label: evidence_kind_to_label(b.kind).to_string(),
        to_key: b.id.clone(),
        props,
    })
}

/// Extract `CONTRADICTS` edges from a [`EnrichedEvent`]. Returns
/// the empty vec for any envelope kind other than `TopicCard`,
/// for malformed payloads, or for TopicCards whose
/// `contradictions` array is empty / all-non-Open.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if env.kind != Kind::TopicCard {
        return Vec::new();
    }
    let payload: TopicCardPayload = match serde_json::from_value(env.redacted_payload.clone()) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    payload
        .contradictions
        .iter()
        .filter_map(|c| contradiction_to_edge(c, &payload.evidence))
        .map(|edge| stamp_provenance(edge, env, ctx))
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

    fn topic_card_envelope(event_id: &str, payload: Value) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::TopicCard);
        env.redacted_payload = payload;
        env
    }

    fn default_payload() -> Value {
        json!({
            "topic_card_id": "topic-test",
            "topic_slug": "auth-rewrite",
            "repos": ["cortex"],
            "revision": 1,
            "synthesis_markdown": "x".repeat(250),
            "evidence": [
                {"kind": "decision", "id": "DEC-0001", "cited_at_rev": 1},
                {"kind": "decision", "id": "DEC-0002", "cited_at_rev": 1},
                {"kind": "turn",     "id": "01HXTURN001", "cited_at_rev": 1}
            ],
            "contradictions": [],
            "confidence": 0.8,
            "last_rev_at": "2026-05-26T22:00:00Z",
            "events_since_last_rev": 0,
            "synthesis_model": "claude-haiku-4-5",
            "synthesis_cost_cents": 0
        })
    }

    #[test]
    fn extract_returns_empty_for_non_topic_card_kinds() {
        for k in [
            Kind::Turn,
            Kind::ToolCall,
            Kind::Decision,
            Kind::Memory,
            Kind::Consolidation,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(
                extract(&env, &ctx()).is_empty(),
                "kind {k:?} must yield no CONTRADICTS edges"
            );
        }
    }

    #[test]
    fn extract_returns_empty_when_payload_has_no_contradictions() {
        let env = topic_card_envelope("01HXTC01", default_payload());
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_returns_empty_when_payload_shape_is_invalid() {
        let env = topic_card_envelope("01HXTC02", json!({"junk": true}));
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_emits_edge_for_open_contradiction_between_two_decisions() {
        let mut payload = default_payload();
        payload["contradictions"] = json!([{
            "kind": "decision_supersession",
            "evidence_a": "DEC-0001",
            "evidence_b": "DEC-0002",
            "surfaced_at_rev": 3,
            "status": "open"
        }]);
        let env = topic_card_envelope("01HXTC03", payload);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "CONTRADICTS");
        assert_eq!(edges[0].from_label, "Decision");
        assert_eq!(edges[0].from_key, "DEC-0001");
        assert_eq!(edges[0].to_label, "Decision");
        assert_eq!(edges[0].to_key, "DEC-0002");
        assert_eq!(
            edges[0].props.get("kind").and_then(|v| v.as_str()),
            Some("decision_supersession")
        );
        assert_eq!(
            edges[0]
                .props
                .get("surfaced_at_rev")
                .and_then(|v| v.as_u64()),
            Some(3)
        );
    }

    #[test]
    fn extract_skips_reconciled_and_deprecated_contradictions() {
        let mut payload = default_payload();
        payload["contradictions"] = json!([
            {
                "kind": "decision_supersession",
                "evidence_a": "DEC-0001",
                "evidence_b": "DEC-0002",
                "surfaced_at_rev": 2,
                "status": "reconciled"
            },
            {
                "kind": "outcome_divergence",
                "evidence_a": "DEC-0001",
                "evidence_b": "01HXTURN001",
                "surfaced_at_rev": 2,
                "status": "deprecated"
            }
        ]);
        let env = topic_card_envelope("01HXTC04", payload);
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_drops_contradiction_with_unknown_evidence_id() {
        let mut payload = default_payload();
        payload["contradictions"] = json!([{
            "kind": "decision_supersession",
            "evidence_a": "DEC-0001",
            "evidence_b": "DEC-UNKNOWN",
            "surfaced_at_rev": 1,
            "status": "open"
        }]);
        let env = topic_card_envelope("01HXTC05", payload);
        assert!(extract(&env, &ctx()).is_empty());
    }

    #[test]
    fn extract_stamps_provenance_triple_on_every_edge() {
        let mut payload = default_payload();
        payload["contradictions"] = json!([{
            "kind": "outcome_divergence",
            "evidence_a": "DEC-0001",
            "evidence_b": "01HXTURN001",
            "surfaced_at_rev": 4,
            "status": "open"
        }]);
        let env = topic_card_envelope("01HXTC06", payload);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        let props = &edges[0].props;
        assert_eq!(
            props.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTC06")
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
    fn extract_emits_multiple_edges_when_payload_has_multiple_open_contradictions() {
        let mut payload = default_payload();
        payload["contradictions"] = json!([
            {
                "kind": "decision_supersession",
                "evidence_a": "DEC-0001",
                "evidence_b": "DEC-0002",
                "surfaced_at_rev": 1,
                "status": "open"
            },
            {
                "kind": "outcome_divergence",
                "evidence_a": "DEC-0002",
                "evidence_b": "01HXTURN001",
                "surfaced_at_rev": 1,
                "status": "open"
            }
        ]);
        let env = topic_card_envelope("01HXTC07", payload);
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 2);
        let kinds: Vec<&str> = edges
            .iter()
            .filter_map(|e| e.props.get("kind").and_then(|v| v.as_str()))
            .collect();
        assert!(kinds.contains(&"decision_supersession"));
        assert!(kinds.contains(&"outcome_divergence"));
    }
}
