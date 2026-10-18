use cortex_local::*;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn audit_path(store: &LocalStore, record: &AuditRecord) -> PathBuf {
    store
        .root()
        .join("audit")
        .join(hash(record.repository.as_bytes()))
        .join(format!("{}.json", hash(record.id.as_bytes())))
}

fn populate(store: &LocalStore) -> (Record, AuditRecord) {
    let memory = store
        .capture(CaptureRequest {
            id: Some("original-memory".into()),
            repository: "Cortex".into(),
            kind: RecordKind::Lesson,
            title: "Original revision".into(),
            body: "original durable evidence".into(),
            source: Source {
                kind: "fixture".into(),
                locator: "audit-recovery-test".into(),
                payload: serde_json::json!({"original":true}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record;
    let audit = store
        .append_audit(AuditRequest {
            id: Some("original-delivery".into()),
            repository: "Cortex".into(),
            payload: AuditPayload::Delivery(DeliveryDraft {
                query_id: "original-query".into(),
                origin: AuditOrigin::Mcp,
                attribution: AuditAttribution::default(),
                query_sha256: hash(b"query"),
                context_sha256: hash(b"context"),
                context_bytes: 7,
                selected_count: 1,
                omitted_count: 0,
                references: vec![AuditReference::from_record(&memory, false).unwrap()],
                outcome: PreparationOutcome::Prepared,
                latency_us: 100,
                error: None,
            }),
        })
        .unwrap()
        .record;
    (memory, audit)
}

fn reseal(record: &mut AuditRecord) -> Vec<u8> {
    record.content_sha256.clear();
    record.content_sha256 = hash(
        &cortex_core::canonical_json::canonicalize(&serde_json::to_value(&*record).unwrap())
            .unwrap(),
    );
    cortex_core::canonical_json::canonicalize(&serde_json::to_value(record).unwrap()).unwrap()
}

#[test]
fn historical_delivery_survives_revision_restart_reindex_and_repair() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (original, audit) = populate(&store);
    let canonical_path = audit_path(&store, &audit);
    let canonical = fs::read(&canonical_path).unwrap();
    let updated = store
        .revise(
            "Cortex",
            &original.id,
            1,
            RevisionRequest {
                title: "New revision".into(),
                body: "different current evidence".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: serde_json::Value::Null,
            },
        )
        .unwrap()
        .record;
    assert_eq!(updated.revision, 2);
    drop(store);
    let store = LocalStore::open(root.path()).unwrap();
    let result = store.reindex().unwrap();
    assert_eq!(result.records, 1);
    assert_eq!(result.revisions, 2);
    assert_eq!(
        store.get_audit("Cortex", &audit.id).unwrap(),
        Some(audit.clone())
    );
    assert_eq!(
        store.get_revision("Cortex", &original.id, 1).unwrap(),
        Some(original.clone())
    );
    assert_eq!(fs::read(&canonical_path).unwrap(), canonical);
    drop(store);

    fs::write(
        root.path().join("index.sqlite3"),
        b"corrupt projection fixture",
    )
    .unwrap();
    LocalStore::repair(root.path()).unwrap();
    let repaired = LocalStore::open(root.path()).unwrap();
    assert_eq!(
        repaired.get_audit("Cortex", &audit.id).unwrap(),
        Some(audit)
    );
    assert_eq!(
        repaired.get_revision("Cortex", &original.id, 1).unwrap(),
        Some(original)
    );
    assert_eq!(
        repaired.get("Cortex", "original-memory").unwrap(),
        Some(updated)
    );
    assert_eq!(fs::read(canonical_path).unwrap(), canonical);
}

#[test]
fn invalid_canonical_audit_keeps_working_projection_and_precise_diagnostics() {
    for mutation in [
        "json",
        "oversized",
        "hash",
        "label",
        "foreign",
        "revision",
        "source",
        "timestamp",
        "path",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let (memory, original) = populate(&store);
        let mut path = audit_path(&store, &original);
        let mut changed = original.clone();
        let bytes = match mutation {
            "json" => b"{ invalid audit json".to_vec(),
            "oversized" => vec![b' '; audit::MAX_AUDIT_BYTES + 1],
            "hash" => {
                changed.content_sha256 = "0".repeat(64);
                serde_json::to_vec(&changed).unwrap()
            }
            "label" => {
                changed.evidence_label = EvidenceLabel::CapturedCitation;
                changed.verification = Some(Verification::CanonicalEventCitation);
                reseal(&mut changed)
            }
            "timestamp" => {
                changed.recorded_at = "2026-10-18T12:00:00-03:00".into();
                reseal(&mut changed)
            }
            "foreign" | "revision" | "source" => {
                let AuditPayload::Delivery(draft) = &mut changed.payload else {
                    unreachable!()
                };
                match mutation {
                    "foreign" => draft.references[0].repository = "cortex".into(),
                    "revision" => draft.references[0].revision = 2,
                    "source" => draft.references[0].source_sha256 = "0".repeat(64),
                    _ => unreachable!(),
                }
                reseal(&mut changed)
            }
            "path" => {
                let moved = path.with_file_name("wrong-identity.json");
                fs::rename(&path, &moved).unwrap();
                path = moved;
                fs::read(&path).unwrap()
            }
            _ => unreachable!(),
        };
        fs::write(&path, &bytes).unwrap();
        let error = store.reindex().unwrap_err().to_string();
        assert!(
            error.contains(&path.display().to_string()),
            "{mutation}: {error}"
        );
        assert_eq!(
            store.get_audit("Cortex", &original.id).unwrap(),
            Some(original)
        );
        assert_eq!(store.get("Cortex", &memory.id).unwrap(), Some(memory));
        assert_eq!(fs::read(&path).unwrap(), bytes);

        let index_path = root.path().join("index.sqlite3");
        let previous_index = fs::read(&index_path).unwrap();
        let error = LocalStore::repair(root.path()).unwrap_err().to_string();
        assert!(
            error.contains(&path.display().to_string()),
            "{mutation}: {error}"
        );
        assert_eq!(fs::read(index_path).unwrap(), previous_index);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
