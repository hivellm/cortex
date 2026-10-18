use cortex_local::import::{import_archive, ImportOptions};
use cortex_local::{Authority, LocalStore, RecordKind, SearchQuery};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

fn envelope(id: &str, repository: Option<&str>, text: &str) -> Value {
    json!({
        "event_id":id,"schema_version":"1","occurred_at":"2026-01-01T12:00:00Z",
        "session_id":"original-session","stream":"live","tool":"claude-code","kind":"turn",
        "context":{"repo":repository,"platform":"win32"},
        "payload":{"user_message":text,"assistant_message":"original response"},
        "legacy_extra":{"retain":true}
    })
}

fn write_events(path: &Path, events: &[Value]) {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend(serde_json::to_vec(event).unwrap());
        bytes.push(b'\n');
    }
    fs::write(path, zstd::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
}

fn hash(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot hash {}: {error}", path.display()));
    format!("{:x}", Sha256::digest(bytes))
}

fn tree_bytes(root: &Path) -> BTreeMap<String, String> {
    fn visit(
        base: &Path,
        path: &Path,
        result: &mut BTreeMap<String, String>,
    ) -> std::io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(base, &path, result)?;
            } else {
                result.insert(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    format!("{:x}", Sha256::digest(fs::read(&path)?)),
                );
            }
        }
        Ok(())
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let mut result = BTreeMap::new();
        match visit(root, root, &mut result) {
            Ok(()) => return result,
            // Windows may enumerate a transient SQLite sidecar cleanup name.
            // Restart the entire snapshot; never omit an unreadable entry.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("cannot snapshot {}: {error}", root.display()),
        }
    }
}

fn search(store: &LocalStore, repository: &str, text: &str) -> Vec<cortex_local::SearchHit> {
    store
        .search(SearchQuery {
            repository: repository.into(),
            text: text.into(),
            limit: 100,
            include_inactive: false,
        })
        .unwrap()
}

fn options() -> ImportOptions {
    ImportOptions {
        namespace: "archive-tests".into(),
        ..Default::default()
    }
}

#[test]
fn multi_megabyte_envelope_resumes_same_namespace_without_truncating_source() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("large.parquet");
    let first = envelope("before-large", Some("Cortex"), "beforelargemarker");
    let large = envelope(
        "large",
        Some("Cortex"),
        &format!("largepayloadmarker {}", "x".repeat(2_800_000)),
    );
    let after = envelope("after-large", Some("Cortex"), "afterlargemarker");
    let mut writer = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
    writer
        .write_all(&serde_json::to_vec(&first).unwrap())
        .unwrap();
    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
    let store = LocalStore::open(destination.path()).unwrap();
    let initial = import_archive(&store, &path, &options()).unwrap();
    assert_eq!(
        (
            initial.imported,
            initial.incomplete_files,
            initial.open_frame_files,
            initial.partial_line_files,
            initial.partial_line_bytes
        ),
        (1, 1, 1, 0, 0)
    );
    assert_eq!(initial.decoder_failed_files, 0);
    let first_id = search(&store, "Cortex", "beforelargemarker")[0]
        .record
        .id
        .clone();
    writer
        .write_all(&serde_json::to_vec(&large).unwrap())
        .unwrap();
    writer.write_all(b"\n").unwrap();
    writer
        .write_all(&serde_json::to_vec(&after).unwrap())
        .unwrap();
    writer.write_all(b"\n").unwrap();
    writer.finish().unwrap();
    let before = hash(&path);
    let resumed = import_archive(
        &store,
        &path,
        &ImportOptions {
            max_records: 1,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(
        (resumed.imported, resumed.rejected, resumed.line_limit_files),
        (1, 0, 0)
    );
    let record = search(&store, "Cortex", "largepayloadmarker")
        .remove(0)
        .record;
    assert_eq!(record.source.payload, large);
    assert!(record.body.len() <= cortex_local::MAX_BODY_BYTES);
    assert_eq!(
        record.source.source_hash,
        Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&large).unwrap())
        ))
    );
    let completed = import_archive(&store, &path, &options()).unwrap();
    assert!(completed.complete, "{:?}", completed.errors);
    assert_eq!(completed.imported, 1);
    assert_eq!(
        search(&store, "Cortex", "beforelargemarker")[0].record.id,
        first_id
    );
    assert_eq!(search(&store, "Cortex", "afterlargemarker").len(), 1);
    assert_eq!(hash(&path), before);
    assert_eq!(
        import_archive(&store, &path, &options()).unwrap().imported,
        0
    );
}

