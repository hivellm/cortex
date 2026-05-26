//! Phase15b §2.10 — `CITES` edge extractor.
//!
//! Surfaces explicit decision-id / event-id citations from Turn
//! and Analysis envelopes. Two sources merge into one edge set:
//!
//! 1. **Classifier relations** with the canonical `REFERENCES`
//!    label (per `ExtractedRelation` "event mentions entity") OR
//!    the explicit `CITES` label — both map to the same graph
//!    edge type here so the dashboard sees one canonical
//!    citation stream.
//! 2. **Body regex** — `DEC-\d{3,}` patterns inside the
//!    envelope's user_message / assistant_message / payload body.
//!    Recovers citations the classifier missed (static-fallback
//!    path or budget-clipped runs).
//!
//! ## Scope
//!
//! - `Kind::Turn` + `Kind::Analysis` (debate envelopes).
//! - Edge shape: `<KindLabel>[event_id] -CITES-> <TargetLabel>[id]`
//!   where TargetLabel is `Decision` for body-regex hits +
//!   classifier-relation resolution for entity-table hits.
//! - Body regex emits `Decision` edges only (DEC-\d{3,} is
//!   decision-shaped). Other citation patterns (event_id ULIDs,
//!   law ids) flow through the classifier-relations path which
//!   already knows the typed target.
//! - Deduplicated: a Decision id surfacing in both the
//!   classifier-relations AND the body-regex paths emits once
//!   (BTreeSet over the resolved (target_label, target_key) pair).

use std::collections::BTreeSet;

use cortex_core::events::Kind;

use super::{resolve_endpoint, stamp_provenance, Edge, ExtractCtx, ANCHOR_THIS_EVENT};
use crate::embedder::EnrichedEvent;

/// Canonical edge type string the projection pipeline filters on.
pub const EDGE_TYPE: &str = "CITES";

/// Extract `CITES` edges from a [`EnrichedEvent`]. Returns the
/// empty vec for any envelope kind other than `Turn` or
/// `Analysis`, or when neither the classifier relations nor the
/// body-regex pass surface any citation.
pub fn extract(env: &EnrichedEvent, ctx: &ExtractCtx) -> Vec<Edge> {
    if !matches!(env.kind, Kind::Turn | Kind::Analysis) {
        return Vec::new();
    }
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut out: Vec<Edge> = Vec::new();

    // 1. Classifier relations (REFERENCES / CITES → CITES edge).
    for rel in &env.classifier.relations {
        if !rel.relation.eq_ignore_ascii_case("REFERENCES")
            && !rel.relation.eq_ignore_ascii_case("CITES")
        {
            continue;
        }
        // The `from` is always the envelope itself; CITES is a
        // one-way reference. Skip relations whose `from` is some
        // other entity (mis-classified shape).
        if rel.from != ANCHOR_THIS_EVENT {
            continue;
        }
        let Some((to_label, to_key)) = resolve_endpoint(env, rel.to.as_str()) else {
            continue;
        };
        if !seen.insert((to_label.clone(), to_key.clone())) {
            continue;
        }
        let edge = Edge {
            edge_type: EDGE_TYPE.to_string(),
            from_label: super::kind_to_label(env.kind).to_string(),
            from_key: env.event_id.clone(),
            to_label,
            to_key,
            props: std::collections::BTreeMap::new(),
        };
        out.push(stamp_provenance(edge, env, ctx));
    }

    // 2. Body regex — DEC-\d{3,} in any string field of the
    // redacted payload. Recovers citations the classifier missed.
    for dec_id in find_dec_ids(&env.redacted_payload) {
        let key = ("Decision".to_string(), dec_id.clone());
        if !seen.insert(key) {
            continue;
        }
        let edge = Edge {
            edge_type: EDGE_TYPE.to_string(),
            from_label: super::kind_to_label(env.kind).to_string(),
            from_key: env.event_id.clone(),
            to_label: "Decision".to_string(),
            to_key: dec_id,
            props: std::collections::BTreeMap::new(),
        };
        out.push(stamp_provenance(edge, env, ctx));
    }

    out
}

/// Walk every string in `payload` and collect `DEC-\d{3,}`
/// matches. Conservative: only the canonical prefix; downstream
/// `CITES` over speculative ids would be noise.
pub(crate) fn find_dec_ids(payload: &serde_json::Value) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    collect_strings(payload, &mut |s| {
        scan_dec_ids(s, &mut out);
    });
    out.into_iter().collect()
}

