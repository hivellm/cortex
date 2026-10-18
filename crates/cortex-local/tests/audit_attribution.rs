use cortex_local::*;
use serde_json::{json, Value};

fn attribution(agent: Option<&str>) -> AuditAttribution {
    AuditAttribution {
        client: Some("codex".into()),
        session_id: Some("session".into()),
        turn_id: Some("turn".into()),
        agent_id: agent.map(str::to_owned),
        ..Default::default()
    }
}
fn memory(store: &LocalStore) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("rule".into()),
            repository: "Cortex".into(),
            body: "retained rule".into(),
            ..Default::default()
        })
        .unwrap()
        .record
}
fn delivery(store: &LocalStore, agent: Option<&str>) -> String {
    recall::prepare(
        store,
        recall::RecallRequest {
            repository: "Cortex".into(),
            query: String::new(),
            limit: 5,
            budget_bytes: 6144,
            origin: AuditOrigin::Mcp,
            attribution: attribution(agent),
            fallback_recent: false,
        },
    )
    .unwrap()
    .delivery_id
    .unwrap()
}
fn event(store: &LocalStore, id: &str, agent: Value, record: &Record) -> Record {
    store.capture(CaptureRequest {id:Some(id.into()),repository:"Cortex".into(),kind:RecordKind::Event,
        body:format!("[{}@{} {}]",record.id,record.revision,record.content_sha256),
        source:Source {kind:"lifecycle_hook".into(),..Default::default()},
        metadata:json!({"client":"codex","session_id":"session","turn_id":"turn","agent_id":agent,"event_id":"real-event","event":"Stop"}),..Default::default()
    }).unwrap().record
}
fn observation(record: &AuditRecord) -> &ObservationDraft {
    let AuditPayload::Observation(observation) = &record.payload else {
        panic!("observation expected")
    };
    observation
}

#[test]
fn direct_verified_observation_rejects_forged_event_or_agent_attribution() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store);
    let delivered = delivery(&store, Some("parent"));
    let event = event(&store, "stop", json!("parent"), &memory);
    for method in [LinkageMethod::NativeTurn, LinkageMethod::ExactCitation] {
        let mut correct = attribution(Some("parent"));
        correct.event_id = Some("real-event".into());
        let make = |attribution| AuditRequest {
            id: None,
            repository: "Cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: Some(delivered.clone()),
                event: Some(AuditReference::from_record(&event, false).unwrap()),
                attribution,
                method,
                evidence: if method == LinkageMethod::ExactCitation {
                    vec![AuditReference::from_record(&memory, false).unwrap()]
                } else {
                    Vec::new()
                },
                uncertainty: None,
            }),
        };
        let mut forged = correct.clone();
        forged.event_id = Some("forged-event".into());
        assert!(store.append_audit(make(forged)).is_err());
        let mut forged = correct.clone();
        forged.agent_id = Some("child".into());
        assert!(store.append_audit(make(forged)).is_err());
        let mut missing = correct.clone();
        missing.agent_id = None;
        assert!(store.append_audit(make(missing)).is_err());
        assert!(store.append_audit(make(correct.clone())).is_ok());
        correct.event_id = None;
        assert!(store.append_audit(make(correct)).is_ok());
    }
    store.reindex().unwrap();
}

#[test]
fn automatic_linkage_separates_parent_and_child_sharing_native_turn() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store);
    delivery(&store, Some("parent"));
    let child_before = event(&store, "child-before", json!("child"), &memory);
    let unknown = store.observe_event(&child_before).unwrap().record;
    assert_eq!(unknown.evidence_label, EvidenceLabel::Unknown);
    assert_eq!(
        observation(&unknown).uncertainty,
        Some(UncertaintyCode::MissingDelivery)
    );
    assert!(observation(&unknown).delivery_id.is_none());
    let child_delivery = delivery(&store, Some("child"));
    let child_after = event(&store, "child-after", json!("child"), &memory);
    let linked = store.observe_event(&child_after).unwrap().record;
    assert_eq!(linked.evidence_label, EvidenceLabel::CapturedCitation);
    assert_eq!(
        observation(&linked).delivery_id.as_deref(),
        Some(child_delivery.as_str())
    );
    assert_eq!(
        observation(&linked).attribution.agent_id.as_deref(),
        Some("child")
    );
    assert_eq!(store.observe_event(&child_before).unwrap().record, unknown);
    store.reindex().unwrap();
    assert_eq!(store.get_audit("Cortex", &linked.id).unwrap(), Some(linked));
}