#[test]
fn envelope_over_migration_ceiling_is_a_blocked_file_not_a_decoded_rejection() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("too-large.parquet");
    let large = envelope("too-large", Some("Cortex"), &"x".repeat(4 * 1024 * 1024));
    write_events(
        &path,
        &[
            envelope("prefix", Some("Cortex"), "prefixmarker"),
            large,
            envelope("suffix", Some("Cortex"), "suffixmarker"),
        ],
    );
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, &path, &options()).unwrap();
    assert!(!report.complete);
    assert_eq!(
        (
            report.records_seen,
            report.imported,
            report.rejected,
            report.decoder_failed_files,
            report.line_limit_files
        ),
        (1, 1, 0, 1, 1)
    );
    assert!(report.errors[0].reason.contains("4194304"));
    assert!(search(&store, "Cortex", "suffixmarker").is_empty());
    let retry = import_archive(&store, &path, &options()).unwrap();
    assert_eq!((retry.imported, retry.line_limit_files), (0, 1));
}

#[test]
fn historical_bootstrap_without_live_context_fields_preserves_evidence_and_requires_mapping() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("historical.parquet");
    let mut historical = envelope("historical", None, "bootstrapmarker");
    historical["stream"] = json!("Bootstrap");
    historical["context"] = json!({"cwd":"E:/some/project","branch":"main"});
    historical["parent_event_id"] = Value::Null;
    historical["model"] = json!("historical-model");
    let current = envelope("current", Some("Cortex"), "livemarker");
    write_events(&path, &[historical.clone(), current.clone()]);
    let source_before = hash(&path);
    let store = LocalStore::open(destination.path()).unwrap();
    let missing_mapping = import_archive(
        &store,
        &path,
        &ImportOptions {
            dry_run: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(missing_mapping.rejected, 1);
    assert!(missing_mapping.errors[0]
        .reason
        .contains("explicit unscoped repository mapping"));
    assert_eq!(store.status(None).unwrap().records, 0);
    let mapped = ImportOptions {
        unscoped_repository: Some("legacy:unscoped".into()),
        ..options()
    };
    let destination_before = tree_bytes(store.root());
    let dry = import_archive(
        &store,
        &path,
        &ImportOptions {
            dry_run: true,
            ..mapped.clone()
        },
    )
    .unwrap();
    assert!(dry.complete, "{:?}", dry.errors);
    assert_eq!((dry.records_seen, dry.rejected, dry.imported), (2, 0, 0));
    assert_eq!(tree_bytes(store.root()), destination_before);
    let imported = import_archive(&store, &path, &mapped).unwrap();
    assert!(imported.complete, "{:?}", imported.errors);
    assert_eq!((imported.imported, imported.rejected), (2, 0));
    let archived = search(&store, "legacy:unscoped", "bootstrapmarker")
        .remove(0)
        .record;
    assert_eq!(archived.source.payload, historical);
    assert!(archived.source.payload["context"].get("platform").is_none());
    assert!(archived.source.payload["context"].get("repo").is_none());
    assert!(archived.source.payload.get("content_hash").is_none());
    assert_eq!(
        archived.source.source_hash,
        Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&historical).unwrap())
        ))
    );
    assert_eq!(archived.metadata["import"]["original_scope"], "unscoped");
    assert_eq!(archived.authority, Authority::Observation);
    assert!(search(&store, "Cortex", "bootstrapmarker").is_empty());
    assert_eq!(
        search(&store, "Cortex", "livemarker")[0]
            .record
            .source
            .payload,
        current
    );
    assert_eq!(hash(&path), source_before);
    assert_eq!(import_archive(&store, &path, &mapped).unwrap().imported, 0);
}

