use cortex_local::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn memory(store: &LocalStore, title: &str, body: &str) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("memory".into()),
            repository: "Cortex".into(),
            title: title.into(),
            body: body.into(),
            kind: RecordKind::Lesson,
            source: Source {
                kind: "fixture".into(),
                locator: "https://example.invalid/?token=fixture-secret".into(),
                payload: json!({"private":"source-payload-fixture"}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}
fn read(store: &LocalStore, args: Value) -> Value {
    mcp::call(store, "cortex_pre_thinking", args, None).unwrap()
}
fn receipt(store: &LocalStore, response: &Value) -> AuditRecord {
    assert_eq!(response["audit_persisted"], true);
    store
        .get_audit("Cortex", response["delivery_id"].as_str().unwrap())
        .unwrap()
        .unwrap()
}
fn list(store: &LocalStore) -> Vec<AuditRecord> {
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
fn mcp_receipts_have_unique_ids_exact_hashes_and_no_raw_source_or_prompt() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let original = memory(&store, "evidence", "opaque-query-token original body");
    let args = json!({"repository":"Cortex","query":"opaque-query-token","attribution":{"client":"reported-client","session_id":"real-session"}});
    let first = read(&store, args.clone());
    let second = read(&store, args);
    assert_ne!(first["query_id"], second["query_id"]);
    assert_ne!(first["delivery_id"], second["delivery_id"]);
    assert_eq!(first["delivery_consumption"], "not_observed");
    let audit = receipt(&store, &first);
    let AuditPayload::Delivery(d) = &audit.payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.origin, AuditOrigin::Mcp);
    assert_eq!(d.query_id, first["query_id"].as_str().unwrap());
    assert_eq!(d.query_sha256, hash("opaque-query-token"));
    assert_eq!(d.context_sha256, hash(first["context"].as_str().unwrap()));
    assert_eq!(d.context_bytes, first["context"].as_str().unwrap().len());
    assert_eq!(
        d.references,
        vec![AuditReference::from_record(&original, false).unwrap()]
    );
    assert_eq!(d.attribution.client.as_deref(), Some("reported-client"));
    assert_eq!(d.attribution.session_id.as_deref(), Some("real-session"));
    assert!(d.attribution.turn_id.is_none());
    assert!(d.attribution.event_id.is_none());
    assert!(d.attribution.agent_id.is_none());
    let serialized = serde_json::to_string(&audit).unwrap();
    for private in [
        "opaque-query-token",
        "original body",
        "fixture-secret",
        "source-payload-fixture",
        "https://example.invalid",
    ] {
        assert!(!serialized.contains(private));
    }
    store
        .revise(
            "Cortex",
            "memory",
            1,
            RevisionRequest {
                title: "changed".into(),
                body: "new revision".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: Value::Null,
            },
        )
        .unwrap();
    store.reindex().unwrap();
    assert_eq!(store.get_audit("Cortex", &audit.id).unwrap(), Some(audit));
}

#[test]
fn clipped_body_is_delivered_but_omitted_citation_is_not_a_reference() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "short", &"body".repeat(1000));
    let response = read(&store, json!({"repository":"Cortex","budget_bytes":512}));
    let audit = receipt(&store, &response);
    let AuditPayload::Delivery(d) = audit.payload else {
        panic!("delivery expected")
    };
    assert_eq!(
        d.references,
        vec![AuditReference::from_record(&record, true).unwrap()]
    );
    assert_eq!(d.omitted_count, 0);
    assert_eq!(d.outcome, PreparationOutcome::Prepared);
    assert!(response["context"].as_str().unwrap().len() <= 512);
    store
        .revise(
            "Cortex",
            "memory",
            1,
            RevisionRequest {
                title: "title".repeat(1000),
                body: "body".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: Value::Null,
            },
        )
        .unwrap();
    let response = read(&store, json!({"repository":"Cortex","budget_bytes":256}));
    let audit = receipt(&store, &response);
    let AuditPayload::Delivery(d) = audit.payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.selected_count, 1);
    assert_eq!(d.omitted_count, 1);
    assert!(d.references.is_empty());
    assert_eq!(d.outcome, PreparationOutcome::Empty);
    assert!(d.context_bytes > 0 && d.context_bytes <= 256);
    assert_eq!(audit.evidence_label, EvidenceLabel::Unknown);
    assert!(!response["context"].as_str().unwrap().contains("[memory@"));
}

#[test]
fn json_rpc_request_id_never_becomes_native_attribution() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    memory(&store, "short", "body");
    let input = format!(
        "{}\n",
        json!({"jsonrpc":"2.0","id":"fake-session-turn-event","method":"tools/call","params":{"name":"cortex_pre_thinking","arguments":{"repository":"Cortex"}}})
    );
    let mut output = Vec::new();
    mcp::run_io(&store, None, input.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["id"], "fake-session-turn-event");
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["isError"], false);
    let audits = list(&store);
    assert_eq!(audits.len(), 1);
    let AuditPayload::Delivery(d) = &audits[0].payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.attribution, AuditAttribution::default());
    assert!(!serde_json::to_string(&audits[0])
        .unwrap()
        .contains("fake-session-turn-event"));
}

#[test]
fn audit_projection_failure_keeps_context_and_reports_unacknowledged_receipt() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    memory(&store, "short", "body");
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_audit BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT,'fixture audit failure'); END;").unwrap();
    drop(connection);
    let response = read(&store, json!({"repository":"Cortex"}));
    assert!(response["context"].as_str().unwrap().contains("memory@1"));
    assert_eq!(response["audit_persisted"], false);
    assert!(response["delivery_id"].is_null());
    assert_eq!(response["delivery_consumption"], "not_observed");
    assert!(root.path().join("pending").exists());
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_audit;")
        .unwrap();
    drop(connection);
    let recovered = LocalStore::open(root.path()).unwrap();
    assert_eq!(list(&recovered).len(), 1);
}

#[test]
fn invalid_preparation_budget_persists_error_when_store_is_healthy() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    assert!(mcp::call(
        &store,
        "cortex_pre_thinking",
        json!({"repository":"Cortex","budget_bytes":255}),
        None
    )
    .is_err());
    let audits = list(&store);
    assert_eq!(audits.len(), 1);
    let AuditPayload::Delivery(d) = &audits[0].payload else {
        panic!("delivery expected")
    };
    assert_eq!(d.outcome, PreparationOutcome::Error);
    assert_eq!(d.error, Some(AuditErrorCode::PreparationFailed));
    assert_eq!(d.context_bytes, 0);
    assert!(d.references.is_empty());
    assert_eq!(audits[0].evidence_label, EvidenceLabel::Unknown);
}
