use cortex_local::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn attribution() -> AuditAttribution {
    AuditAttribution {
        client: Some("codex".into()),
        session_id: Some("session".into()),
        turn_id: Some("turn".into()),
        ..Default::default()
    }
}
fn memory(store: &LocalStore, repository: &str) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("memory".into()),
            repository: repository.into(),
            body: "retained source".into(),
            source: Source {
                kind: "fixture".into(),
                locator: "original-import".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}
fn delivery(
    store: &LocalStore,
    memory: &Record,
    id: &str,
    outcome: PreparationOutcome,
) -> AuditRecord {
    let prepared = outcome == PreparationOutcome::Prepared;
    store
        .append_audit(AuditRequest {
            id: Some(id.into()),
            repository: memory.repository.clone(),
            payload: AuditPayload::Delivery(DeliveryDraft {
                query_id: format!("query-{id}"),
                origin: AuditOrigin::Mcp,
                attribution: attribution(),
                query_sha256: hash(b"query"),
                context_sha256: hash(if prepared { b"context" } else { b"" }),
                context_bytes: if prepared { 7 } else { 0 },
                selected_count: usize::from(prepared),
                omitted_count: 0,
                references: if prepared {
                    vec![AuditReference::from_record(memory, false).unwrap()]
                } else {
                    Vec::new()
                },
                outcome,
                latency_us: 100,
                error: (outcome == PreparationOutcome::Error)
                    .then_some(AuditErrorCode::PreparationFailed),
            }),
        })
        .unwrap()
        .record
}
fn reported(store: &LocalStore, repository: &str, id: &str, delivery_id: &str) -> AuditRecord {
    store
        .append_audit(AuditRequest {
            id: Some(id.into()),
            repository: repository.into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: Some(delivery_id.into()),
                event: None,
                attribution: attribution(),
                method: LinkageMethod::ReportedUse,
                evidence: Vec::new(),
                uncertainty: Some(UncertaintyCode::ReportedOnly),
            }),
        })
        .unwrap()
        .record
}
fn query(repository: &str) -> AuditListQuery {
    AuditListQuery {
        repository: repository.into(),
        limit: 100,
        after: None,
        before: None,
        kind: None,
        cursor: None,
        include_archived: false,
    }
}
fn retention(repository: &str, apply: bool) -> AuditRetentionRequest {
    AuditRetentionRequest {
        repository: repository.into(),
        cutoff: "9999-01-01T00:00:00Z".into(),
        limit: 100,
        apply,
    }
}
fn canonical_path(store: &LocalStore, record: &AuditRecord) -> PathBuf {
    store
        .root()
        .join("audit")
        .join(hash(record.repository.as_bytes()))
        .join(format!("{}.json", hash(record.id.as_bytes())))
}
fn set_time(store: &LocalStore, mut record: AuditRecord, time: &str) {
    record.recorded_at = time.into();
    record.content_sha256.clear();
    record.content_sha256 = hash(
        &cortex_core::canonical_json::canonicalize(&serde_json::to_value(&record).unwrap())
            .unwrap(),
    );
    fs::write(
        canonical_path(store, &record),
        cortex_core::canonical_json::canonicalize(&serde_json::to_value(&record).unwrap()).unwrap(),
    )
    .unwrap();
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path)
            .unwrap_or_else(|error| panic!("snapshot enumerate {}: {error}", path.display()))
        {
            let path = entry
                .unwrap_or_else(|error| panic!("snapshot entry in {}: {error}", path.display()))
                .path();
            if path.is_dir() {
                walk(root, &path, result);
            } else if !(path.parent() == Some(root)
                && (path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("index.sqlite3")
                    || path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .eq_ignore_ascii_case("store.lock")))
            {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(&path).unwrap_or_else(|error| {
                        panic!(
                            "snapshot read {}: {error}; metadata now: {:?}",
                            path.display(),
                            fs::symlink_metadata(&path)
                        )
                    }),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
fn ids(page: &AuditListPage) -> Vec<&str> {
    page.entries.iter().map(|e| e.record.id.as_str()).collect()
}

#[test]
fn list_orders_utc_instants_then_ids_and_pages_without_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let memory = memory(&store, "Cortex");
    for (id, time) in [
        ("b", "2026-01-01T00:00:00.100000000+00:00"),
        ("a", "2026-01-01T00:00:00.1Z"),
        ("z", "2026-01-01T00:00:00Z"),
        ("c", "2026-01-01T00:00:00.2Z"),
    ] {
        set_time(
            &store,
            delivery(&store, &memory, id, PreparationOutcome::Prepared),
            time,
        );
    }
    store.reindex().unwrap();
    let mut q = query("Cortex");
    q.limit = 2;
    let first = store.list_audit(q.clone()).unwrap();
    assert_eq!(ids(&first), vec!["c", "b"]);
    assert_eq!(first.next_cursor.as_deref(), Some("b"));
    q.cursor = first.next_cursor;
    let second = store.list_audit(q).unwrap();
    assert_eq!(ids(&second), vec!["a", "z"]);
    assert!(second.next_cursor.is_none());
    let mut q = query("Cortex");
    q.after = Some("2026-01-01T00:00:00.1Z".into());
    q.before = Some("2026-01-01T00:00:00.2+00:00".into());
    assert_eq!(ids(&store.list_audit(q).unwrap()), vec!["b", "a"]);
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    let normalized: String = connection
        .query_row(
            "SELECT recorded_at FROM audit_records WHERE id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(normalized, "2026-01-01T00:00:00.100000000Z");
}

#[test]
fn pagination_and_cutoff_preserve_sub_microsecond_instants() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    for (id, time) in [
        ("z-older", "2026-01-01T00:00:00.123456100Z"),
        ("a-newer", "2026-01-01T00:00:00.123456900Z"),
    ] {
        set_time(
            &store,
            delivery(&store, &record, id, PreparationOutcome::Prepared),
            time,
        );
    }
    store.reindex().unwrap();
    let mut q = query("Cortex");
    q.limit = 1;
    let first = store.list_audit(q.clone()).unwrap();
    assert_eq!(ids(&first), vec!["a-newer"]);
    q.cursor = first.next_cursor;
    assert_eq!(ids(&store.list_audit(q).unwrap()), vec!["z-older"]);
    let mut q = query("Cortex");
    q.after = Some("2026-01-01T00:00:00.123456500Z".into());
    assert_eq!(ids(&store.list_audit(q).unwrap()), vec!["a-newer"]);
    let result = store
        .retain_audit(AuditRetentionRequest {
            repository: "Cortex".into(),
            cutoff: "2026-01-01T00:00:00.123456900Z".into(),
            limit: 100,
            apply: true,
        })
        .unwrap();
    assert_eq!(result.delivery_ids, vec!["z-older"]);
}

#[test]
fn list_rejects_invalid_bounds_and_cursor_scope_and_filters_payloads() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let upper = memory(&store, "Cortex");
    let lower = memory(&store, "cortex");
    delivery(&store, &upper, "upper", PreparationOutcome::Prepared);
    delivery(&store, &lower, "foreign", PreparationOutcome::Prepared);
    reported(&store, "Cortex", "observation", "upper");
    let mut q = query("Cortex");
    q.kind = Some(AuditKind::Observation);
    assert_eq!(ids(&store.list_audit(q).unwrap()), vec!["observation"]);
    for limit in [0, 101] {
        let mut q = query("Cortex");
        q.limit = limit;
        assert!(store.list_audit(q).is_err());
    }
    for scope in ["", "\n"] {
        assert!(store.list_audit(query(scope)).is_err());
    }
    for cursor in ["foreign", "missing"] {
        let mut q = query("Cortex");
        q.cursor = Some(cursor.into());
        assert!(store.list_audit(q).is_err());
    }
    for bad in ["garbage", "2026-01-01T01:00:00+01:00"] {
        let mut q = query("Cortex");
        q.after = Some(bad.into());
        assert!(store.list_audit(q).is_err());
    }
    let mut q = query("Cortex");
    q.after = Some("2026-01-02T00:00:00Z".into());
    q.before = Some("2026-01-01T00:00:00Z".into());
    assert!(store.list_audit(q).is_err());
    assert_eq!(
        ids(&store.list_audit(query("cortex")).unwrap()),
        vec!["foreign"]
    );
}