#[test]
fn archive_compatibility_keeps_unknown_stream_and_malformed_identity_rejected() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let mut stream = envelope("bad-stream", Some("Cortex"), "bad");
    stream["stream"] = json!("FutureStream");
    let mut context = envelope("bad-context", Some("Cortex"), "bad");
    context["context"] = json!({"repo":42});
    let mut hash_type = envelope("bad-hash", Some("Cortex"), "bad");
    hash_type["content_hash"] = json!({"not":"a string"});
    let mut legacy_live = envelope("legacy-live", Some("Cortex"), "legacyLiveMarker");
    legacy_live["stream"] = json!("Live");
    legacy_live["context"]
        .as_object_mut()
        .unwrap()
        .remove("platform");
    let path = source.path().join("mixed.parquet");
    write_events(&path, &[stream, context, hash_type, legacy_live.clone()]);
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, &path, &options()).unwrap();
    assert_eq!(
        (report.records_seen, report.imported, report.rejected),
        (4, 1, 3)
    );
    assert_eq!(
        search(&store, "Cortex", "legacyLiveMarker")[0]
            .record
            .source
            .payload,
        legacy_live
    );
}

#[test]
fn both_quarantine_directories_count_archives_separately_from_diagnostics() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    for folder in ["quarantine", "_quarantine"] {
        let folder = source.path().join(folder);
        fs::create_dir(&folder).unwrap();
        write_events(
            &folder.join("excluded.parquet"),
            &[envelope("excluded", Some("Cortex"), "neverimport")],
        );
        fs::write(folder.join("excluded.parquet.reason"), b"diagnostic only").unwrap();
        fs::write(folder.join("incomplete.corrupt"), b"broken frame").unwrap();
    }
    write_events(
        &source.path().join("valid.parquet"),
        &[envelope("valid", Some("Cortex"), "validmarker")],
    );
    let before = tree_bytes(source.path());
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, source.path(), &options()).unwrap();
    assert!(report.complete, "{:?}", report.errors);
    assert_eq!(
        (
            report.quarantined_files,
            report.quarantined_archives,
            report.quarantine_diagnostics
        ),
        (6, 2, 4)
    );
    assert_eq!(
        (
            report.files_scanned,
            report.records_seen,
            report.imported,
            report.rejected
        ),
        (1, 1, 1, 0)
    );
    assert!(search(&store, "Cortex", "neverimport").is_empty());
    assert_eq!(tree_bytes(source.path()), before);
    for folder in ["quarantine", "_quarantine"] {
        let excluded = source.path().join(folder).join("excluded.parquet");
        let direct = import_archive(&store, &excluded, &options()).unwrap();
        assert_eq!(
            (
                direct.records_seen,
                direct.imported,
                direct.quarantined_archives
            ),
            (0, 0, 1)
        );
        let nested = source.path().join(folder).join("nested");
        fs::create_dir(&nested).unwrap();
        write_events(
            &nested.join("nested.parquet"),
            &[envelope("nested-excluded", Some("Cortex"), "neverimport")],
        );
        let direct = import_archive(&store, &nested, &options()).unwrap();
        assert_eq!(
            (
                direct.records_seen,
                direct.imported,
                direct.quarantined_archives
            ),
            (0, 0, 1)
        );
    }
}

#[test]
fn historical_recall_exceeds_snapshot_window_and_resumes_without_source_changes() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("months.parquet");
    let events: Vec<_> = (0..2205)
        .map(|id| {
            envelope(
                &format!("event-{id}"),
                Some("Cortex"),
                &format!("historicalmarker{id}"),
            )
        })
        .collect();
    write_events(&path, &events);
    let original_hash = hash(&path);
    let mut total = 0;
    let mut runs = 0;
    loop {
        let store = LocalStore::open(destination.path()).unwrap();
        let report = import_archive(
            &store,
            source.path(),
            &ImportOptions {
                max_records: 333,
                ..options()
            },
        )
        .unwrap();
        total += report.imported;
        runs += 1;
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.duplicates, 0);
        if report.complete {
            break;
        }
        assert!(report.limit_reached);
        assert!(runs < 10);
    }
    assert_eq!(total, 2205);
    assert!(runs > 1);
    let store = LocalStore::open(destination.path()).unwrap();
    assert_eq!(search(&store, "Cortex", "historicalmarker0").len(), 1);
    assert_eq!(search(&store, "Cortex", "historicalmarker2204").len(), 1);
    let first = search(&store, "Cortex", "historicalmarker0")
        .remove(0)
        .record;
    assert_eq!(first.kind, RecordKind::Event);
    assert_eq!(first.authority, Authority::Observation);
    assert_eq!(first.source.payload, events[0]);
    assert_eq!(first.source.source_id.as_deref(), Some("event-0"));
    assert_eq!(
        first.metadata["import"]["legacy_missing_content_hash"],
        true
    );
    assert_eq!(
        first.source.source_hash,
        Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&events[0]).unwrap())
        ))
    );
    store.reindex().unwrap();
    assert_eq!(search(&store, "Cortex", "historicalmarker0").len(), 1);
    assert_eq!(hash(&path), original_hash);
    let repeated = import_archive(&store, source.path(), &options()).unwrap();
    assert!(repeated.complete);
    assert_eq!(
        (
            repeated.unchanged_files,
            repeated.imported,
            repeated.records_seen
        ),
        (1, 0, 0)
    );
}

