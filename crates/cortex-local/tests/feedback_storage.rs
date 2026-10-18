use cortex_local::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn seed(store: &LocalStore, repository: &str) -> (Record, FeedbackRequest) {
    let memory = store
        .capture(CaptureRequest {
            id: Some("memory".into()),
            repository: repository.into(),
            kind: RecordKind::Lesson,
            title: "Retained original".into(),
            body: "original guidance".into(),
            source: Source {
                kind: "fixture".into(),
                locator: "source-fixture".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record;
    let response = recall::prepare(
        store,
        recall::RecallRequest {
            repository: repository.into(),
            query: String::new(),
            limit: 5,
            budget_bytes: 6144,
            origin: AuditOrigin::Mcp,
            attribution: AuditAttribution::default(),
            fallback_recent: false,
        },
    )
    .unwrap();
    let request = FeedbackRequest {
        id: "feedback".into(),
        repository: repository.into(),
        source: FeedbackSource {
            kind: "user".into(),
            reference: "explicit review".into(),
            actor: "declared-human".into(),
        },
        evidence: FeedbackEvidence::Explicit,
        judgment: FeedbackJudgment::Helpful,
        query_id: Some(response.query_id),
        delivery_id: response.delivery_id,
        reference: Some(AuditReference::from_record(&memory, false).unwrap()),
        comment: Some("Useful exact revision".into()),
    };
    (memory, request)
}
fn canonical_path(root: &Path, record: &FeedbackRecord) -> PathBuf {
    root.join("feedback")
        .join(hash(record.request.repository.as_bytes()))
        .join(format!("{}.json", hash(record.request.id.as_bytes())))
}
fn reseal(record: &mut FeedbackRecord) -> Vec<u8> {
    record.content_sha256.clear();
    record.content_sha256 = hash(
        &cortex_core::canonical_json::canonicalize(&serde_json::to_value(&*record).unwrap())
            .unwrap(),
    );
    cortex_core::canonical_json::canonicalize(&serde_json::to_value(record).unwrap()).unwrap()
}
fn list(
    store: &LocalStore,
    repository: &str,
    limit: usize,
    cursor: Option<String>,
) -> Result<FeedbackListPage> {
    store.list_feedback(FeedbackListQuery {
        repository: repository.into(),
        limit,
        cursor,
    })
}

#[test]
fn immutable_retries_historical_references_and_scope_survive_restart_reindex() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (original, request) = seed(&store, "Cortex");
    let receipt = store
        .get_audit("Cortex", request.delivery_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let first = store.record_feedback(request.clone()).unwrap();
    assert!(first.created);
    assert_eq!(first.entry.eligibility, FeedbackEligibility::Eligible);
    let path = canonical_path(root.path(), &first.entry.record);
    let bytes = fs::read(&path).unwrap();
    let retry = store.record_feedback(request.clone()).unwrap();
    assert!(!retry.created);
    assert_eq!(retry.entry.record, first.entry.record);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut changed = request.clone();
    changed.comment = Some("different".into());
    assert!(matches!(
        store.record_feedback(changed),
        Err(LocalError::Conflict(_))
    ));
    let revised = store
        .revise(
            "Cortex",
            "memory",
            1,
            RevisionRequest {
                title: "new".into(),
                body: "new guidance".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: json!({}),
            },
        )
        .unwrap()
        .record;
    assert_eq!(revised.revision, 2);
    drop(store);
    let store = LocalStore::open(root.path()).unwrap();
    store.reindex().unwrap();
    assert_eq!(
        store
            .get_feedback("Cortex", "feedback")
            .unwrap()
            .unwrap()
            .record,
        first.entry.record
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        store.get_revision("Cortex", "memory", 1).unwrap(),
        Some(original)
    );
    assert_eq!(
        store.get_audit("Cortex", &receipt.id).unwrap(),
        Some(receipt)
    );
    assert!(store.get_feedback("cortex", "feedback").unwrap().is_none());
    assert_eq!(store.status(Some("Cortex")).unwrap().records, 1);
    let (_, foreign) = seed(&store, "cortex");
    store.record_feedback(foreign).unwrap();
    assert_eq!(list(&store, "Cortex", 100, None).unwrap().entries.len(), 1);
    assert_eq!(list(&store, "cortex", 100, None).unwrap().entries.len(), 1);
}

#[test]
fn feedback_requires_exact_delivery_query_and_rendered_historical_fields() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    for field in [
        "scope",
        "query",
        "delivery",
        "revision",
        "hash",
        "source",
        "clipping",
        "reference_scope",
        "legacy_scope",
        "legacy_evidence",
        "source_kind",
        "missing_query",
        "missing_delivery",
    ] {
        let mut bad = request.clone();
        match field {
            "scope" => bad.repository = "cortex".into(),
            "query" => bad.query_id = Some("wrong".into()),
            "delivery" => bad.delivery_id = Some("wrong".into()),
            "revision" => bad.reference.as_mut().unwrap().revision = 2,
            "hash" => bad.reference.as_mut().unwrap().content_sha256 = "0".repeat(64),
            "source" => bad.reference.as_mut().unwrap().source_sha256 = "0".repeat(64),
            "clipping" => bad.reference.as_mut().unwrap().body_clipped = true,
            "reference_scope" => bad.reference.as_mut().unwrap().repository = "cortex".into(),
            "legacy_scope" => bad.repository = LEGACY_FEEDBACK_REPOSITORY.into(),
            "legacy_evidence" => bad.evidence = FeedbackEvidence::LegacyUnknown,
            "source_kind" => bad.source.kind = "legacy_sqlite".into(),
            "missing_query" => bad.query_id = None,
            "missing_delivery" => bad.delivery_id = None,
            _ => unreachable!(),
        }
        assert!(store.record_feedback(bad).is_err(), "{field}");
    }
    assert!(list(&store, "Cortex", 100, None)
        .unwrap()
        .entries
        .is_empty());
    for (id, expected) in [
        ("inferred", FeedbackEligibility::Inferred),
        ("unknown", FeedbackEligibility::UnknownJudgment),
        ("bundle", FeedbackEligibility::Bundle),
    ] {
        let mut candidate = request.clone();
        candidate.id = id.into();
        match id {
            "inferred" => candidate.evidence = FeedbackEvidence::Inferred,
            "unknown" => candidate.judgment = FeedbackJudgment::Unknown,
            _ => candidate.reference = None,
        }
        assert_eq!(
            store.record_feedback(candidate).unwrap().entry.eligibility,
            expected
        );
    }
    store
        .retain_audit(AuditRetentionRequest {
            repository: "Cortex".into(),
            cutoff: "9999-01-01T00:00:00Z".into(),
            limit: 100,
            apply: true,
        })
        .unwrap();
    assert_eq!(
        store.record_feedback(request).unwrap().entry.eligibility,
        FeedbackEligibility::Eligible
    );
}

#[test]
fn declared_actor_duplicates_conflicts_and_query_hashes_have_honest_eligibility() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    store.record_feedback(request.clone()).unwrap();
    let mut unknown = request.clone();
    unknown.id = "unknown".into();
    unknown.judgment = FeedbackJudgment::Unknown;
    store.record_feedback(unknown).unwrap();
    let mut inferred = request.clone();
    inferred.id = "inferred".into();
    inferred.evidence = FeedbackEvidence::Inferred;
    inferred.judgment = FeedbackJudgment::Unhelpful;
    store.record_feedback(inferred).unwrap();
    assert_eq!(
        store
            .get_feedback("Cortex", "feedback")
            .unwrap()
            .unwrap()
            .eligibility,
        FeedbackEligibility::Eligible
    );
    let mut conflict = request.clone();
    conflict.id = "other-actor".into();
    conflict.source.actor = "another-declared-human".into();
    conflict.judgment = FeedbackJudgment::Unhelpful;
    assert_eq!(
        store.record_feedback(conflict).unwrap().entry.eligibility,
        FeedbackEligibility::ConflictingJudgments
    );
    assert_eq!(
        store
            .get_feedback("Cortex", "feedback")
            .unwrap()
            .unwrap()
            .eligibility,
        FeedbackEligibility::ConflictingJudgments
    );
    let receipt = store
        .get_audit("Cortex", request.delivery_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let AuditPayload::Delivery(mut draft) = receipt.payload else {
        panic!("delivery required")
    };
    draft.query_sha256 = "f".repeat(64);
    store
        .append_audit(AuditRequest {
            id: Some("inconsistent-delivery".into()),
            repository: "Cortex".into(),
            payload: AuditPayload::Delivery(draft),
        })
        .unwrap();
    assert_eq!(
        store
            .get_feedback("Cortex", "feedback")
            .unwrap()
            .unwrap()
            .eligibility,
        FeedbackEligibility::InconsistentQueryHashes
    );
    let first = list(&store, "Cortex", 2, None).unwrap();
    assert_eq!(first.entries.len(), 2);
    let second = list(&store, "Cortex", 2, first.next_cursor).unwrap();
    assert_eq!(second.entries.len(), 2);
    assert!(second.next_cursor.is_none());
    for limit in [0, 101] {
        assert!(list(&store, "Cortex", limit, None).is_err());
    }
    assert!(list(&store, "cortex", 10, Some("feedback".into())).is_err());
}

#[test]
fn failed_projection_recovers_published_feedback_without_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_feedback BEFORE INSERT ON feedback_records BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    drop(connection);
    assert!(store.record_feedback(request.clone()).is_err());
    assert!(root.path().join("pending").exists());
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_feedback;")
        .unwrap();
    drop(connection);
    let store = LocalStore::open(root.path()).unwrap();
    let recovered = store.get_feedback("Cortex", "feedback").unwrap().unwrap();
    assert!(!root.path().join("pending").exists());
    let bytes = fs::read(canonical_path(root.path(), &recovered.record)).unwrap();
    let retry = store.record_feedback(request).unwrap();
    assert!(!retry.created);
    assert_eq!(retry.entry.record, recovered.record);
    assert_eq!(
        fs::read(canonical_path(root.path(), &retry.entry.record)).unwrap(),
        bytes
    );
}

#[test]
fn corrupt_feedback_rebuild_preserves_working_projection_and_reports_path() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    let written = store.record_feedback(request).unwrap().entry.record;
    let path = canonical_path(root.path(), &written);
    let original = fs::read(&path).unwrap();
    for case in ["json", "size", "hash", "timestamp", "reference"] {
        let mut changed = written.clone();
        let bytes = match case {
            "json" => b"{broken".to_vec(),
            "size" => vec![b' '; MAX_FEEDBACK_BYTES + 1],
            "hash" => {
                changed.content_sha256 = "0".repeat(64);
                serde_json::to_vec(&changed).unwrap()
            }
            "timestamp" => {
                changed.recorded_at = "2000-01-01T00:00:00Z".into();
                reseal(&mut changed)
            }
            _ => {
                changed.request.reference.as_mut().unwrap().revision = 99;
                reseal(&mut changed)
            }
        };
        fs::write(&path, bytes).unwrap();
        let error = store.reindex().unwrap_err().to_string();
        assert!(
            error.contains(&path.display().to_string()),
            "{case}: {error}"
        );
        let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
        let projected: String = connection
            .query_row(
                "SELECT record FROM feedback_records WHERE id='feedback'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<FeedbackRecord>(&projected).unwrap(),
            written
        );
        drop(connection);
        fs::write(&path, &original).unwrap();
        store.reindex().unwrap();
    }
    let wrong = path.with_file_name("wrong.json");
    fs::rename(&path, &wrong).unwrap();
    assert!(store
        .reindex()
        .unwrap_err()
        .to_string()
        .contains("path mismatch"));
    fs::rename(&wrong, &path).unwrap();
    store.reindex().unwrap();
}