#[test]
fn retention_preview_is_read_only_and_apply_preserves_sources_and_archive_families() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    let foreign = memory(&store, "cortex");
    delivery(&store, &record, "first", PreparationOutcome::Prepared);
    delivery(&store, &record, "second", PreparationOutcome::Prepared);
    delivery(&store, &foreign, "foreign", PreparationOutcome::Prepared);
    reported(&store, "Cortex", "before", "first");
    fs::create_dir(root.path().join("imports")).unwrap();
    fs::write(
        root.path().join("imports/source.jsonl"),
        b"original imported evidence\n",
    )
    .unwrap();
    fs::write(
        root.path().join("imports/index.sqlite3-source.jsonl"),
        b"imported evidence whose name resembles an index\n",
    )
    .unwrap();
    let before = snapshot(root.path());
    assert_eq!(
        before
            .get(Path::new("imports/index.sqlite3-source.jsonl"))
            .map(Vec::as_slice),
        Some(b"imported evidence whose name resembles an index\n".as_slice()),
    );
    let state = store.status(Some("Cortex")).unwrap();
    let request: AuditRetentionRequest = serde_json::from_value(
        serde_json::json!({"repository":"Cortex","cutoff":"9999-01-01T00:00:00Z","limit":100}),
    )
    .unwrap();
    assert!(!request.apply);
    let preview = store.retain_audit(request).unwrap();
    assert_eq!(preview.delivery_ids.len(), 2);
    assert_eq!(preview.linked_observations, 1);
    assert_eq!(preview.historical_references, 1);
    assert!(preview.canonical_files_retained);
    assert!(preview.control.is_none());
    assert_eq!(snapshot(root.path()), before);
    let applied = store.retain_audit(retention("Cortex", true)).unwrap();
    assert_eq!(applied.delivery_ids, preview.delivery_ids);
    assert!(applied.control.is_some());
    for (path, bytes) in &before {
        assert_eq!(&fs::read(root.path().join(path)).unwrap(), bytes);
    }
    reported(&store, "Cortex", "after", "first");
    drop(store);
    let store = LocalStore::open(root.path()).unwrap();
    store.reindex().unwrap();
    let default = store.list_audit(query("Cortex")).unwrap();
    assert_eq!(default.entries.len(), 1);
    assert!(matches!(
        default.entries[0].record.payload,
        AuditPayload::ArchiveControl(_)
    ));
    assert!(!default.entries[0].archived);
    let mut q = query("Cortex");
    q.include_archived = true;
    let all = store.list_audit(q).unwrap();
    assert_eq!(all.entries.len(), 5);
    assert_eq!(all.entries.iter().filter(|e| e.archived).count(), 4);
    for id in ["first", "second", "before", "after"] {
        assert!(store.get_audit("Cortex", id).unwrap().is_some());
    }
    assert_eq!(
        ids(&store.list_audit(query("cortex")).unwrap()),
        vec!["foreign"]
    );
    assert_eq!(store.status(Some("Cortex")).unwrap().records, state.records);
    assert_eq!(store.get("Cortex", "memory").unwrap(), Some(record));
    let before_repeat = snapshot(root.path());
    let repeat = store.retain_audit(retention("Cortex", true)).unwrap();
    assert!(repeat.delivery_ids.is_empty());
    assert!(repeat.control.is_none());
    assert_eq!(snapshot(root.path()), before_repeat);
}

