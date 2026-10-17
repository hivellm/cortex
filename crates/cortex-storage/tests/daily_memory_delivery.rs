use chrono::Utc;
use cortex_storage::daily_memory::{
    MemoryCapture, MemoryCategory, MemoryDeliveredItem, MemoryDeliveryRequest, MemoryDeliveryStage,
    MemoryError, MemoryQuery, MemoryScope,
};
use cortex_storage::MetadataStore;

fn prepared_request(store: &MetadataStore) -> MemoryDeliveryRequest {
    let found = store
        .query_daily_memories(&MemoryQuery::new("repo", "durableguard"))
        .unwrap();
    MemoryDeliveryRequest {
        repo: "repo".into(),
        caller: "test-transport".into(),
        revision: found.revision,
        global_revision: found.global_revision,
        memories: found
            .records
            .into_iter()
            .map(|record| MemoryDeliveredItem {
                scope: record.scope,
                id: record.id,
                version: record.version,
                event_id: record.event_id,
            })
            .collect(),
        context_bytes: 200,
        channel: "http".into(),
    }
}

#[test]
fn prepared_receipt_and_version_linked_feedback_survive_correction_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let original = store
        .capture_daily_memory(&MemoryCapture::new(
            "repo",
            "durableguard",
            "Original sourced lesson.",
            MemoryCategory::Lesson,
        ))
        .unwrap();
    let request = prepared_request(&store);
    let delivery = store.record_daily_memory_delivery(&request).unwrap();
    assert_eq!(delivery.stage, MemoryDeliveryStage::Prepared);
    assert_eq!(
        store
            .daily_memory_revision(&MemoryScope::Repository("repo".into()))
            .unwrap(),
        original.scope_revision
    );
    store
        .upsert_pre_thinking_feedback(
            &delivery.delivery_id,
            Some("memory"),
            true,
            "[]",
            Some(5),
            Some("Prepared context contained the correction."),
            None,
            Utc::now(),
        )
        .unwrap();
    let mut correction = MemoryCapture::new(
        "repo",
        "durableguard successor",
        "Corrected sourced lesson.",
        MemoryCategory::Lesson,
    );
    correction.supersedes = Some(original.id.clone());
    correction.expected_revision = Some(original.scope_revision);
    store.capture_daily_memory(&correction).unwrap();
    drop(store);

    let reopened = MetadataStore::open(&path).unwrap();
    let persisted = reopened
        .daily_memory_delivery("repo", &delivery.delivery_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, delivery);
    assert_eq!(
        reopened
            .daily_memory_delivery_by_id(&delivery.delivery_id)
            .unwrap(),
        Some(delivery.clone())
    );
    assert_eq!(persisted.request.memories[0].id, original.id);
    assert_eq!(persisted.request.memories[0].version, original.version);
    assert!(reopened
        .pre_thinking_feedback(&delivery.delivery_id)
        .unwrap()
        .is_some());
    assert!(reopened
        .daily_memory_delivery("other", &delivery.delivery_id)
        .unwrap()
        .is_none());
}