fn scan_dec_ids(text: &str, out: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == b"DEC-" {
            let mut j = i + 4;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - (i + 4) >= 3 {
                if let Ok(id) = std::str::from_utf8(&bytes[i..j]) {
                    out.insert(id.to_string());
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

fn collect_strings(value: &serde_json::Value, sink: &mut dyn FnMut(&str)) {
    match value {
        serde_json::Value::String(s) => sink(s),
        serde_json::Value::Array(a) => {
            for v in a {
                collect_strings(v, sink);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_strings(v, sink);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture_envelope;
    use super::*;
    use crate::classifier::{ExtractedEntity, ExtractedRelation};
    use serde_json::json;

    fn ctx() -> ExtractCtx {
        ExtractCtx::with_now_ms("phase15b-v1", 1_700_000_000_000)
    }

    fn turn_envelope(event_id: &str) -> EnrichedEvent {
        let mut env = fixture_envelope(event_id, Kind::Turn);
        env.redacted_payload = json!({
            "user_message": "hi",
            "assistant_message": "Implements DEC-0042 and supersedes DEC-0001.",
            "tool_call_event_ids": []
        });
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
    fn extract_returns_empty_for_non_turn_or_analysis_kinds() {
        for k in [
            Kind::ToolCall,
            Kind::Decision,
            Kind::Memory,
            Kind::TopicCard,
            Kind::Consolidation,
        ] {
            let env = fixture_envelope("01HXEVT01", k);
            assert!(extract(&env, &ctx()).is_empty());
        }
    }

    #[test]
    fn extract_emits_body_regex_decision_citations() {
        let env = turn_envelope("01HXTURN01");
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 2);
        let to_keys: BTreeSet<String> = edges.iter().map(|e| e.to_key.clone()).collect();
        assert!(to_keys.contains("DEC-0042"));
        assert!(to_keys.contains("DEC-0001"));
        for e in &edges {
            assert_eq!(e.edge_type, "CITES");
            assert_eq!(e.from_label, "Turn");
            assert_eq!(e.to_label, "Decision");
        }
    }

    #[test]
    fn extract_emits_classifier_references_relation() {
        let mut env = turn_envelope("01HXTURN02");
        env.redacted_payload = json!({"user_message": "plain", "tool_call_event_ids": []});
        env.classifier.entities = vec![entity("concept", "tokio::spawn")];
        env.classifier.relations = vec![rel("this_event", "REFERENCES", "tokio::spawn")];
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_label, "Concept");
        assert_eq!(edges[0].to_key, "tokio::spawn");
    }

    #[test]
    fn extract_matches_cites_label_in_addition_to_references() {
        let mut env = turn_envelope("01HXTURN03");
        env.redacted_payload = json!({"user_message": "plain", "tool_call_event_ids": []});
        env.classifier.entities = vec![entity("law", "LAW-007")];
        env.classifier.relations = vec![rel("this_event", "CITES", "LAW-007")];
        let edges = extract(&env, &ctx());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_label, "Law");
    }

    #[test]
    fn extract_dedupes_decision_citation_appearing_in_both_sources() {
        let mut env = turn_envelope("01HXTURN04");
        env.classifier.entities = vec![entity("decision", "DEC-0042")];
        env.classifier.relations = vec![rel("this_event", "REFERENCES", "DEC-0042")];
        let edges = extract(&env, &ctx());
        // Body regex finds DEC-0042 and DEC-0001; classifier adds
        // DEC-0042 (dup) — total unique: 2.
        let unique_keys: BTreeSet<String> = edges.iter().map(|e| e.to_key.clone()).collect();
        assert_eq!(unique_keys.len(), 2);
        assert!(unique_keys.contains("DEC-0042"));
        assert!(unique_keys.contains("DEC-0001"));
    }

    #[test]
    fn extract_ignores_relations_with_non_this_event_from() {
        let mut env = turn_envelope("01HXTURN05");
        env.redacted_payload = json!({"user_message": "plain", "tool_call_event_ids": []});
        env.classifier.entities = vec![
            entity("decision", "DEC-0001"),
            entity("decision", "DEC-0002"),
        ];
        env.classifier.relations = vec![rel("DEC-0001", "REFERENCES", "DEC-0002")];
        let edges = extract(&env, &ctx());
        assert!(
            edges.is_empty(),
            "non-this_event REFERENCES should not emit CITES from the envelope"
        );
    }

    #[test]
    fn extract_stamps_provenance_triple() {
        let env = turn_envelope("01HXTURN06");
        let edges = extract(&env, &ctx());
        let p = &edges[0].props;
        assert_eq!(
            p.get("source_event_id").and_then(|v| v.as_str()),
            Some("01HXTURN06")
        );
        assert_eq!(
            p.get("analyzer_version").and_then(|v| v.as_str()),
            Some("phase15b-v1")
        );
    }

    #[test]
    fn extract_returns_empty_when_no_citations_anywhere() {
        let mut env = turn_envelope("01HXTURN07");
        env.redacted_payload = json!({"user_message": "no refs", "tool_call_event_ids": []});
        env.classifier.relations = Vec::new();
        assert!(extract(&env, &ctx()).is_empty());
    }
}
