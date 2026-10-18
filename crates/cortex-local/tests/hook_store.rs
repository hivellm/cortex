use cortex_local::*;
use fs2::FileExt;
use std::{
    fs,
    time::{Duration, Instant},
};

fn request(repository: &str, id: &str, authority: Authority) -> CaptureRequest {
    CaptureRequest {
        repository: repository.into(),
        id: Some(id.into()),
        title: id.into(),
        body: "hook searchable memory".into(),
        authority,
        ..Default::default()
    }
}

#[test]
fn hook_open_never_creates_missing_root_lock_or_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing");
    assert!(LocalStore::open_for_hook(&root, Duration::from_millis(100)).is_err());
    assert!(!root.exists());
    fs::create_dir(&root).unwrap();
    assert!(LocalStore::open_for_hook(&root, Duration::from_millis(100)).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::write(root.join("store.lock"), b"").unwrap();
    assert!(LocalStore::open_for_hook(&root, Duration::from_millis(100)).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn healthy_hook_captures_and_reads_with_exact_scope_and_bounded_recent() {
    let root = tempfile::tempdir().unwrap();
    let normal = LocalStore::open(root.path()).unwrap();
    normal
        .capture(request("Other", "foreign", Authority::Accepted))
        .unwrap();
    normal
        .capture(request("Cortex", "old", Authority::Deprecated))
        .unwrap();
    let hook = LocalStore::open_for_hook(root.path(), Duration::from_millis(100)).unwrap();
    hook.capture(request("Cortex", "current", Authority::Accepted))
        .unwrap();
    assert_eq!(hook.recent("Cortex", 5, false).unwrap().len(), 1);
    assert_eq!(hook.recent("Cortex", 5, true).unwrap().len(), 2);
    assert_eq!(hook.recent("Other", 5, false).unwrap()[0].id, "foreign");
    assert!(hook.recent("", 1, false).is_err());
    assert!(hook.recent("Cortex", 0, false).is_err());
    assert!(hook.recent("Cortex", 101, false).is_err());
    assert_eq!(
        hook.search(SearchQuery {
            repository: "Cortex".into(),
            text: "searchable".into(),
            limit: 5,
            include_inactive: false
        })
        .unwrap()
        .len(),
        1
    );
    assert!(hook.reindex().is_err());
}

#[test]
fn existing_hook_handle_never_recovers_later_pending_or_missing_index() {
    let root = tempfile::tempdir().unwrap();
    LocalStore::open(root.path()).unwrap();
    let hook = LocalStore::open_for_hook(root.path(), Duration::from_millis(100)).unwrap();
    fs::write(root.path().join("pending"), b"unfinished").unwrap();
    let index_before = fs::read(root.path().join("index.sqlite3")).unwrap();
    assert!(hook
        .capture(request("Cortex", "blocked", Authority::Accepted))
        .is_err());
    assert!(hook.get("Cortex", "blocked").is_err());
    assert!(LocalStore::open_for_hook(root.path(), Duration::from_millis(100)).is_err());
    assert_eq!(
        fs::read(root.path().join("index.sqlite3")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(root.path().join("pending")).unwrap(),
        b"unfinished"
    );
    fs::rename(
        root.path().join("pending"),
        root.path().join("saved-pending"),
    )
    .unwrap();
    fs::rename(
        root.path().join("index.sqlite3"),
        root.path().join("saved-index"),
    )
    .unwrap();
    assert!(hook.get("Cortex", "blocked").is_err());
    assert!(!root.path().join("index.sqlite3").exists());
    assert!(!root.path().join("pending").exists());
}

#[test]
fn corrupt_and_uninitialized_hook_indexes_fail_without_schema_repair() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("store.lock"), b"").unwrap();
    fs::write(root.path().join("index.sqlite3"), b"broken sqlite").unwrap();
    assert!(LocalStore::open_for_hook(root.path(), Duration::from_millis(100)).is_err());
    assert_eq!(
        fs::read(root.path().join("index.sqlite3")).unwrap(),
        b"broken sqlite"
    );
    fs::rename(
        root.path().join("index.sqlite3"),
        root.path().join("broken-backup"),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(connection);
    let before = fs::read(root.path().join("index.sqlite3")).unwrap();
    assert!(LocalStore::open_for_hook(root.path(), Duration::from_millis(100)).is_err());
    assert_eq!(fs::read(root.path().join("index.sqlite3")).unwrap(), before);
}

#[test]
fn hook_lock_timeout_is_capped_even_if_caller_requests_long_wait() {
    let root = tempfile::tempdir().unwrap();
    LocalStore::open(root.path()).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("store.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let start = Instant::now();
    assert!(LocalStore::open_for_hook(root.path(), Duration::from_secs(30)).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn recent_memories_filters_event_flood_before_limiting() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    store
        .capture(request("Cortex", "durable", Authority::Accepted))
        .unwrap();
    store
        .capture(request("Other", "foreign", Authority::Accepted))
        .unwrap();
    let events = (0..30)
        .map(|index| CaptureRequest {
            kind: RecordKind::Event,
            source: Source {
                captured_at: Some("2099-01-01T00:00:00Z".into()),
                ..Default::default()
            },
            ..request("Cortex", &format!("event-{index}"), Authority::Observation)
        })
        .collect();
    store.capture_batch(events).unwrap();
    let hook = LocalStore::open_for_hook(root.path(), Duration::from_millis(150)).unwrap();
    assert_eq!(
        hook.recent("Cortex", 1, false).unwrap()[0].kind,
        RecordKind::Event
    );
    assert_eq!(
        hook.recent_memories("Cortex", 1, false).unwrap()[0].id,
        "durable"
    );
    assert_eq!(
        hook.search_memories(SearchQuery {
            repository: "Cortex".into(),
            text: "searchable".into(),
            limit: 1,
            include_inactive: false
        })
        .unwrap()[0]
            .record
            .id,
        "durable"
    );
}

#[test]
fn recent_recall_excludes_stale_and_context_restrictions_before_limit() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    store
        .capture(request("Cortex", "eligible", Authority::Accepted))
        .unwrap();
    for (id, applicability) in [
        ("stale", serde_json::json!({"stale":true})),
        ("branch", serde_json::json!({"branch":"work"})),
        ("path", serde_json::json!({"path":"src/only.rs"})),
    ] {
        let mut input = request("Cortex", id, Authority::Accepted);
        input.metadata = serde_json::json!({"applicability":applicability});
        input.source.captured_at = Some("2099-01-01T00:00:00Z".into());
        store.capture(input).unwrap();
    }
    for memories_only in [false, true] {
        let recent = if memories_only {
            store.recent_memories("Cortex", 1, false)
        } else {
            store.recent("Cortex", 1, false)
        }
        .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, "eligible");
    }
    assert_eq!(store.recent_memories("Cortex", 10, true).unwrap().len(), 4);
}