#[test]
fn stale_snapshot_cannot_receive_a_new_prepared_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut reader = MetadataStore::open(&path).unwrap();
    let original = reader
        .capture_daily_memory(&MemoryCapture::new(
            "repo",
            "durableguard",
            "Original lesson.",
            MemoryCategory::Lesson,
        ))
        .unwrap();
    let request = prepared_request(&reader);
    let mut writer = MetadataStore::open(&path).unwrap();
    let mut correction = MemoryCapture::new(
        "repo",
        "durableguard",
        "Successor lesson.",
        MemoryCategory::Lesson,
    );
    correction.expected_revision = Some(original.scope_revision);
    correction.supersedes = Some(original.id);
    writer.capture_daily_memory(&correction).unwrap();
    assert!(matches!(
        reader.record_daily_memory_delivery(&request),
        Err(MemoryError::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    let count: i64 = reader
        .conn()
        .query_row("SELECT count(*) FROM daily_memory_deliveries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn receipt_rejects_wrong_scope_version_duplicates_and_over_budget_context() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&directory.path().join("metadata.sqlite")).unwrap();
    store
        .capture_daily_memory(&MemoryCapture::new(
            "repo",
            "durableguard",
            "Applicable lesson.",
            MemoryCategory::Lesson,
        ))
        .unwrap();
    let valid = prepared_request(&store);
    let mut invalid = valid.clone();
    invalid.memories[0].scope = MemoryScope::Global;
    assert!(store.record_daily_memory_delivery(&invalid).is_err());
    invalid = valid.clone();
    invalid.memories[0].version += 1;
    assert!(store.record_daily_memory_delivery(&invalid).is_err());
    invalid = valid.clone();
    invalid.memories.push(invalid.memories[0].clone());
    assert!(store.record_daily_memory_delivery(&invalid).is_err());
    invalid = valid.clone();
    invalid.context_bytes = 1537;
    assert!(store.record_daily_memory_delivery(&invalid).is_err());
    invalid = valid.clone();
    invalid.memories.clear();
    assert!(store.record_daily_memory_delivery(&invalid).is_err());
    assert!(store.record_daily_memory_delivery(&valid).is_ok());
    assert_eq!(
        store
            .daily_memory_revision(&MemoryScope::Repository("repo".into()))
            .unwrap(),
        1
    );
}

#[test]
fn global_receipt_requires_opt_in_and_current_independent_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let mut capture = MemoryCapture::new(
        "repo",
        "durableguard",
        "Repository lesson.",
        MemoryCategory::Lesson,
    );
    capture.id = Some("shared-id".into());
    store.capture_daily_memory(&capture).unwrap();
    capture.scope = MemoryScope::Global;
    capture.body = "Explicit global lesson.".into();
    let global = store.capture_daily_memory(&capture).unwrap();

    let mut query = MemoryQuery::new("repo", "durableguard");
    query.include_global = true;
    let found = store.query_daily_memories(&query).unwrap();
    let request = MemoryDeliveryRequest {
        repo: "repo".into(),
        caller: "authenticated-reader".into(),
        revision: found.revision,
        global_revision: found.global_revision,
        memories: found
            .records
            .into_iter()
            .map(|record| MemoryDeliveredItem {
                scope: record.scope,
                id: record.id,
                version: record.version,
                event_id: record.event_id,
            })
            .collect(),
        context_bytes: 400,
        channel: "http".into(),
    };
    assert_eq!(request.memories.len(), 2);
    let mut no_global_opt_in = request.clone();
    no_global_opt_in.global_revision = None;
    assert!(matches!(
        store.record_daily_memory_delivery(&no_global_opt_in),
        Err(MemoryError::InvalidInput(_))
    ));
    let delivery = store.record_daily_memory_delivery(&request).unwrap();

    let mut writer = MetadataStore::open(&path).unwrap();
    capture.id = None;
    capture.body = "Corrected global lesson.".into();
    capture.supersedes = Some(global.id);
    capture.expected_revision = Some(global.scope_revision);
    writer.capture_daily_memory(&capture).unwrap();
    assert_eq!(
        store
            .daily_memory_revision(&MemoryScope::Repository("repo".into()))
            .unwrap(),
        request.revision
    );
    assert!(matches!(
        store.record_daily_memory_delivery(&request),
        Err(MemoryError::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(
        store
            .daily_memory_delivery("repo", &delivery.delivery_id)
            .unwrap(),
        Some(delivery)
    );
}

#[test]
fn bounded_open_applies_lock_timeout_before_schema_migration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    let result = MetadataStore::open_with_busy_timeout(&path, std::time::Duration::from_millis(25));
    assert!(
        result.is_err(),
        "competing migration must observe the held lock"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    connection.execute_batch("ROLLBACK").unwrap();
    assert!(
        MetadataStore::open_with_busy_timeout(&path, std::time::Duration::from_millis(25)).is_ok()
    );
}

#[test]
fn observed_legacy_capture_kind_survives_restart_and_participates_in_retry_identity() {
    use cortex_core::events::Kind;
    use cortex_storage::daily_memory::MemoryRetryKey;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let mut capture = MemoryCapture::new(
        "repo",
        "legacyguard",
        "Explicit legacy body.",
        MemoryCategory::Lesson,
    );
    capture.capture_kind = Some(Kind::Learning);
    capture.retry = Some(MemoryRetryKey {
        caller: "transport:test".into(),
        key: "retry".into(),
    });
    let receipt = store.capture_daily_memory(&capture).unwrap();
    drop(store);
    let mut store = MetadataStore::open(&path).unwrap();
    let scope = MemoryScope::Repository("repo".into());
    assert_eq!(
        store
            .daily_memory(&scope, &receipt.id)
            .unwrap()
            .unwrap()
            .capture_kind,
        Some(Kind::Learning)
    );
    let evidence = store.daily_memory_evidence(&scope, &receipt.id).unwrap();
    assert_eq!(evidence[0].kind, Kind::Decision);
    assert_eq!(evidence[0].capture_kind, Some(Kind::Learning));
    assert_eq!(store.capture_daily_memory(&capture).unwrap(), receipt);
    capture.capture_kind = Some(Kind::Memory);
    assert!(matches!(
        store.capture_daily_memory(&capture),
        Err(MemoryError::IdempotencyConflict)
    ));
}