#[test]
fn retention_selection_is_bounded_and_cutoff_is_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    for (id, time) in [
        ("old", "2026-01-01T00:00:00Z"),
        ("middle", "2026-01-02T00:00:00Z"),
        ("cutoff", "2026-01-03T00:00:00Z"),
    ] {
        set_time(
            &store,
            delivery(&store, &record, id, PreparationOutcome::Prepared),
            time,
        );
    }
    store.reindex().unwrap();
    let request = AuditRetentionRequest {
        repository: "Cortex".into(),
        cutoff: "2026-01-03T00:00:00Z".into(),
        limit: 1,
        apply: true,
    };
    let first = store.retain_audit(request.clone()).unwrap();
    assert_eq!(first.delivery_ids, vec!["middle"]);
    let second = store.retain_audit(request).unwrap();
    assert_eq!(second.delivery_ids, vec!["old"]);
    let mut q = query("Cortex");
    q.kind = Some(AuditKind::Delivery);
    assert_eq!(ids(&store.list_audit(q).unwrap()), vec!["cutoff"]);
    for limit in [0, 101] {
        let mut request = retention("Cortex", false);
        request.limit = limit;
        assert!(store.retain_audit(request).is_err());
    }
}

#[test]
fn archive_controls_reject_missing_foreign_duplicate_non_delivery_and_new_targets() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    let foreign = memory(&store, "cortex");
    let receipt = delivery(&store, &record, "receipt", PreparationOutcome::Prepared);
    delivery(&store, &foreign, "foreign", PreparationOutcome::Prepared);
    reported(&store, "Cortex", "observation", "receipt");
    let before = snapshot(root.path());
    for targets in [
        vec![],
        vec!["missing"],
        vec!["foreign"],
        vec!["observation"],
        vec!["receipt", "receipt"],
        vec!["receipt"; 101],
    ] {
        assert!(store
            .append_audit(AuditRequest {
                id: None,
                repository: "Cortex".into(),
                payload: AuditPayload::ArchiveControl(ArchiveControlDraft {
                    delivery_ids: targets.into_iter().map(str::to_owned).collect(),
                    cutoff: "9999-01-01T00:00:00Z".into()
                })
            })
            .is_err());
    }
    assert!(store
        .append_audit(AuditRequest {
            id: None,
            repository: "Cortex".into(),
            payload: AuditPayload::ArchiveControl(ArchiveControlDraft {
                delivery_ids: vec!["receipt".into()],
                cutoff: receipt.recorded_at
            })
        })
        .is_err());
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn native_turn_cannot_upgrade_empty_or_error_preparation() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    for (id, outcome) in [
        ("prepared", PreparationOutcome::Prepared),
        ("empty", PreparationOutcome::Empty),
        ("error", PreparationOutcome::Error),
    ] {
        delivery(&store, &record, id, outcome);
    }
    let event=store.capture(CaptureRequest {id:Some("event".into()),repository:"Cortex".into(),kind:RecordKind::Event,source:Source {kind:"lifecycle_hook".into(),..Default::default()},metadata:serde_json::json!({"event":"PostToolUse","client":"codex","session_id":"session","turn_id":"turn"}),..Default::default()}).unwrap().record;
    for id in ["prepared", "empty", "error"] {
        let result = store.append_audit(AuditRequest {
            id: Some(format!("native-{id}")),
            repository: "Cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: Some(id.into()),
                event: Some(AuditReference::from_record(&event, false).unwrap()),
                attribution: attribution(),
                method: LinkageMethod::NativeTurn,
                evidence: Vec::new(),
                uncertainty: None,
            }),
        });
        if id == "prepared" {
            assert_eq!(
                result.unwrap().record.evidence_label,
                EvidenceLabel::ObservedRelatedAction
            );
        } else {
            assert!(result.is_err());
        }
    }
}
