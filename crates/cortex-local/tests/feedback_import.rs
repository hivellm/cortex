use cortex_local::{
    feedback_import::{import_feedback, import_feedback_page},
    LocalStore,
};
use rusqlite::Connection;
use std::{collections::BTreeMap, fs, path::Path};

fn source(path: &Path) -> Connection {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE pre_thinking_feedback(query_id TEXT PRIMARY KEY,intent TEXT,helpful INTEGER NOT NULL,files_cited TEXT NOT NULL DEFAULT '[]',rating INTEGER,free_text TEXT,implicit_score REAL,recorded_at TEXT NOT NULL);
    INSERT INTO pre_thinking_feedback VALUES('q1',NULL,1,'[]',5,'old',0.5,'2026-01-01T00:00:00Z');").unwrap();
    db
}
fn snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn immutable_import_retries_changed_row_and_dry_run_preserve_source() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy #%.sqlite");
    let db = source(&path);
    drop(db);
    fs::write(input.path().join("legacy #%.sqlite-wal"), b"").unwrap();
    fs::write(input.path().join("legacy #%.sqlite-journal"), b"").unwrap();
    fs::write(
        input.path().join("legacy #%.sqlite-shm"),
        b"existing shared memory",
    )
    .unwrap();
    let store = LocalStore::open(output.path()).unwrap();
    let before = snapshot(input.path());
    let dry = import_feedback(&store, &path, true, 100).unwrap();
    assert!(dry.complete);
    assert_eq!(dry.validated, 1);
    assert_eq!(dry.imported, 0);
    let first = import_feedback(&store, &path, false, 100).unwrap();
    assert!(first.complete);
    assert_eq!(first.imported, 1);
    let retry = import_feedback(&store, &path, false, 100).unwrap();
    assert_eq!(retry.duplicates, 1);
    assert_eq!(snapshot(input.path()), before);
    let db = Connection::open(&path).unwrap();
    db.execute("UPDATE pre_thinking_feedback SET free_text='changed'", [])
        .unwrap();
    drop(db);
    let changed = import_feedback(&store, &path, false, 100).unwrap();
    assert_eq!(changed.imported, 1);
    let db = Connection::open(&path).unwrap();
    db.execute("CREATE TABLE unrelated(value TEXT)", [])
        .unwrap();
    drop(db);
    assert_eq!(
        import_feedback(&store, &path, false, 100)
            .unwrap()
            .duplicates,
        1
    );
}

#[test]
fn rejects_pending_sidecars_without_changing_any_source_bytes() {
    for suffix in ["-wal", "-journal"] {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let path = input.path().join("legacy.sqlite");
        drop(source(&path));
        fs::write(
            input.path().join(format!("legacy.sqlite{suffix}")),
            b"pending",
        )
        .unwrap();
        let before = snapshot(input.path());
        let store = LocalStore::open(output.path()).unwrap();
        assert!(import_feedback(&store, &path, false, 100).is_err());
        assert_eq!(snapshot(input.path()), before);
    }
}

#[test]
fn bounded_rows_report_remaining_and_oversized_rows_are_rejected() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES('q2',NULL,0,'[]',NULL,?1,NULL,'time')",
        ["x".repeat(70000)],
    )
    .unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let before = snapshot(input.path());
    let limited = import_feedback(&store, &path, false, 1).unwrap();
    assert!(!limited.complete);
    assert_eq!(limited.remaining, 1);
    assert_eq!(limited.total_rows, 2);
    let all = import_feedback(&store, &path, false, 100).unwrap();
    assert!(!all.complete);
    assert_eq!(all.rejected, 1);
    assert_eq!(all.duplicates, 1);
    assert_eq!(all.remaining, 0);
    assert_eq!(snapshot(input.path()), before);
    assert!(import_feedback(&store, &path, false, 0).is_err());
}

