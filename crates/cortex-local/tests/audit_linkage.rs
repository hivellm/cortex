use cortex_local::*;
use serde_json::json;

fn memory(store: &LocalStore, repository: &str) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("rule".into()),
            repository: repository.into(),
            title: "Compile first".into(),
            body: "Run cargo check before tests".into(),
            ..Default::default()
        })
        .unwrap()
        .record
}
fn attribution() -> AuditAttribution {
    AuditAttribution {
        client: Some("codex".into()),
        session_id: Some("session".into()),
        turn_id: Some("turn".into()),
        ..Default::default()
    }
}
fn recall(store: &LocalStore, repository: &str, attribution: AuditAttribution) -> String {
    recall::prepare(
        store,
        recall::RecallRequest {
            repository: repository.into(),
            query: String::new(),
            limit: 5,
            budget_bytes: 6144,
            origin: AuditOrigin::Mcp,
            attribution,
            fallback_recent: false,
        },
    )
    .unwrap()
    .delivery_id
    .unwrap()
}
fn event(
    store: &LocalStore,
    repository: &str,
    id: &str,
    name: &str,
    body: &str,
    metadata: serde_json::Value,
) -> Record {
    let mut metadata = metadata;
    metadata["event"] = json!(name);
    store
        .capture(CaptureRequest {
            id: Some(id.into()),
            repository: repository.into(),
            kind: RecordKind::Event,
            body: body.into(),
            source: Source {
                kind: "lifecycle_hook".into(),
                ..Default::default()
            },
            metadata,
            ..Default::default()
        })
        .unwrap()
        .record
}
fn identity() -> serde_json::Value {
    json!({"client":"codex","session_id":"session","turn_id":"turn"})
}
fn draft(record: &AuditRecord) -> &ObservationDraft {
    let AuditPayload::Observation(draft) = &record.payload else {
        panic!("observation required")
    };
    draft
}

#[test]
fn related_actions_and_self_reports_stay_distinct_from_captured_citations() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let original = memory(&store, "Cortex");
    let delivery = recall(&store, "Cortex", attribution());
    for (id, name, body) in [
        ("action", "PostToolUse", "cargo check passed"),
        ("self-report", "Stop", "I followed the memory correctly"),
    ] {
        let action = event(&store, "Cortex", id, name, body, identity());
        let receipt = store.observe_event(&action).unwrap().record;
        assert_eq!(receipt.evidence_label, EvidenceLabel::ObservedRelatedAction);
        assert_eq!(
            draft(&receipt).delivery_id.as_deref(),
            Some(delivery.as_str())
        );
        assert_eq!(draft(&receipt).method, LinkageMethod::NativeTurn);
        assert!(draft(&receipt).evidence.is_empty());
        assert!(receipt.verification.is_none());
    }
    store
        .revise(
            "Cortex",
            "rule",
            1,
            RevisionRequest {
                title: "Changed".into(),
                body: "Different current revision".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: json!({}),
            },
        )
        .unwrap();
    let citation = format!(
        "[{}@{} {}]",
        original.id, original.revision, original.content_sha256
    );
    let action = event(&store, "Cortex", "citation", "Stop", &citation, identity());
    let receipt = store.observe_event(&action).unwrap().record;
    assert_eq!(receipt.evidence_label, EvidenceLabel::CapturedCitation);
    assert_eq!(
        receipt.verification,
        Some(Verification::CanonicalEventCitation)
    );
    assert_eq!(
        draft(&receipt).evidence,
        vec![AuditReference::from_record(&original, false).unwrap()]
    );
    assert_eq!(
        draft(&receipt).event,
        Some(AuditReference::from_record(&action, false).unwrap())
    );
    store.reindex().unwrap();
    assert_eq!(
        store.get_audit("Cortex", &receipt.id).unwrap(),
        Some(receipt)
    );
}

#[test]
fn absent_or_mismatched_identity_and_foreign_scope_never_borrow_delivery() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    memory(&store, "Cortex");
    recall(&store, "Cortex", attribution());
    for (id, repository, meta, expected) in [
        (
            "missing",
            "Cortex",
            json!({"client":"codex","session_id":"session"}),
            UncertaintyCode::MissingAttribution,
        ),
        (
            "blank",
            "Cortex",
            json!({"client":"codex","session_id":"session","turn_id":" "}),
            UncertaintyCode::MissingAttribution,
        ),
        (
            "foreign",
            "cortex",
            identity(),
            UncertaintyCode::MissingDelivery,
        ),
        (
            "other-turn",
            "Cortex",
            json!({"client":"codex","session_id":"session","turn_id":"other"}),
            UncertaintyCode::MissingDelivery,
        ),
        (
            "other-client",
            "Cortex",
            json!({"client":"claude-code","session_id":"session","turn_id":"turn"}),
            UncertaintyCode::MissingDelivery,
        ),
    ] {
        let action = event(&store, repository, id, "PostToolUse", "cargo check", meta);
        let receipt = store.observe_event(&action).unwrap().record;
        assert_eq!(receipt.evidence_label, EvidenceLabel::Unknown);
        assert_eq!(draft(&receipt).method, LinkageMethod::Unlinked);
        assert_eq!(draft(&receipt).uncertainty, Some(expected));
        assert!(draft(&receipt).delivery_id.is_none());
    }
}

