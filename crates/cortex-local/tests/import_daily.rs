use cortex_local::{
    import::{import_daily, ImportOptions},
    *,
};
use cortex_storage::{
    daily_memory::{MemoryCapture, MemoryCategory, MemoryOrigin, MemoryScope},
    metadata::MetadataStore,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn memory(repo: &str, id: &str) -> MemoryCapture {
    let mut value = MemoryCapture::new(
        repo,
        "historic decision",
        "source body",
        MemoryCategory::Decision,
    );
    value.id = Some(id.into());
    value
}

fn imported_id(options: &ImportOptions, scope: &str, id: &str) -> String {
    let canonical = cortex_core::canonical_json::canonicalize(&serde_json::json!([
        options.namespace,
        scope,
        id
    ]))
    .unwrap();
    format!("daily-{:x}", Sha256::digest(canonical))
}

fn schema(path: &Path) -> Vec<(String, String)> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut query = connection
        .prepare("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")
        .unwrap();
    query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn readonly_wal_import_preserves_all_versions_and_resumes_without_duplicates() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    let mut old = memory("Cortex", "a-old");
    old.branch = Some("main".into());
    old.path = Some("src".into());
    old.pinned = true;
    let initial = source.capture_daily_memory(&old).unwrap();
    let stale = source
        .mark_daily_memory_stale(
            &old.scope,
            "a-old",
            true,
            initial.scope_revision,
            &MemoryOrigin::default(),
        )
        .unwrap();
    let mut successor = memory("Cortex", "z-new");
    successor.supersedes = Some("a-old".into());
    successor.expected_revision = Some(stale.scope_revision);
    source.capture_daily_memory(&successor).unwrap();
    let original = source.daily_memory(&old.scope, "a-old").unwrap().unwrap();
    let db_bytes = fs::read(&path).unwrap();
    let wal_path = source_home.path().join("daily.sqlite-wal");
    let wal_bytes = fs::read(&wal_path).unwrap();
    let schema_before = schema(&path);
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let options = ImportOptions {
        max_records: 1,
        ..Default::default()
    };
    for ordinal in 0..4 {
        let report = import_daily(&local, &path, &options).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.records_seen, 1);
        assert_eq!(report.imported, 1);
        assert_eq!(report.complete, ordinal == 3);
        assert_eq!(local.status(None).unwrap().revisions, ordinal + 1);
    }
    let old_id = imported_id(&options, "repo:Cortex", "a-old");
    let latest = local.get("Cortex", &old_id).unwrap().unwrap();
    assert_eq!(latest.revision, 3);
    assert_eq!(latest.authority, Authority::Superseded);
    assert_eq!(
        latest.metadata["daily_import"]["snapshot"],
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(
        latest.metadata["applicability"],
        serde_json::json!({"stale":true,"branch":"main","path":"src","pinned":true})
    );
    let first = local.get_revision("Cortex", &old_id, 1).unwrap().unwrap();
    assert_eq!(first.authority, Authority::Accepted);
    assert_eq!(first.metadata["applicability"]["stale"], false);
    assert_eq!(first.source, latest.source);
    for version in 1..=3 {
        let stored = local
            .get_revision("Cortex", &old_id, version)
            .unwrap()
            .unwrap();
        let raw = stored.metadata["daily_import"]["evidence_raw"]
            .as_str()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(raw).unwrap(),
            stored.metadata["daily_import"]["evidence"]
        );
        assert_eq!(
            stored.metadata["daily_import"]["evidence_sha256"],
            format!("{:x}", Sha256::digest(raw.as_bytes()))
        );
        assert_eq!(stored.metadata["daily_import"]["source_version"], version);
    }
    let new = local
        .get("Cortex", &imported_id(&options, "repo:Cortex", "z-new"))
        .unwrap()
        .unwrap();
    assert_eq!(new.links[0].target_id, old_id);
    assert_eq!(new.links[0].relation, Relation::Supersedes);
    let repeated = import_daily(&local, &path, &options).unwrap();
    assert!(repeated.complete);
    assert_eq!(repeated.imported, 0);
    assert_eq!(repeated.duplicates, 4);
    local.reindex().unwrap();
    assert_eq!(local.get("Cortex", &old_id).unwrap().unwrap(), latest);
    assert_eq!(fs::read(path).unwrap(), db_bytes);
    assert_eq!(fs::read(wal_path).unwrap(), wal_bytes);
    assert_eq!(
        schema(source_home.path().join("daily.sqlite").as_path()),
        schema_before
    );
}