#[test]
fn storage_classes_and_invalid_utf8_are_preserved_without_interpretation() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute("UPDATE pre_thinking_feedback SET intent=x'00ff',free_text=CAST(x'fffe' AS TEXT),rating=-9223372036854775808",[]).unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let report = import_feedback(&store, &path, false, 100).unwrap();
    assert!(report.complete, "{report:?}");
    assert_eq!(report.imported, 1);
    fn collect(path: &Path, out: &mut Vec<serde_json::Value>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, out)
            } else if path.extension().is_some_and(|v| v == "json") {
                if let Ok(value) = serde_json::from_slice(&fs::read(path).unwrap()) {
                    out.push(value)
                }
            }
        }
    }
    let mut values = Vec::new();
    collect(store.root(), &mut values);
    let record = values
        .iter()
        .find(|v| v["legacy"]["raw_row"]["query_id"].is_object())
        .unwrap();
    assert_eq!(
        record["legacy"]["raw_row"]["intent"],
        serde_json::json!({"storage_class":"blob","value":"00ff"})
    );
    assert_eq!(
        record["legacy"]["raw_row"]["free_text"],
        serde_json::json!({"storage_class":"text","value":"fffe"})
    );
    assert_eq!(
        record["legacy"]["raw_row"]["rating"]["value"],
        "-9223372036854775808"
    );
    assert_eq!(record["request"]["evidence"], "legacy_unknown");
    assert_eq!(
        record["legacy"]["raw_row"]["implicit_score"],
        serde_json::json!({"storage_class":"real","value":"3fe0000000000000"})
    );
    assert!(record["request"]["reference"].is_null());
    let id = record["request"]["id"].as_str().unwrap();
    store.reindex().unwrap();
    let recovered = store
        .get_feedback("legacy-feedback-unattributed", id)
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.eligibility,
        cortex_local::FeedbackEligibility::LegacyUnknown
    );
    assert_eq!(serde_json::to_value(recovered.record).unwrap(), *record);
}

#[test]
fn snapshot_pagination_retries_and_stale_cursors_are_explicit() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES('q2',NULL,0,'[]',NULL,NULL,NULL,'time')",
        [],
    )
    .unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let first = import_feedback_page(&store, &path, false, 1, None).unwrap();
    assert_eq!(first.imported, 1);
    assert_eq!(first.remaining, 1);
    assert!(!first.complete);
    let cursor = first.next_cursor.unwrap();
    let second = import_feedback_page(&store, &path, false, 1, Some(&cursor)).unwrap();
    assert_eq!(second.imported, 1);
    assert_eq!(second.remaining, 0);
    assert!(second.complete);
    assert!(second.next_cursor.is_none());
    let repeated = import_feedback_page(&store, &path, false, 1, Some(&cursor)).unwrap();
    assert_eq!(repeated.imported, 0);
    assert_eq!(repeated.duplicates, 1);
    let entries = store
        .list_feedback(cortex_local::FeedbackListQuery {
            repository: "legacy-feedback-unattributed".into(),
            limit: 100,
            cursor: None,
        })
        .unwrap();
    assert_eq!(entries.entries.len(), 2);
    assert!(entries
        .entries
        .iter()
        .any(
            |entry| entry.record.legacy.as_ref().unwrap()["raw_row"]["implicit_score"]
                == serde_json::json!({"storage_class":"null","value":null})
        ));
    let db = Connection::open(&path).unwrap();
    db.execute(
        "UPDATE pre_thinking_feedback SET free_text='new snapshot' WHERE query_id='q2'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(import_feedback_page(&store, &path, false, 1, Some(&cursor)).is_err());
    let after = store
        .list_feedback(cortex_local::FeedbackListQuery {
            repository: "legacy-feedback-unattributed".into(),
            limit: 100,
            cursor: None,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(after.entries).unwrap(),
        serde_json::to_value(entries.entries).unwrap()
    );
    let malformed =
        serde_json::json!({"source":"x","snapshot_sha256":"x","offset":0,"extra":true}).to_string();
    assert!(import_feedback_page(&store, &path, false, 1, Some(&malformed)).is_err());
}

#[test]
fn cursors_bind_mode_and_final_page_verifies_destination_prefix() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute("INSERT INTO pre_thinking_feedback SELECT 'q2',intent,helpful,files_cited,rating,free_text,implicit_score,recorded_at FROM pre_thinking_feedback", []).unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let preview = import_feedback_page(&store, &path, true, 1, None).unwrap();
    let cursor = preview.next_cursor.unwrap();
    assert!(import_feedback_page(&store, &path, false, 1, Some(&cursor)).is_err());
    let mut forged: serde_json::Value = serde_json::from_str(&cursor).unwrap();
    forged["dry_run"] = false.into();
    let final_page =
        import_feedback_page(&store, &path, false, 1, Some(&forged.to_string())).unwrap();
    assert_eq!(final_page.prior_missing, 1);
    assert!(!final_page.prior_progress_verified);
    assert!(!final_page.complete);
    assert!(final_page
        .failures
        .iter()
        .any(|v| v.starts_with("row 1 key_sha256=")));
    let applied = import_feedback_page(&store, &path, false, 1, None).unwrap();
    assert!(import_feedback_page(&store, &path, true, 1, applied.next_cursor.as_deref()).is_err());
    assert!(
        import_feedback_page(&store, &path, false, 1, applied.next_cursor.as_deref())
            .unwrap()
            .complete
    );
    assert!(
        import_feedback_page(&store, &path, true, 1, Some(&cursor))
            .unwrap()
            .complete
    );
}

