use cortex_local::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, LocalStore) {
    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("cortex.toml"), "[cortex]\nid='Cortex'\n").unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    (root, repo, store)
}
fn memory(store: &LocalStore) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("known".into()),
            repository: "Cortex".into(),
            body: "retain original evidence".into(),
            kind: RecordKind::Lesson,
            source: Source {
                kind: "fixture".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}
fn receipts(store: &LocalStore) -> Vec<AuditRecord> {
    store
        .list_audit(AuditListQuery {
            repository: "Cortex".into(),
            limit: 100,
            after: None,
            before: None,
            kind: Some(AuditKind::Delivery),
            cursor: None,
            include_archived: true,
        })
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.record)
        .collect()
}

#[test]
fn hook_origins_and_exact_rendered_receipts_match_each_client_event() {
    for (client, event, origin) in [
        ("codex", "SessionStart", AuditOrigin::CodexSessionStart),
        (
            "codex",
            "UserPromptSubmit",
            AuditOrigin::CodexUserPromptSubmit,
        ),
        (
            "claude-code",
            "SessionStart",
            AuditOrigin::ClaudeSessionStart,
        ),
        (
            "claude-code",
            "UserPromptSubmit",
            AuditOrigin::ClaudeUserPromptSubmit,
        ),
        (
            "opencode",
            "SessionStart",
            AuditOrigin::OpenCodeSessionStart,
        ),
        (
            "opencode",
            "UserPromptSubmit",
            AuditOrigin::OpenCodeUserPromptSubmit,
        ),
    ] {
        let (root, repo, store) = setup();
        let memory = memory(&store);
        let response = hooks::run(
            root.path(),
            client,
            Some(event),
            json!({"cwd":repo.path(),"prompt":"retain","session_id":"session","turn_id":"turn","event_id":"native-event","agent_id":"agent"}),
        );
        let context = response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains(&format!(
            "[{}@{} {}]",
            memory.id, memory.revision, memory.content_sha256
        )));
        let records = receipts(&store);
        assert_eq!(records.len(), 1);
        let AuditPayload::Delivery(d) = &records[0].payload else {
            panic!("delivery expected")
        };
        assert_eq!(d.origin, origin);
        assert_eq!(d.attribution.client.as_deref(), Some(client));
        assert_eq!(d.attribution.session_id.as_deref(), Some("session"));
        assert_eq!(d.attribution.turn_id.as_deref(), Some("turn"));
        assert_eq!(d.attribution.event_id.as_deref(), Some("native-event"));
        assert_eq!(d.attribution.agent_id.as_deref(), Some("agent"));
        assert_eq!(
            d.references,
            vec![AuditReference::from_record(&memory, false).unwrap()]
        );
        assert_eq!(d.context_bytes, context.len());
        assert_eq!(
            d.context_sha256,
            format!("{:x}", Sha256::digest(context.as_bytes()))
        );
        assert_eq!(records[0].evidence_label, EvidenceLabel::PreparedDelivery);
    }
}

#[test]
fn redacted_native_ids_never_collapse_events_or_link_distinct_turns() {
    let (root, repo, store) = setup();
    memory(&store);
    for suffix in ["A", "B"] {
        let secret = format!("ghp_{}", suffix.repeat(40));
        let input = json!({"cwd":repo.path(),"session_id":secret,"turn_id":secret,
            "event_id":secret,"tool_use_id":secret,"agent_id":secret,
            "last_assistant_message":"same response"});
        hooks::run(root.path(), "codex", Some("SessionStart"), input.clone());
        hooks::run(root.path(), "codex", Some("Stop"), input);
    }
    let deliveries = receipts(&store);
    assert_eq!(deliveries.len(), 2);
    for record in deliveries {
        let AuditPayload::Delivery(delivery) = record.payload else {
            panic!("delivery")
        };
        assert_eq!(
            delivery.attribution,
            AuditAttribution {
                client: Some("codex".into()),
                ..Default::default()
            }
        );
    }
    let events: Vec<_> = store
        .recent("Cortex", 100, true)
        .unwrap()
        .into_iter()
        .filter(|record| record.kind == RecordKind::Event)
        .collect();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].id, events[1].id);
    for record in events {
        let encoded = serde_json::to_string(&record).unwrap();
        assert!(!encoded.contains("ghp_") && !encoded.contains("[REDACTED:"));
        assert!(record.metadata.get("session_id").is_none());
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
    assert_eq!(observations.entries.len(), 2);
    assert!(observations
        .entries
        .iter()
        .all(|entry| entry.record.evidence_label == EvidenceLabel::Unknown));
}

#[test]
fn stop_fallback_preserves_changed_answers_and_agents_but_deduplicates_retries() {
    let (root, repo, store) = setup();
    for (agent, body) in [
        ("one", "blocked response"),
        ("one", "final response"),
        ("two", "final response"),
    ] {
        let input = json!({"cwd":repo.path(),"session_id":"session","turn_id":"turn",
            "agent_id":agent,"last_assistant_message":body});
        for _ in 0..2 {
            hooks::run(root.path(), "codex", Some("Stop"), input.clone());
        }
    }
    let events = store.recent("Cortex", 100, true).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .filter(|record| record.body == "final response")
            .count(),
        2
    );
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
    assert_eq!(observations.entries.len(), 3);
}

#[test]
fn pre_redacted_and_invalid_native_ids_are_absent() {
    let (root, repo, store) = setup();
    hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({
            "cwd":repo.path(),"session_id":"[REDACTED:credential]",
            "turn_id":"invalid\nturn","agent_id":" ","event_id":"x".repeat(257)
        }),
    );
    let records = receipts(&store);
    let AuditPayload::Delivery(delivery) = &records[0].payload else {
        panic!("delivery")
    };
    assert_eq!(
        delivery.attribution,
        AuditAttribution {
            client: Some("codex".into()),
            ..Default::default()
        }
    );
}