#[test]
fn global_scope_requires_explicit_mapping_and_repository_case_is_preserved() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    source
        .capture_daily_memory(&memory("Cortex", "same"))
        .unwrap();
    source
        .capture_daily_memory(&memory("cortex", "same"))
        .unwrap();
    let mut global = memory("ignored", "same");
    global.scope = MemoryScope::Global;
    source.capture_daily_memory(&global).unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let rejected = import_daily(&local, &path, &ImportOptions::default()).unwrap();
    assert!(!rejected.complete);
    assert_eq!(local.status(None).unwrap().records, 0);
    let options = ImportOptions {
        global_repository: Some("explicit-global".into()),
        ..Default::default()
    };
    assert!(import_daily(&local, &path, &options).unwrap().complete);
    for repo in ["Cortex", "cortex", "explicit-global"] {
        assert_eq!(local.status(Some(repo)).unwrap().records, 1);
    }
    let changed = ImportOptions {
        global_repository: Some("other-global".into()),
        ..options
    };
    assert!(!import_daily(&local, &path, &changed).unwrap().complete);
    assert_eq!(local.status(Some("other-global")).unwrap().records, 0);
}

#[test]
fn explicit_repository_remapping_keeps_original_identities_distinct() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    let mut proposed = memory("Repo", "same");
    proposed.category = MemoryCategory::Lesson;
    proposed.status = cortex_storage::daily_memory::MemoryStatus::Proposed;
    source.capture_daily_memory(&proposed).unwrap();
    source
        .capture_daily_memory(&memory("repo", "same"))
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let mut options = ImportOptions::default();
    options
        .repository_map
        .insert("Repo".into(), "merged".into());
    options
        .repository_map
        .insert("repo".into(), "merged".into());
    assert!(import_daily(&local, &path, &options).unwrap().complete);
    assert_eq!(local.status(Some("merged")).unwrap().records, 2);
    let first = local
        .get("merged", &imported_id(&options, "repo:Repo", "same"))
        .unwrap()
        .unwrap();
    let second = local
        .get("merged", &imported_id(&options, "repo:repo", "same"))
        .unwrap()
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.kind, RecordKind::Lesson);
    assert_eq!(first.authority, Authority::Proposed);
    assert_eq!(
        first.metadata["daily_import"]["original_scope"],
        "repo:Repo"
    );
    assert_eq!(
        second.metadata["daily_import"]["original_scope"],
        "repo:repo"
    );
}

#[test]
fn dry_run_writes_no_progress_and_source_changes_do_not_overwrite_import() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    source.capture_daily_memory(&memory("repo", "one")).unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let dry = ImportOptions {
        dry_run: true,
        ..Default::default()
    };
    let report = import_daily(&local, &path, &dry).unwrap();
    assert!(report.complete);
    assert_eq!(report.records_seen, 1);
    assert_eq!(report.imported, 0);
    assert_eq!(local.status(None).unwrap().records, 0);
    assert!(!target.path().join("import-state").exists());
    let options = ImportOptions::default();
    assert!(import_daily(&local, &path, &options).unwrap().complete);
    let id = imported_id(&options, "repo:repo", "one");
    let first = local.get("repo", &id).unwrap().unwrap();
    source.capture_daily_memory(&memory("repo", "two")).unwrap();
    let changed = import_daily(&local, &path, &options).unwrap();
    assert!(!changed.complete);
    assert!(!changed.errors.is_empty());
    assert_eq!(local.get("repo", &id).unwrap().unwrap(), first);
    assert_eq!(local.status(None).unwrap().records, 1);
}

#[test]
fn inconsistent_or_missing_history_is_rejected_before_publication() {
    for defect in ["missing", "scope", "snapshot"] {
        let source_home = tempfile::tempdir().unwrap();
        let path = source_home.path().join("daily.sqlite");
        let mut source = MetadataStore::open(&path).unwrap();
        source.capture_daily_memory(&memory("repo", "one")).unwrap();
        match defect {
            "missing" => {
                source
                    .conn()
                    .execute("DELETE FROM daily_memory_evidence", [])
                    .unwrap();
            }
            "scope" => {
                source.conn().execute("UPDATE daily_memory_records SET data=json_set(data,'$.scope.repository','foreign')",[]).unwrap();
            }
            _ => {
                source
                    .conn()
                    .execute(
                        "UPDATE daily_memory_records SET data=json_set(data,'$.body','forged')",
                        [],
                    )
                    .unwrap();
            }
        }
        let target = tempfile::tempdir().unwrap();
        let local = LocalStore::open(target.path()).unwrap();
        let report = import_daily(&local, &path, &ImportOptions::default()).unwrap();
        assert!(!report.complete, "accepted {defect}");
        assert!(!report.errors.is_empty());
        assert_eq!(local.status(None).unwrap().records, 0);
        assert!(!target.path().join("import-state").exists());
    }
}

#[test]
fn unsupported_source_schema_is_not_migrated() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("unknown.sqlite");
    let source = rusqlite::Connection::open(&path).unwrap();
    source
        .execute("CREATE TABLE untouched(value TEXT)", [])
        .unwrap();
    drop(source);
    let bytes = fs::read(&path).unwrap();
    let schema_before = schema(&path);
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    assert!(
        !import_daily(&local, &path, &ImportOptions::default())
            .unwrap()
            .complete
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(schema(&path), schema_before);
}