#[test]
fn tiny_source_files_share_segments_and_file_budget_advances_past_completed_sources() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    for id in 0..300 {
        write_events(
            &source.path().join(format!("{id:04}.parquet")),
            &[envelope(&id.to_string(), Some("Cortex"), "smallmarker")],
        );
    }
    let store = LocalStore::open(destination.path()).unwrap();
    let result = import_archive(&store, source.path(), &options()).unwrap();
    assert!(result.complete, "{:?}", result.errors);
    assert_eq!(result.imported, 300);
    assert_eq!(
        fs::read_dir(store.root().join("segments")).unwrap().count(),
        2
    );

    let next_source = TempDir::new().unwrap();
    write_events(
        &next_source.path().join("a.parquet"),
        &[envelope("first", Some("Cortex"), "first")],
    );
    write_events(
        &next_source.path().join("b.parquet"),
        &[envelope("second", Some("Cortex"), "second")],
    );
    let limited = ImportOptions {
        max_files: 1,
        ..options()
    };
    let first = import_archive(&store, next_source.path(), &limited).unwrap();
    assert_eq!(first.imported, 1);
    assert!(first.limit_reached && !first.complete);
    let second = import_archive(&store, next_source.path(), &limited).unwrap();
    assert_eq!((second.imported, second.unchanged_files), (1, 1));
    assert!(second.complete, "{:?}", second.errors);
}

#[test]
fn duplicate_occurrences_and_conflicts_are_explicit_durable_and_preserve_first_source() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let original = envelope("shared", Some("Cortex"), "originalmarker");
    write_events(
        &source.path().join("a.parquet"),
        std::slice::from_ref(&original),
    );
    write_events(
        &source.path().join("b.jsonl.zst"),
        std::slice::from_ref(&original),
    );
    write_events(
        &source.path().join("c.ndjson.zst"),
        &[envelope("shared", Some("Cortex"), "conflictingmarker")],
    );
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, source.path(), &options()).unwrap();
    assert_eq!(
        (report.imported, report.duplicates, report.rejected),
        (1, 1, 1)
    );
    assert!(!report.complete);
    let record = search(&store, "Cortex", "originalmarker").remove(0).record;
    assert_eq!(record.source.payload, original);
    assert!(record.source.locator.ends_with("a.parquet#line=1"));
    assert!(search(&store, "Cortex", "conflictingmarker").is_empty());
    let ledger = rusqlite::Connection::open(store.root().join("archive-import.sqlite3")).unwrap();
    let occurrences: i64 = ledger
        .query_row("SELECT count(*) FROM occurrences", [], |row| row.get(0))
        .unwrap();
    let rejected: Vec<u8> = ledger
        .query_row(
            "SELECT raw_rejected FROM occurrences WHERE rejection IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(occurrences, 3);
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected).unwrap()["payload"]["user_message"],
        "conflictingmarker"
    );
    drop(ledger);
    let again = import_archive(&store, source.path(), &options()).unwrap();
    assert_eq!((again.imported, again.rejected), (0, 1));
    assert!(!again.complete);
}

