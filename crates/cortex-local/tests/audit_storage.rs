use cortex_local::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn memory(store: &LocalStore, repository: &str) -> Record {
    store
        .capture(CaptureRequest {
            id: Some("shared-memory".into()),
            repository: repository.into(),
            kind: RecordKind::Lesson,
            title: "Retain exact evidence".into(),
            body: "Private fixture body is not copied into audit".into(),
            source: Source {
                kind: "fixture".into(),
                locator: "https://example.invalid/?token=fake-audit-fixture".into(),
                payload: serde_json::json!({"private_fixture":"not-an-audit-summary"}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}

fn delivery(record: &Record, id: &str) -> AuditRequest {
    AuditRequest {
        id: Some(id.into()),
        repository: record.repository.clone(),
        payload: AuditPayload::Delivery(DeliveryDraft {
            query_id: format!("query-{id}"),
            origin: AuditOrigin::Mcp,
            attribution: AuditAttribution::default(),
            query_sha256: hash("fixture query"),
            context_sha256: hash("fixture rendered context"),
            context_bytes: 24,
            selected_count: 1,
            omitted_count: 0,
            references: vec![AuditReference::from_record(record, false).unwrap()],
            outcome: PreparationOutcome::Prepared,
            latency_us: 750,
            error: None,
        }),
    }
}

fn observation(repository: &str, id: &str) -> AuditRequest {
    AuditRequest {
        id: Some(id.into()),
        repository: repository.into(),
        payload: AuditPayload::Observation(ObservationDraft {
            delivery_id: None,
            event: None,
            attribution: AuditAttribution::default(),
            method: LinkageMethod::ReportedUse,
            evidence: Vec::new(),
            uncertainty: Some(UncertaintyCode::ReportedOnly),
        }),
    }
}

fn snapshot(root: &Path, directory: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !path.exists() {
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &root.join(directory), &mut out);
    out
}

#[test]
fn restart_and_retries_keep_exact_audit_and_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex");
    let sources = snapshot(root.path(), "records");
    let request = delivery(&record, "delivery-one");
    let first = store.append_audit(request.clone()).unwrap();
    assert!(first.created);
    assert_eq!(first.record.evidence_label, EvidenceLabel::PreparedDelivery);
    let canonical = snapshot(root.path(), "audit");
    assert_eq!(canonical.len(), 1);
    let bytes = canonical.values().next().unwrap();
    let text = std::str::from_utf8(bytes).unwrap();
    for excluded in [
        "fake-audit-fixture",
        "not-an-audit-summary",
        "Private fixture body",
        "fixture query",
    ] {
        assert!(!text.contains(excluded));
    }
    drop(store);
    let reopened = LocalStore::open(root.path()).unwrap();
    let retry = reopened.append_audit(request.clone()).unwrap();
    assert!(!retry.created);
    assert_eq!(retry.record, first.record);
    assert_eq!(snapshot(root.path(), "audit"), canonical);
    assert_eq!(snapshot(root.path(), "records"), sources);
    assert_eq!(reopened.status(Some("Cortex")).unwrap().records, 1);
    assert_eq!(
        reopened.get_audit("Cortex", "delivery-one").unwrap(),
        Some(first.record)
    );
    assert!(reopened
        .get_audit("cortex", "delivery-one")
        .unwrap()
        .is_none());

    let mut changed = request;
    let AuditPayload::Delivery(draft) = &mut changed.payload else {
        unreachable!()
    };
    draft.latency_us += 1;
    assert!(matches!(
        reopened.append_audit(changed),
        Err(LocalError::Conflict(_))
    ));
    assert_eq!(snapshot(root.path(), "audit"), canonical);
}

#[test]
fn same_ids_in_distinct_scopes_do_not_share_audit_or_references() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let upper = memory(&store, "Cortex");
    let lower = memory(&store, "cortex");
    for record in [&upper, &lower] {
        let result = store.append_audit(delivery(record, "same-id")).unwrap();
        assert!(result.created);
        assert_eq!(result.record.repository, record.repository);
    }
    let before = snapshot(root.path(), "audit");
    let mut foreign = delivery(&upper, "foreign-reference");
    foreign.repository = "cortex".into();
    assert!(store.append_audit(foreign).is_err());
    assert_eq!(snapshot(root.path(), "audit"), before);
    assert_eq!(before.len(), 2);
}

#[test]
fn failed_projection_keeps_canonical_record_for_pending_recovery() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_audit_projection BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT, 'fixture projection failure'); END;").unwrap();
    assert!(store
        .append_audit(observation("Cortex", "published-before-index"))
        .is_err());
    assert!(root.path().join("pending").exists());
    let canonical = snapshot(root.path(), "audit");
    assert_eq!(canonical.len(), 1);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM audit_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TRIGGER fail_audit_projection;")
        .unwrap();
    drop(connection);
    drop(store);
    let reopened = LocalStore::open(root.path()).unwrap();
    let recovered = reopened
        .get_audit("Cortex", "published-before-index")
        .unwrap()
        .unwrap();
    assert_eq!(recovered.evidence_label, EvidenceLabel::Unknown);
    assert!(!root.path().join("pending").exists());
    assert_eq!(snapshot(root.path(), "audit"), canonical);
    assert!(
        !reopened
            .append_audit(observation("Cortex", "published-before-index"))
            .unwrap()
            .created
    );
}

