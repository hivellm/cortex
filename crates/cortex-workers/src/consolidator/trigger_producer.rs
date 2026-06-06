//! phase24 §1 — consolidator trigger producer.
//!
//! The consolidator daemon (`daemon::ConsolidatorDaemon`) is event-driven:
//! it dispatches one consolidation per envelope read off the Synap stream
//! [`daemon::TRIGGER_STREAM`] (`cortex.consolidator.triggers`). Until this
//! module landed, nothing published to that stream in the live stack — the
//! daemon ran with `dispatched=0` and the `cortex_consolidations` /
//! `cortex_topic_cards` indexes were never created, leaving
//! `cortex_similar_sessions`, `cortex_topic_search`, and the
//! `cortex_consolidations_*` MCP surfaces empty.
//!
//! This module builds the trigger envelopes (§3 wire shape in
//! `docs/specs/27-consolidation.md`). The classifier worker hosts the
//! decision-landed hook (§1.4): it already sees every enriched event with
//! its [`Kind`] and holds a Synap publisher, so a `Kind::Decision` event
//! fans out one `decision_landed` trigger. The producer is gated behind a
//! default-off config flag because the decision-trace grain auto-promotes
//! to Opus — firing it per decision is real spend the operator must opt
//! into.
//!
//! Re-triggering the same decision is safe: the daemon writes a
//! `producer_checkpoints` row keyed on `decision:<decision_id>` (spec 27
//! §2.4), so a decision already consolidated is skipped on the next fire.

use cortex_core::events::{DecisionPayload, Kind};
use serde_json::{json, Value};

use crate::embedder::EnrichedEvent;

/// Build a `decision_landed` trigger envelope for a `Kind::Decision`
/// event, or `None` when the event is not a decision or carries no
/// usable `decision_id`.
///
/// Wire shape (spec 27 §3):
/// ```json
/// { "kind": "decision_landed", "decision_id": "ADR-0042", "force_deep": false }
/// ```
#[must_use]
pub fn decision_landed_trigger(event: &EnrichedEvent) -> Option<Value> {
    if !matches!(event.kind, Kind::Decision) {
        return None;
    }
    let payload: DecisionPayload = serde_json::from_value(event.redacted_payload.clone()).ok()?;
    if payload.decision_id.trim().is_empty() {
        return None;
    }
    Some(json!({
        "kind": "decision_landed",
        "decision_id": payload.decision_id,
        "force_deep": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use serde_json::json;

    fn classifier_stub(event_id: &str) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.to_string(),
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn event(kind: Kind, payload: Value) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "evt-1".into(),
            kind,
            content_hash: "h-1".into(),
            redacted_payload: payload,
            classifier: classifier_stub("evt-1"),
            context_repo: Some("cortex".into()),
            context_path: Some(".rulebook/decisions/042.md".into()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
        }
    }

    #[test]
    fn decision_event_yields_trigger() {
        let ev = event(
            Kind::Decision,
            json!({
                "decision_id": "ADR-0042",
                "title": "Adopt Meili",
                "status": "accepted",
                "body": "body",
                "tags": []
            }),
        );
        let trigger = decision_landed_trigger(&ev).expect("decision yields a trigger");
        assert_eq!(trigger["kind"], "decision_landed");
        assert_eq!(trigger["decision_id"], "ADR-0042");
        assert_eq!(trigger["force_deep"], false);
    }

    #[test]
    fn non_decision_event_yields_nothing() {
        let ev = event(Kind::Turn, json!({ "user_message": "hi" }));
        assert!(decision_landed_trigger(&ev).is_none());
    }

    #[test]
    fn decision_without_id_yields_nothing() {
        let ev = event(
            Kind::Decision,
            json!({
                "decision_id": "",
                "title": "No id",
                "status": "proposed",
                "body": "body",
                "tags": []
            }),
        );
        assert!(decision_landed_trigger(&ev).is_none());
    }

    #[test]
    fn malformed_decision_payload_yields_nothing() {
        let ev = event(Kind::Decision, json!({ "not": "a decision" }));
        assert!(decision_landed_trigger(&ev).is_none());
    }
}