#[test]
fn dry_run_rejects_oversized_lossless_metadata_before_any_publication() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    source.capture_daily_memory(&memory("repo", "one")).unwrap();
    source
        .conn()
        .execute(
            "UPDATE daily_memory_evidence SET data=json_set(data,'$.legacy_extra',?1)",
            ["x".repeat(2 * 1024 * 1024)],
        )
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    for dry_run in [true, false] {
        let report = import_daily(
            &local,
            &path,
            &ImportOptions {
                dry_run,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!report.complete);
        assert!(!report.errors.is_empty());
        assert!(
            report
                .errors
                .iter()
                .any(|failure| failure.reason.contains("4 MiB")),
            "{:?}",
            report.errors
        );
        assert_eq!(report.imported, 0);
        assert_eq!(local.status(None).unwrap().records, 0);
        assert!(!target.path().join("import-state").exists());
    }
}

#[test]
fn tiny_publication_budget_does_not_allow_unbounded_prepared_history() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    let input = memory("repo", "one");
    let mut revision = source.capture_daily_memory(&input).unwrap().scope_revision;
    for ordinal in 0..34 {
        revision = source
            .mark_daily_memory_stale(
                &input.scope,
                "one",
                ordinal % 2 == 0,
                revision,
                &MemoryOrigin::default(),
            )
            .unwrap()
            .scope_revision;
    }
    // A legacy unknown field is losslessly retained in every revision's snapshot.
    source
        .conn()
        .execute(
            "UPDATE daily_memory_records SET data=json_set(data,'$.legacy_extra',?1)",
            ["x".repeat(2 * 1024 * 1024)],
        )
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let report = import_daily(
        &local,
        &path,
        &ImportOptions {
            max_records: 1,
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!report.complete);
    assert!(
        report
            .errors
            .iter()
            .any(|failure| failure.reason.contains("prepared daily snapshot")),
        "{:?}",
        report.errors
    );
    assert_eq!(local.status(None).unwrap().records, 0);
    assert!(!target.path().join("import-state").exists());
}

fn directory_bytes(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, String> {
    let mut previous = None;
    for _ in 0..50 {
        match directory_bytes_once(root) {
            Ok(current) => {
                if previous.as_ref() == Some(&current) {
                    return current;
                }
                previous = Some(current);
            }
            // Windows can finish SQLite sidecar cleanup after connection drop.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => previous = None,
            Err(error) => panic!("unexpected fixture snapshot failure: {error}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("fixture files did not become stable for byte-identity comparison");
}

fn directory_bytes_once(
    root: &Path,
) -> std::io::Result<std::collections::BTreeMap<std::path::PathBuf, String>> {
    let mut output = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                output.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    format!("{:x}", Sha256::digest(fs::read(entry.path())?)),
                );
            }
        }
    }
    Ok(output)
}

#[test]
fn dry_run_checks_existing_identity_conflicts_without_writing_files() {
    let sources = tempfile::tempdir().unwrap();
    let first_path = sources.path().join("first.sqlite");
    let second_path = sources.path().join("second.sqlite");
    let mut first = MetadataStore::open(&first_path).unwrap();
    first.capture_daily_memory(&memory("repo", "same")).unwrap();
    let mut second = MetadataStore::open(&second_path).unwrap();
    let mut conflicting = memory("repo", "same");
    conflicting.body = "conflicting source evidence".into();
    second.capture_daily_memory(&conflicting).unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    assert!(
        import_daily(&local, &first_path, &ImportOptions::default())
            .unwrap()
            .complete
    );
    let before = directory_bytes(target.path());
    let report = import_daily(
        &local,
        &second_path,
        &ImportOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!report.complete);
    assert_eq!(report.rejected, 1);
    assert!(report
        .errors
        .iter()
        .any(|failure| failure.reason.contains("conflict")));
    assert_eq!(report.imported, 0);
    assert_eq!(directory_bytes(target.path()), before);
}

#[test]
fn dry_run_accepts_planned_contiguous_revisions_without_publishing_them() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    let input = memory("repo", "same");
    let receipt = source.capture_daily_memory(&input).unwrap();
    source
        .mark_daily_memory_stale(
            &input.scope,
            "same",
            true,
            receipt.scope_revision,
            &MemoryOrigin::default(),
        )
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let before = directory_bytes(target.path());
    let report = import_daily(
        &local,
        &path,
        &ImportOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.complete, "{:?}", report.errors);
    assert_eq!(report.records_seen, 2);
    assert_eq!(report.imported, 0);
    assert_eq!(directory_bytes(target.path()), before);
}

#[test]
fn dry_run_rejects_future_destination_schema_without_repairing_it() {
    let source_home = tempfile::tempdir().unwrap();
    let path = source_home.path().join("daily.sqlite");
    let mut source = MetadataStore::open(&path).unwrap();
    source.capture_daily_memory(&memory("repo", "one")).unwrap();
    let target = tempfile::tempdir().unwrap();
    let local = LocalStore::open(target.path()).unwrap();
    let connection = rusqlite::Connection::open(target.path().join("index.sqlite3")).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    let before = directory_bytes(target.path());
    assert!(import_daily(
        &local,
        &path,
        &ImportOptions {
            dry_run: true,
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(directory_bytes(target.path()), before);
}