#[test]
fn feedback_field_bounds_fail_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    for field in ["id", "kind", "reference", "actor", "comment"] {
        let mut candidate = request.clone();
        match field {
            "id" => candidate.id = "x".repeat(257),
            "kind" => candidate.source.kind = "x".repeat(129),
            "reference" => candidate.source.reference = "x".repeat(2049),
            "actor" => candidate.source.actor = "x".repeat(257),
            _ => candidate.comment = Some("x".repeat(4097)),
        };
        assert!(store.record_feedback(candidate).is_err(), "{field}");
    }
    assert!(!root.path().join("feedback").exists());
}

#[test]
fn retry_after_clock_regression_keeps_original_record_but_rejects_new_feedback() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    let mut written = store.record_feedback(request.clone()).unwrap().entry.record;
    let mut delivery = store
        .get_audit("Cortex", request.delivery_id.as_deref().unwrap())
        .unwrap()
        .unwrap();

    // Valid future evidence models a clock correction without changing the host clock.
    delivery.recorded_at = "9998-01-01T00:00:00Z".into();
    delivery.content_sha256.clear();
    delivery.content_sha256 = hash(
        &cortex_core::canonical_json::canonicalize(&serde_json::to_value(&delivery).unwrap())
            .unwrap(),
    );
    let delivery_path = root
        .path()
        .join("audit")
        .join(hash(b"Cortex"))
        .join(format!("{}.json", hash(delivery.id.as_bytes())));
    let delivery_bytes =
        cortex_core::canonical_json::canonicalize(&serde_json::to_value(&delivery).unwrap())
            .unwrap();
    fs::write(&delivery_path, &delivery_bytes).unwrap();
    written.recorded_at = "9998-01-01T00:00:01Z".into();
    let path = canonical_path(root.path(), &written);
    let bytes = reseal(&mut written);
    fs::write(&path, &bytes).unwrap();
    store.reindex().unwrap();

    let retry = store.record_feedback(request.clone()).unwrap();
    assert!(!retry.created);
    assert_eq!(retry.entry.record, written);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read(&delivery_path).unwrap(), delivery_bytes);

    let mut fresh = request;
    fresh.id = "fresh-after-clock-regression".into();
    let error = store.record_feedback(fresh).unwrap_err().to_string();
    assert!(error.contains("feedback predates delivery"), "{error}");
    assert!(store
        .get_feedback("Cortex", "fresh-after-clock-regression")
        .unwrap()
        .is_none());
    assert_eq!(list(&store, "Cortex", 100, None).unwrap().entries.len(), 1);
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(fs::read(delivery_path).unwrap(), delivery_bytes);
}

#[cfg(unix)]
#[test]
fn feedback_rebuild_rejects_symlink_without_reading_external_payload() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let (_, request) = seed(&store, "Cortex");
    let record = store.record_feedback(request).unwrap().entry.record;
    let path = canonical_path(root.path(), &record);
    let external = tempfile::tempdir().unwrap();
    let target = external.path().join("preserved.json");
    fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(store.reindex().is_err());
    assert!(target.exists());
}