#[test]
fn same_size_replacement_gets_new_generation_and_never_reuses_old_checkpoint() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("replace.parquet");
    write_events(&path, &[envelope("same", Some("Cortex"), "AAAAAA")]);
    let store = LocalStore::open(destination.path()).unwrap();
    assert_eq!(
        import_archive(&store, &path, &options()).unwrap().imported,
        1
    );
    let size = fs::metadata(&path).unwrap().len();
    let before = hash(&path);
    write_events(&path, &[envelope("same", Some("Cortex"), "BBBBBB")]);
    assert_eq!(fs::metadata(&path).unwrap().len(), size);
    assert_ne!(hash(&path), before);
    let changed = import_archive(&store, &path, &options()).unwrap();
    assert!(!changed.complete);
    assert_eq!(changed.rejected, 1);
    assert!(changed
        .errors
        .iter()
        .any(|error| error.reason.contains("generation 2")));
    assert_eq!(search(&store, "Cortex", "AAAAAA").len(), 1);
    assert!(search(&store, "Cortex", "BBBBBB").is_empty());
    let ledger = rusqlite::Connection::open(store.root().join("archive-import.sqlite3")).unwrap();
    let generations: i64 = ledger
        .query_row("SELECT count(*) FROM source_generations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(generations, 2);
}

#[test]
fn incomplete_live_frame_resumes_after_verified_append() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let live = source.path().join("writer.bin");
    let path = source.path().join("snapshot.parquet");
    let first = serde_json::to_vec(&envelope("first", Some("Cortex"), "firstmarker")).unwrap();
    let second = serde_json::to_vec(&envelope("second", Some("Cortex"), "secondmarker")).unwrap();
    let mut writer = zstd::Encoder::new(File::create(&live).unwrap(), 1).unwrap();
    writer.write_all(&first).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.write_all(&second[..second.len() / 2]).unwrap();
    writer.flush().unwrap();
    fs::write(&path, fs::read(&live).unwrap()).unwrap();
    let store = LocalStore::open(destination.path()).unwrap();
    let partial = import_archive(&store, &path, &options()).unwrap();
    assert_eq!((partial.imported, partial.incomplete_files), (1, 1));
    assert!(!partial.complete);
    writer.write_all(&second[second.len() / 2..]).unwrap();
    writer.write_all(b"\n").unwrap();
    writer.finish().unwrap();
    fs::write(&path, fs::read(&live).unwrap()).unwrap();
    let resumed = import_archive(&store, &path, &options()).unwrap();
    assert!(resumed.complete, "{:?}", resumed.errors);
    assert_eq!(
        (resumed.imported, resumed.duplicates, resumed.records_seen),
        (1, 0, 1)
    );
    assert_eq!(search(&store, "Cortex", "firstmarker").len(), 1);
    assert_eq!(search(&store, "Cortex", "secondmarker").len(), 1);
}