#[test]
fn redacted_child_identity_cannot_link_to_parent_without_agent_id() {
    let (root, repo, store) = setup();
    memory(&store);
    hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({
            "cwd":repo.path(),"session_id":"session","turn_id":"turn"
        }),
    );
    hooks::run(
        root.path(),
        "codex",
        Some("PostToolUse"),
        json!({
            "cwd":repo.path(),"session_id":"session","turn_id":"turn",
            "event_id":"child-action","agent_id":format!("ghp_{}", "A".repeat(40)),
            "tool_response":"child action"
        }),
    );
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
    let record = &observations.entries[0].record;
    assert_eq!(record.evidence_label, EvidenceLabel::Unknown);
    let AuditPayload::Observation(observation) = &record.payload else {
        panic!("observation")
    };
    assert!(observation.delivery_id.is_none());
    assert!(observation.attribution.turn_id.is_none());
    assert!(observation.attribution.agent_id.is_none());
}

#[test]
fn null_agent_identity_preserves_valid_native_attribution() {
    let (root, repo, store) = setup();
    hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({
            "cwd":repo.path(),"session_id":"session","turn_id":"turn",
            "event_id":"event","agent_id":null
        }),
    );
    let records = receipts(&store);
    let AuditPayload::Delivery(delivery) = &records[0].payload else {
        panic!("delivery")
    };
    assert_eq!(delivery.attribution.session_id.as_deref(), Some("session"));
    assert_eq!(delivery.attribution.turn_id.as_deref(), Some("turn"));
    assert_eq!(delivery.attribution.event_id.as_deref(), Some("event"));
    assert!(delivery.attribution.agent_id.is_none());
}

#[test]
fn hooks_keep_missing_or_invalid_native_ids_absent_and_record_empty_receipt() {
    let (root, repo, store) = setup();
    let response = hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({"cwd":repo.path(),"session_id":"","turn_id":" ","agent_id":"x".repeat(257),"event_id":42}),
    );
    assert_eq!(response, json!({}));
    let records = receipts(&store);
    assert_eq!(records.len(), 1);
    let AuditPayload::Delivery(d) = &records[0].payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.outcome, PreparationOutcome::Empty);
    assert!(d.references.is_empty());
    assert_eq!(
        d.attribution,
        AuditAttribution {
            client: Some("codex".into()),
            ..Default::default()
        }
    );
    assert_eq!(records[0].evidence_label, EvidenceLabel::Unknown);
    hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({"cwd":repo.path(),"turn_id":"real-turn","tool_use_id":"real-tool"}),
    );
    let records = receipts(&store);
    let d = records
        .iter()
        .find_map(|r| match &r.payload {
            AuditPayload::Delivery(d) if d.attribution.turn_id.is_some() => Some(d),
            _ => None,
        })
        .unwrap();
    assert_eq!(d.attribution.event_id.as_deref(), Some("real-tool"));
    assert!(d.attribution.session_id.is_none());
}

#[test]
fn hook_preparation_failure_persists_bounded_error_without_raw_prompt() {
    let (root, repo, store) = setup();
    memory(&store);
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE revisions SET record='invalid JSON fixture' WHERE repository='Cortex'",
            [],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        hooks::run(
            root.path(),
            "codex",
            Some("SessionStart"),
            json!({"cwd":repo.path()})
        ),
        json!({})
    );
    let records = receipts(&store);
    assert_eq!(records.len(), 1);
    let AuditPayload::Delivery(d) = &records[0].payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.outcome, PreparationOutcome::Error);
    assert!(d.error.is_some());
    assert_eq!(d.context_bytes, 0);
    assert!(d.references.is_empty());
    assert!(!serde_json::to_string(&records[0])
        .unwrap()
        .contains("invalid JSON fixture"));
}

#[test]
fn oversized_single_token_prompt_preserves_recent_fallback() {
    let (root, repo, store) = setup();
    let record = memory(&store);
    let response = hooks::run(
        root.path(),
        "codex",
        Some("UserPromptSubmit"),
        json!({"cwd":repo.path(),"prompt":"x".repeat(5000)}),
    );
    assert!(response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("known@1"));
    let records = receipts(&store);
    assert_eq!(records.len(), 1);
    let AuditPayload::Delivery(d) = &records[0].payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.outcome, PreparationOutcome::Prepared);
    assert_eq!(d.query_sha256, format!("{:x}", Sha256::digest(b"")));
    assert_eq!(
        d.references,
        vec![AuditReference::from_record(&record, false).unwrap()]
    );
}

#[test]
fn prepared_hook_context_survives_later_prompt_capture_failure() {
    let (root, repo, store) = setup();
    memory(&store);
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_event_insert BEFORE INSERT ON revisions WHEN json_extract(NEW.record,'$.kind')='event' BEGIN SELECT RAISE(ABORT,'fixture capture failure'); END;").unwrap();
    drop(connection);
    let response = hooks::run(
        root.path(),
        "codex",
        Some("UserPromptSubmit"),
        json!({"cwd":repo.path(),"prompt":"retain"}),
    );
    assert!(response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("known@1"));
    assert!(root.path().join("pending").exists());
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_event_insert;")
        .unwrap();
    drop(connection);
    let recovered = LocalStore::open(root.path()).unwrap();
    assert_eq!(receipts(&recovered).len(), 1);
    assert!(!root.path().join("pending").exists());
}