#[test]
fn ambiguity_and_retries_keep_original_evidence_even_after_new_deliveries() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    memory(&store, "Cortex");
    let delivery = recall(&store, "Cortex", attribution());
    let action = event(
        &store,
        "Cortex",
        "first",
        "PostToolUse",
        "check",
        identity(),
    );
    let first = store.observe_event(&action).unwrap();
    assert!(first.created);
    recall(&store, "Cortex", attribution());
    let replay = store.observe_event(&action).unwrap();
    assert!(!replay.created);
    assert_eq!(first.record, replay.record);
    assert_eq!(
        draft(&replay.record).delivery_id.as_deref(),
        Some(delivery.as_str())
    );
    let second = event(
        &store,
        "Cortex",
        "second",
        "PostToolUse",
        "check",
        identity(),
    );
    let uncertain = store.observe_event(&second).unwrap().record;
    assert_eq!(uncertain.evidence_label, EvidenceLabel::Unknown);
    assert_eq!(
        draft(&uncertain).uncertainty,
        Some(UncertaintyCode::AmbiguousDelivery)
    );
    assert!(draft(&uncertain).delivery_id.is_none());
}

#[test]
fn empty_deliveries_and_future_receipts_are_ineligible_but_archived_prepared_remains_linkable() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    recall(&store, "Cortex", attribution());
    let before = event(&store, "Cortex", "before", "Stop", "done", identity());
    memory(&store, "Cortex");
    let delivery = recall(&store, "Cortex", attribution());
    let unknown = store.observe_event(&before).unwrap().record;
    assert_eq!(
        draft(&unknown).uncertainty,
        Some(UncertaintyCode::MissingDelivery)
    );
    store
        .retain_audit(AuditRetentionRequest {
            repository: "Cortex".into(),
            cutoff: "9999-01-01T00:00:00Z".into(),
            limit: 100,
            apply: true,
        })
        .unwrap();
    let after = event(
        &store,
        "Cortex",
        "after",
        "PostToolUse",
        "check",
        identity(),
    );
    let related = store.observe_event(&after).unwrap().record;
    assert_eq!(related.evidence_label, EvidenceLabel::ObservedRelatedAction);
    assert_eq!(
        draft(&related).delivery_id.as_deref(),
        Some(delivery.as_str())
    );
    store.reindex().unwrap();
    let visible = store
        .list_audit(AuditListQuery {
            repository: "Cortex".into(),
            limit: 100,
            after: None,
            before: None,
            kind: Some(AuditKind::Observation),
            cursor: None,
            include_archived: false,
        })
        .unwrap();
    assert_eq!(visible.entries.len(), 1);
    assert_eq!(visible.entries[0].record, unknown);
}

#[test]
fn forged_event_and_partial_or_tool_citations_cannot_upgrade_evidence() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let original = memory(&store, "Cortex");
    let delivery = recall(&store, "Cortex", attribution());
    let citation = format!(
        "[{}@{} {}]",
        original.id, original.revision, original.content_sha256
    );
    for (id, name, body) in [
        ("tool", "PostToolUse", citation.as_str()),
        ("partial", "Stop", "[rule@1]"),
    ] {
        let action = event(&store, "Cortex", id, name, body, identity());
        let receipt = store.observe_event(&action).unwrap().record;
        assert_eq!(receipt.evidence_label, EvidenceLabel::ObservedRelatedAction);
        assert!(draft(&receipt).evidence.is_empty());
        let request = AuditRequest {
            id: None,
            repository: "Cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: Some(delivery.clone()),
                event: Some(AuditReference::from_record(&action, false).unwrap()),
                attribution: attribution(),
                method: LinkageMethod::ExactCitation,
                evidence: vec![AuditReference::from_record(&original, false).unwrap()],
                uncertainty: None,
            }),
        };
        assert!(store.append_audit(request).is_err());
        let mut forged = action;
        forged.body = format!("{citation} fabricated");
        assert!(store.observe_event(&forged).is_err());
    }
    assert!(store.observe_event(&original).is_err());
}

#[test]
fn real_hook_capture_creates_idempotent_observations_in_separate_audit_store() {
    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("cortex.toml"), "[cortex]\nid='Cortex'\n").unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store, "Cortex");
    hooks::run(
        root.path(),
        "codex",
        Some("UserPromptSubmit"),
        json!({"cwd":repo.path(),"prompt":"Compile","session_id":"session","turn_id":"turn"}),
    );
    let input = json!({"cwd":repo.path(),"session_id":"session","turn_id":"turn","last_assistant_message":format!("[{}@1 {}]",memory.id,memory.content_sha256)});
    for _ in 0..2 {
        assert_eq!(
            hooks::run(root.path(), "codex", Some("Stop"), input.clone()),
            json!({})
        );
    }
    let observations = store
        .list_audit(AuditListQuery {
            repository: "Cortex".into(),
            limit: 100,
            after: None,
            before: None,
            kind: Some(AuditKind::Observation),
            cursor: None,
            include_archived: true,
        })
        .unwrap();
    assert_eq!(observations.entries.len(), 1);
    assert_eq!(
        observations.entries[0].record.evidence_label,
        EvidenceLabel::CapturedCitation
    );
    assert_eq!(store.status(Some("Cortex")).unwrap().records, 3);
}