#[test]
fn corrupt_invalid_and_quarantined_sources_do_not_hide_other_successes() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    fs::write(source.path().join("a.parquet"), b"not a zstd archive").unwrap();
    fs::write(
        source.path().join("b.parquet"),
        zstd::encode_all(b"\xff\xfe\n".as_slice(), 1).unwrap(),
    )
    .unwrap();
    write_events(
        &source.path().join("c.parquet"),
        &[envelope("good", Some("Cortex"), "survivingmarker")],
    );
    fs::create_dir(source.path().join("_quarantine")).unwrap();
    fs::write(
        source.path().join("_quarantine").join("bad.parquet"),
        b"untouched",
    )
    .unwrap();
    fs::write(
        source.path().join("c.parquet.reason"),
        b"historical sidecar",
    )
    .unwrap();
    let before: Vec<_> = ["a.parquet", "b.parquet", "c.parquet"]
        .iter()
        .map(|name| hash(&source.path().join(name)))
        .collect();
    let store = LocalStore::open(destination.path()).unwrap();
    let result = import_archive(&store, source.path(), &options()).unwrap();
    assert_eq!(
        (
            result.files_scanned,
            result.imported,
            result.rejected,
            result.quarantined_files
        ),
        (3, 1, 1, 2)
    );
    assert!(!result.complete);
    assert!(result.errors.len() >= 2);
    assert_eq!(search(&store, "Cortex", "survivingmarker").len(), 1);
    let after: Vec<_> = ["a.parquet", "b.parquet", "c.parquet"]
        .iter()
        .map(|name| hash(&source.path().join(name)))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn dry_run_detects_existing_and_in_batch_conflicts_without_any_store_mutation() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("existing.parquet");
    write_events(&path, &[envelope("existing", Some("Cortex"), "old")]);
    let store = LocalStore::open(destination.path()).unwrap();
    import_archive(&store, &path, &options()).unwrap();
    write_events(
        &path,
        &[
            envelope("existing", Some("Cortex"), "changed"),
            envelope("new", Some("Cortex"), "one"),
            envelope("new", Some("Cortex"), "two"),
        ],
    );
    let index_before = hash(&store.root().join("index.sqlite3"));
    let ledger_before = hash(&store.root().join("archive-import.sqlite3"));
    let tree_before = tree_bytes(store.root());
    let report = import_archive(
        &store,
        &path,
        &ImportOptions {
            dry_run: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!((report.imported, report.rejected), (0, 2));
    assert!(!report.complete);
    assert_eq!(hash(&store.root().join("index.sqlite3")), index_before);
    assert_eq!(
        hash(&store.root().join("archive-import.sqlite3")),
        ledger_before
    );
    assert_eq!(tree_bytes(store.root()), tree_before);
    let mut changed = ImportOptions {
        dry_run: true,
        ..options()
    };
    changed
        .repository_map
        .insert("Cortex".into(), "renamed".into());
    assert!(import_archive(&store, &path, &changed).is_err());
    assert_eq!(store.status(None).unwrap().records, 1);
}

#[test]
fn dry_run_new_store_only_reads_sources_and_preflights_envelope_errors() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("events.parquet");
    let mut invalid = envelope("invalid", Some("Cortex"), "invalid");
    invalid["occurred_at"] = json!("not-a-timestamp");
    write_events(
        &path,
        &[envelope("valid", Some("Cortex"), "valid"), invalid],
    );
    let store = LocalStore::open(destination.path()).unwrap();
    let before = hash(&store.root().join("index.sqlite3"));
    let tree_before = tree_bytes(store.root());
    let report = import_archive(
        &store,
        &path,
        &ImportOptions {
            dry_run: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(
        (report.records_seen, report.imported, report.rejected),
        (2, 0, 1)
    );
    assert!(!report.complete);
    assert!(!store.root().join("archive-import.sqlite3").exists());
    assert!(!store.root().join("archive-import.lock").exists());
    assert!(!store.root().join("segments").exists());
    assert_eq!(hash(&store.root().join("index.sqlite3")), before);
    assert_eq!(tree_bytes(store.root()), tree_before);
}

#[test]
fn mappings_keep_original_identity_domains_and_require_explicit_unscoped_mapping() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("scope.parquet");
    write_events(
        &path,
        &[
            envelope("shared", Some("A"), "scopemarker"),
            envelope("shared", Some("a"), "scopemarker"),
            envelope("shared", None, "scopemarker"),
        ],
    );
    let store = LocalStore::open(destination.path()).unwrap();
    let mut configuration = options();
    configuration
        .repository_map
        .insert("A".into(), "Cortex".into());
    configuration
        .repository_map
        .insert("a".into(), "Cortex".into());
    let first = import_archive(&store, &path, &configuration).unwrap();
    assert_eq!((first.imported, first.rejected), (2, 1));
    assert_eq!(search(&store, "Cortex", "scopemarker").len(), 2);
    configuration.namespace = "explicit-unscoped".into();
    configuration.unscoped_repository = Some("Unscoped".into());
    let mapped = import_archive(&store, &path, &configuration).unwrap();
    assert_eq!(mapped.imported, 3);
    assert!(mapped.complete);
    assert_eq!(search(&store, "Unscoped", "scopemarker").len(), 1);
    assert!(search(&store, "cortex", "scopemarker").is_empty());
}

#[test]
fn discovery_depth_limit_is_reported_without_losing_other_records() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    write_events(
        &source.path().join("a.parquet"),
        &[envelope("safe", Some("Cortex"), "safe")],
    );
    let mut deep = source.path().join("z");
    for _ in 0..66 {
        fs::create_dir_all(&deep).unwrap();
        deep = deep.join("x");
    }
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, source.path(), &options()).unwrap();
    assert_eq!(report.imported, 1);
    assert!(!report.complete);
    assert!(report
        .errors
        .iter()
        .any(|error| error.reason.contains("depth64")));
}