#[test]
fn missing_or_invalid_agent_cannot_match_an_agent_specific_delivery() {
    for (delivered_agent, event_agent, expected) in [
        (
            Some("parent"),
            Value::Null,
            UncertaintyCode::MissingDelivery,
        ),
        (None, json!("child"), UncertaintyCode::MissingDelivery),
        (None, json!(" "), UncertaintyCode::MissingAttribution),
        (None, json!(42), UncertaintyCode::MissingAttribution),
        (
            None,
            json!("x".repeat(257)),
            UncertaintyCode::MissingAttribution,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let memory = memory(&store);
        delivery(&store, delivered_agent);
        let action = event(&store, "stop", event_agent, &memory);
        let result = store.observe_event(&action).unwrap().record;
        assert_eq!(result.evidence_label, EvidenceLabel::Unknown);
        assert_eq!(observation(&result).method, LinkageMethod::Unlinked);
        assert_eq!(observation(&result).uncertainty, Some(expected));
        assert!(observation(&result).delivery_id.is_none());
    }
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store);
    let delivered = delivery(&store, None);
    let action = event(&store, "no-agent", Value::Null, &memory);
    let result = store.observe_event(&action).unwrap().record;
    assert_eq!(result.evidence_label, EvidenceLabel::CapturedCitation);
    assert_eq!(
        observation(&result).delivery_id.as_deref(),
        Some(delivered.as_str())
    );
}

#[test]
fn supplied_event_id_requires_canonical_value_even_when_other_ids_match() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store);
    let delivered = delivery(&store, None);
    let action=store.capture(CaptureRequest {id:Some("no-event-id".into()),repository:"Cortex".into(),kind:RecordKind::Event,source:Source {kind:"lifecycle_hook".into(),..Default::default()},metadata:json!({"event":"PostToolUse","client":"codex","session_id":"session","turn_id":"turn"}),..Default::default()}).unwrap().record;
    let mut supplied = attribution(None);
    supplied.event_id = Some("invented".into());
    assert!(store
        .append_audit(AuditRequest {
            id: None,
            repository: "Cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: Some(delivered),
                event: Some(AuditReference::from_record(&action, false).unwrap()),
                attribution: supplied,
                method: LinkageMethod::NativeTurn,
                evidence: Vec::new(),
                uncertainty: None
            })
        })
        .is_err());
    assert_eq!(
        store.observe_event(&action).unwrap().record.evidence_label,
        EvidenceLabel::ObservedRelatedAction
    );
    assert_eq!(store.get("Cortex", &memory.id).unwrap(), Some(memory));
}

#[test]
fn unknown_observations_require_consistent_uncertainty_breakdown() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for (method, uncertainty, valid) in [
        (LinkageMethod::ReportedUse, None, false),
        (
            LinkageMethod::ReportedUse,
            Some(UncertaintyCode::MissingDelivery),
            false,
        ),
        (
            LinkageMethod::ReportedUse,
            Some(UncertaintyCode::ReportedOnly),
            true,
        ),
        (LinkageMethod::Unlinked, None, false),
        (
            LinkageMethod::Unlinked,
            Some(UncertaintyCode::ReportedOnly),
            false,
        ),
        (
            LinkageMethod::Unlinked,
            Some(UncertaintyCode::MissingAttribution),
            true,
        ),
        (
            LinkageMethod::Unlinked,
            Some(UncertaintyCode::MissingDelivery),
            true,
        ),
        (
            LinkageMethod::Unlinked,
            Some(UncertaintyCode::AmbiguousDelivery),
            true,
        ),
    ] {
        let result = store.append_audit(AuditRequest {
            id: None,
            repository: "Cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: None,
                event: None,
                attribution: AuditAttribution::default(),
                method,
                evidence: Vec::new(),
                uncertainty,
            }),
        });
        assert_eq!(result.is_ok(), valid);
        if let Ok(outcome) = result {
            assert_eq!(outcome.record.evidence_label, EvidenceLabel::Unknown);
        }
    }
}