#[test]
fn old_projection_migrates_outside_hooks_without_changing_memory_sources() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    memory(&store, "Cortex");
    let sources = snapshot(root.path(), "records");
    drop(store);
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection
        .execute_batch("DROP TABLE audit_records; PRAGMA user_version=1;")
        .unwrap();
    drop(connection);
    assert!(LocalStore::open_for_hook(root.path(), Duration::ZERO).is_err());
    let store = LocalStore::open(root.path()).unwrap();
    assert!(LocalStore::open_for_hook(root.path(), Duration::ZERO).is_ok());
    assert_eq!(snapshot(root.path(), "records"), sources);
    assert_eq!(store.status(Some("Cortex")).unwrap().records, 1);
    assert!(
        store
            .append_audit(observation("Cortex", "after-upgrade"))
            .unwrap()
            .created
    );
}

const WRITERS: usize = 3;
const EACH: usize = 6;

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(Instant::now() < deadline, "audit writer barrier timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn audit_subprocess_writer() {
    let Some(root) = std::env::var_os("CORTEX_AUDIT_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let writer = std::env::var("CORTEX_AUDIT_TEST_WRITER").unwrap();
    fs::write(root.join(format!("ready-{writer}")), b"ready").unwrap();
    wait_for(&root.join("start"));
    let store = LocalStore::open(root.join("store")).unwrap();
    let mut shared_created = 0;
    for item in 0..EACH {
        assert!(
            store
                .append_audit(observation("Cortex", &format!("writer-{writer}-{item}")))
                .unwrap()
                .created
        );
        shared_created += usize::from(
            store
                .append_audit(observation("Cortex", &format!("shared-{item}")))
                .unwrap()
                .created,
        );
    }
    fs::write(
        root.join(format!("result-{writer}")),
        shared_created.to_string(),
    )
    .unwrap();
}

struct Children(Vec<Child>);

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn independent_processes_preserve_unique_and_idempotent_observations() {
    let root = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children = Children(Vec::new());
    for writer in 0..WRITERS {
        children.0.push(
            Command::new(&executable)
                .args(["--exact", "audit_subprocess_writer", "--nocapture"])
                .env("CORTEX_AUDIT_TEST_ROOT", root.path())
                .env("CORTEX_AUDIT_TEST_WRITER", writer.to_string())
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for writer in 0..WRITERS {
        wait_for(&root.path().join(format!("ready-{writer}")));
    }
    fs::write(root.path().join("start"), b"start").unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    for child in &mut children.0 {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "audit writer stalled");
            thread::sleep(Duration::from_millis(10));
        }
    }
    let created: usize = (0..WRITERS)
        .map(|writer| {
            fs::read_to_string(root.path().join(format!("result-{writer}")))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .sum();
    assert_eq!(created, EACH);
    let store = LocalStore::open(root.path().join("store")).unwrap();
    for writer in 0..WRITERS {
        for item in 0..EACH {
            let record = store
                .get_audit("Cortex", &format!("writer-{writer}-{item}"))
                .unwrap()
                .unwrap();
            assert_eq!(record.evidence_label, EvidenceLabel::Unknown);
        }
    }
    for item in 0..EACH {
        assert!(store
            .get_audit("Cortex", &format!("shared-{item}"))
            .unwrap()
            .is_some());
    }
    assert_eq!(snapshot(store.root(), "audit").len(), EACH * (WRITERS + 1));
    assert_eq!(store.status(Some("Cortex")).unwrap().records, 0);
}