#[test]
fn rejected_prefix_cannot_be_hidden_by_clean_page_or_forged_count() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "UPDATE pre_thinking_feedback SET free_text=?1",
        ["x".repeat(70000)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES('q2',NULL,0,'[]',NULL,NULL,NULL,'time')",
        [],
    )
    .unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    for dry_run in [true, false] {
        let first = import_feedback_page(&store, &path, dry_run, 1, None).unwrap();
        assert_eq!(first.rejected, 1);
        let cursor = first.next_cursor.unwrap();
        let last = import_feedback_page(&store, &path, dry_run, 1, Some(&cursor)).unwrap();
        assert_eq!(last.rejected, 0);
        assert_eq!(last.cumulative_rejected, 1);
        assert_eq!(last.prior_rejected, 1);
        assert!(last.prior_progress_verified);
        assert!(!last.complete);
        let mut forged: serde_json::Value = serde_json::from_str(&cursor).unwrap();
        forged["cumulative_rejected"] = 0.into();
        let last =
            import_feedback_page(&store, &path, dry_run, 1, Some(&forged.to_string())).unwrap();
        assert_eq!(last.cumulative_rejected, 1);
        assert!(!last.prior_progress_verified);
        assert!(!last.complete);
    }
}

#[test]
fn failures_use_absolute_rows_and_hashed_keys_including_store_errors() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES('q2',NULL,0,'[]',NULL,?1,NULL,'time')",
        ["x".repeat(70000)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES('q3',NULL,0,'[]',NULL,NULL,NULL,'time')",
        [],
    )
    .unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let first = import_feedback_page(&store, &path, false, 1, None).unwrap();
    let second =
        import_feedback_page(&store, &path, false, 1, first.next_cursor.as_deref()).unwrap();
    assert!(second.failures[0].starts_with("row 2 key_sha256="));
    let index = Connection::open(output.path().join("index.sqlite3")).unwrap();
    index.execute_batch("CREATE TRIGGER reject_feedback BEFORE INSERT ON feedback_records BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    drop(index);
    let third =
        import_feedback_page(&store, &path, false, 1, second.next_cursor.as_deref()).unwrap();
    assert_eq!(third.rejected, 1);
    assert!(third
        .failures
        .iter()
        .any(|v| v.starts_with("row 3 key_sha256=") && v.contains("test failure")));
}

#[test]
fn unexpected_legacy_columns_are_rejected_without_source_changes() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "ALTER TABLE pre_thinking_feedback ADD COLUMN extra TEXT",
        [],
    )
    .unwrap();
    drop(db);
    let before = snapshot(input.path());
    let store = LocalStore::open(output.path()).unwrap();
    assert!(import_feedback(&store, &path, false, 100).is_err());
    assert_eq!(before, snapshot(input.path()));
}

#[test]
fn checkpointed_wal_snapshot_imports_without_source_sidecars_or_changes() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    let mode: String = db
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    db.execute(
        "UPDATE pre_thinking_feedback SET free_text='checkpointed'",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(db);
    let before = snapshot(input.path());
    assert_eq!(&before["legacy.sqlite"][18..20], &[2, 2]);
    assert!(!input.path().join("legacy.sqlite-wal").exists());
    assert!(!input.path().join("legacy.sqlite-shm").exists());
    let store = LocalStore::open(output.path()).unwrap();
    let report = import_feedback(&store, &path, false, 100).unwrap();
    assert!(report.complete, "{report:?}");
    assert_eq!(report.imported, 1);
    assert_eq!(snapshot(input.path()), before);
}