#[test]
fn canonical_commit_before_checkpoint_retry_deduplicates_and_keeps_source_hash() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("append.parquet");
    write_events(&path, &[envelope("first", Some("Cortex"), "first")]);
    let store = LocalStore::open(destination.path()).unwrap();
    import_archive(&store, &path, &options()).unwrap();
    let ledger_path = store.root().join("archive-import.sqlite3");
    let durable_checkpoint = fs::read(&ledger_path).unwrap();
    let mut second = serde_json::to_vec(&envelope("second", Some("Cortex"), "second")).unwrap();
    second.push(b'\n');
    let frame = zstd::encode_all(second.as_slice(), 1).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&frame)
        .unwrap();
    let source_hash = hash(&path);
    assert_eq!(
        import_archive(&store, &path, &options()).unwrap().imported,
        1
    );
    // Restore only the prior derived checkpoint to model abrupt termination
    // after canonical publication but before the checkpoint transaction.
    fs::write(&ledger_path, durable_checkpoint).unwrap();
    let recovered = import_archive(&store, &path, &options()).unwrap();
    assert_eq!(
        (
            recovered.imported,
            recovered.duplicates,
            recovered.records_seen
        ),
        (0, 1, 1)
    );
    assert!(recovered.complete);
    assert_eq!(store.status(None).unwrap().records, 2);
    assert_eq!(hash(&path), source_hash);
}

#[test]
fn dry_run_detects_conflicts_across_batch_boundaries() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("dry.parquet");
    let mut events: Vec<_> = (0..256)
        .map(|id| envelope(&format!("id-{id}"), Some("Cortex"), "original"))
        .collect();
    events.push(envelope("id-0", Some("Cortex"), "changed"));
    write_events(&path, &events);
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(
        &store,
        &path,
        &ImportOptions {
            dry_run: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(
        (report.records_seen, report.rejected, report.imported),
        (257, 1, 0)
    );
    assert!(!report.complete);
    assert!(!store.root().join("segments").exists());
}

#[test]
fn archived_pre_redaction_content_hash_is_preserved_without_comparing_to_redacted_payload() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("redacted.parquet");
    let mut original = envelope("redacted", Some("Cortex"), "[REDACTED] retainedmarker");
    original["content_hash"] = json!(format!("sha256:{}", "0".repeat(64)));
    original["redactions"] = json!(["credential"]);
    write_events(&path, std::slice::from_ref(&original));
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, &path, &options()).unwrap();
    assert_eq!(report.imported, 1);
    assert!(report.complete, "{:?}", report.errors);
    let record = search(&store, "Cortex", "retainedmarker").remove(0).record;
    assert_eq!(record.source.payload, original);
    assert_eq!(
        record.metadata["import"]["legacy_missing_content_hash"],
        false
    );
}

#[test]
fn explicit_parent_links_resolve_only_inside_exact_repository() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let path = source.path().join("parents.parquet");
    let parent = envelope("parent", Some("Cortex"), "parentmarker");
    let mut child = envelope("child", Some("Cortex"), "childmarker");
    child["parent_event_id"] = json!("parent");
    let mut dangling = envelope("dangling", Some("Cortex"), "danglingmarker");
    dangling["parent_event_id"] = json!("foreign-parent");
    let foreign = envelope("foreign-parent", Some("Other"), "foreignmarker");
    write_events(&path, &[parent, child, dangling, foreign]);
    let store = LocalStore::open(destination.path()).unwrap();
    let report = import_archive(&store, &path, &options()).unwrap();
    assert_eq!(report.imported, 4);
    let child = search(&store, "Cortex", "childmarker").remove(0).record;
    let graph = store
        .graph(cortex_local::GraphQuery {
            repository: "Cortex".into(),
            id: child.id,
            depth: 2,
            limit: 10,
        })
        .unwrap();
    assert_eq!((graph.records.len(), graph.edges.len()), (2, 1));
    assert_eq!(graph.edges[0].relation, cortex_local::Relation::DerivedFrom);
    let dangling = search(&store, "Cortex", "danglingmarker").remove(0).record;
    assert_eq!(dangling.links.len(), 1);
    let graph = store
        .graph(cortex_local::GraphQuery {
            repository: "Cortex".into(),
            id: dangling.id,
            depth: 2,
            limit: 10,
        })
        .unwrap();
    assert_eq!((graph.records.len(), graph.edges.len()), (1, 0));
    assert_eq!(store.reindex().unwrap().dangling_links, 1);
}