#[test]
fn null_primary_keys_have_stable_rowid_pagination() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES(NULL,NULL,0,'[]',NULL,NULL,NULL,'first')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO pre_thinking_feedback VALUES(NULL,NULL,0,'[]',NULL,?1,NULL,'second')",
        ["x".repeat(70000)],
    )
    .unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let first = import_feedback_page(&store, &path, true, 1, None).unwrap();
    assert!(first.failures[0].starts_with("row 1 key_sha256="));
    assert!(first.failures[0].contains("NULL query_id"));
    let second =
        import_feedback_page(&store, &path, true, 1, first.next_cursor.as_deref()).unwrap();
    assert!(second.failures[0].starts_with("row 2 key_sha256="));
    assert!(second.failures[0].contains("size bound"));
    let last = import_feedback_page(&store, &path, true, 1, second.next_cursor.as_deref()).unwrap();
    assert_eq!(last.prior_rejected, 2);
    assert!(last.prior_progress_verified);
    assert!(!last.complete);
}

#[test]
fn without_rowid_source_preserves_pagination() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute_batch("ALTER TABLE pre_thinking_feedback RENAME TO old;
        CREATE TABLE pre_thinking_feedback(query_id TEXT PRIMARY KEY,intent TEXT,helpful INTEGER,files_cited TEXT,rating INTEGER,free_text TEXT,implicit_score REAL,recorded_at TEXT) WITHOUT ROWID;
        INSERT INTO pre_thinking_feedback SELECT * FROM old;
        INSERT INTO pre_thinking_feedback SELECT 'q2',intent,helpful,files_cited,rating,free_text,implicit_score,recorded_at FROM old;").unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let first = import_feedback_page(&store, &path, false, 1, None).unwrap();
    let last = import_feedback_page(&store, &path, false, 1, first.next_cursor.as_deref()).unwrap();
    assert!(last.complete);
}

#[test]
fn generated_columns_are_not_silently_dropped() {
    for kind in ["VIRTUAL", "STORED"] {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let path = input.path().join("legacy.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!("CREATE TABLE pre_thinking_feedback(query_id TEXT PRIMARY KEY,intent TEXT,helpful INTEGER,files_cited TEXT,rating INTEGER,free_text TEXT,implicit_score REAL,recorded_at TEXT,extra TEXT GENERATED ALWAYS AS (query_id || '-extra') {kind});
            INSERT INTO pre_thinking_feedback(query_id) VALUES('q1');")).unwrap();
        drop(db);
        let before = snapshot(input.path());
        let store = LocalStore::open(output.path()).unwrap();
        assert!(import_feedback(&store, &path, false, 100).is_err());
        assert_eq!(snapshot(input.path()), before);
    }
}

#[test]
fn recovered_store_failure_is_not_reported_as_forged_cursor() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = input.path().join("legacy.sqlite");
    let db = source(&path);
    db.execute("INSERT INTO pre_thinking_feedback SELECT 'q2',intent,helpful,files_cited,rating,free_text,implicit_score,recorded_at FROM pre_thinking_feedback", []).unwrap();
    drop(db);
    let store = LocalStore::open(output.path()).unwrap();
    let index = Connection::open(output.path().join("index.sqlite3")).unwrap();
    index.execute_batch("CREATE TRIGGER reject_feedback BEFORE INSERT ON feedback_records BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    let first = import_feedback_page(&store, &path, false, 1, None).unwrap();
    assert_eq!(first.rejected, 1);
    index.execute_batch("DROP TRIGGER reject_feedback").unwrap();
    drop(index);
    store.reindex().unwrap();
    let last = import_feedback_page(&store, &path, false, 1, first.next_cursor.as_deref()).unwrap();
    assert_eq!(last.prior_rejected, 0);
    assert_eq!(last.cumulative_rejected, 1);
    assert!(last.prior_progress_verified);
    assert!(!last.complete);
    assert!(!last
        .failures
        .iter()
        .any(|v| v.contains("cursor rejection count differs")));
}
