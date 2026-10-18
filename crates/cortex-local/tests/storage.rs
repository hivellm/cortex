use cortex_local::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn memory(repo: &str, id: &str, body: &str) -> CaptureRequest {
    CaptureRequest {
        id: Some(id.into()),
        repository: repo.into(),
        kind: RecordKind::Lesson,
        authority: Authority::Accepted,
        title: "Durable lesson".into(),
        body: body.into(),
        source: Source {
            kind: "test".into(),
            locator: "original/source.md".into(),
            source_id: Some(format!("source-{id}")),
            source_hash: Some("original-hash".into()),
            captured_at: Some("2026-01-01T00:00:00Z".into()),
            payload: json!({"original":body,"unknown_field":{"preserved":true}}),
        },
        ..Default::default()
    }
}

fn event(id: &str) -> CaptureRequest {
    CaptureRequest {
        kind: RecordKind::Event,
        authority: Authority::Observation,
        ..memory("Cortex", id, "historical event searchablemarker")
    }
}

fn query(repository: &str, text: &str) -> SearchQuery {
    SearchQuery {
        repository: repository.into(),
        text: text.into(),
        limit: 100,
        include_inactive: false,
    }
}

#[test]
fn scoped_batch_lookup_preserves_order_and_preflight_enforces_canonical_limits() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let first = store
        .capture(memory("Cortex", "shared", "first"))
        .unwrap()
        .record;
    store
        .capture(memory("cortex", "shared", "other scope"))
        .unwrap();
    assert_eq!(
        store
            .get_many(
                "Cortex",
                &["missing".into(), "shared".into(), "shared".into()]
            )
            .unwrap(),
        vec![None, Some(first.clone()), Some(first)]
    );
    assert!(store
        .get_many("Cortex", &vec!["shared".into(); 257])
        .is_err());
    let mut invalid = memory("Cortex", "invalid", "body");
    invalid.title = "x".repeat(16 * 1024 + 1);
    assert!(LocalStore::validate_capture(&invalid).is_err());
    assert!(store.capture(invalid).is_err());
    assert_eq!(store.status(None).unwrap().records, 2);
}

#[test]
fn read_queries_do_not_rewrite_initialized_database() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    store
        .capture(memory("Cortex", "read", "readmarker"))
        .unwrap();
    let path = store.root().join("index.sqlite3");
    let before = std::fs::read(&path).unwrap();
    assert!(store.get("Cortex", "read").unwrap().is_some());
    assert_eq!(
        store.search(query("Cortex", "readmarker")).unwrap().len(),
        1
    );
    assert_eq!(store.status(None).unwrap().records, 1);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn integral_float_payload_retry_is_identical_after_reindex() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let mut request = memory("Cortex", "float", "floatmarker");
    request.source.payload = json!({"whole_float":1.0,"fraction":0.25,"integer":1});
    request.metadata = json!({"confidence":1.0});
    store.capture(request.clone()).unwrap();
    store.reindex().unwrap();
    assert!(!store.capture(request).unwrap().created);
}

#[test]
fn ordinary_search_excludes_stale_and_context_restricted_imports() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for (id, applicability) in [
        ("eligible", json!({"stale":false,"branch":null,"path":null})),
        ("stale", json!({"stale":true})),
        ("branch", json!({"branch":"work"})),
        ("path", json!({"path":"src/specific.rs"})),
    ] {
        let mut request = memory("Cortex", id, "applicabilitymarker");
        request.metadata = json!({"applicability":applicability});
        if id != "eligible" {
            request.links.push(TypedLink {
                target_id: "eligible".into(),
                relation: Relation::RelatedTo,
            });
        }
        store.capture(request).unwrap();
    }
    for _ in 0..2 {
        let hits = store
            .search(query("Cortex", "applicabilitymarker"))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.id, "eligible");
        let mut historical = query("Cortex", "applicabilitymarker");
        historical.include_inactive = true;
        assert_eq!(store.search(historical).unwrap().len(), 4);
        assert!(store.get("Cortex", "stale").unwrap().is_some());
        let graph = store
            .graph(GraphQuery {
                repository: "Cortex".into(),
                id: "eligible".into(),
                depth: 5,
                limit: 100,
            })
            .unwrap();
        assert_eq!(graph.records.len(), 1);
        assert!(graph.edges.is_empty());
        assert!(store
            .graph(GraphQuery {
                repository: "Cortex".into(),
                id: "stale".into(),
                depth: 5,
                limit: 100
            })
            .unwrap()
            .records
            .is_empty());
        store.reindex().unwrap();
    }
}

fn paths(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !root.exists() {
        return result;
    }
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(paths(&path, extension));
        } else if path.extension().is_some_and(|found| found == extension) {
            result.push(path);
        }
    }
    result.sort();
    result
}

#[test]
fn immutable_memories_versions_provenance_and_advisory_metadata_rebuild() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let input = memory("Cortex", "lesson", "first memory");
    let first = store.capture(input.clone()).unwrap();
    assert!(first.created);
    let canonical = paths(root.path(), "md");
    assert_eq!(canonical.len(), 1);
    let original_bytes = std::fs::read(&canonical[0]).unwrap();
    let original_text = String::from_utf8(original_bytes.clone()).unwrap();
    assert!(original_text.starts_with("---json\n{"));
    assert!(original_text.ends_with("\n---\nfirst memory"));
    assert!(!store.capture(input.clone()).unwrap().created);
    let changed = store.revise("Cortex","lesson",1,RevisionRequest {
        title:"Corrected memory".into(),body:"corrected searchablemarker".into(),authority:Authority::Accepted,
        links:vec![TypedLink { target_id:"missing".into(),relation:Relation::DerivedFrom }],
        metadata:json!({"advisory":{"model":"test-model","probabilities":{"lesson":0.9,"unknown":0.1}}}),
    }).unwrap().record;
    assert_eq!(changed.revision, 2);
    assert_eq!(changed.source, first.record.source);
    assert_eq!(changed.created_at, first.record.created_at);
    assert_eq!(std::fs::read(&canonical[0]).unwrap(), original_bytes);
    assert_eq!(
        store.get_revision("Cortex", "lesson", 1).unwrap().unwrap(),
        first.record
    );
    assert_eq!(store.capture(input).unwrap().record, first.record);
    assert_eq!(store.search(query("Cortex", "first")).unwrap().len(), 0);
    assert_eq!(
        store.search(query("Cortex", "searchablemarker")).unwrap()[0].record,
        changed
    );
    let report = store.reindex().unwrap();
    assert_eq!(
        (
            report.files,
            report.records,
            report.revisions,
            report.dangling_links
        ),
        (2, 1, 2, 1)
    );
    assert_eq!(store.get("Cortex", "lesson").unwrap().unwrap(), changed);
    assert_eq!(
        store.get_revision("Cortex", "lesson", 1).unwrap().unwrap(),
        first.record
    );
}

#[test]
fn raw_events_use_bounded_segments_and_never_promote_authority() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let requests: Vec<_> = (0..64).map(|i| event(&format!("event-{i}"))).collect();
    let outcome = store.capture_batch(requests.clone()).unwrap();
    assert_eq!((outcome.created, outcome.duplicates), (64, 0));
    assert_eq!(paths(root.path(), "jsonl").len(), 1);
    assert!(paths(root.path(), "md").is_empty());
    let duplicate = store.capture_batch(requests).unwrap();
    assert_eq!((duplicate.created, duplicate.duplicates), (0, 64));
    assert_eq!(paths(root.path(), "jsonl").len(), 1);
    assert_eq!(
        store
            .search(query("Cortex", "searchablemarker"))
            .unwrap()
            .len(),
        64
    );
    assert!(store
        .capture(CaptureRequest {
            authority: Authority::Accepted,
            ..event("invalid")
        })
        .is_err());
    assert!(store
        .revise(
            "Cortex",
            "event-0",
            1,
            RevisionRequest {
                title: "bad promotion".into(),
                body: String::new(),
                authority: Authority::Accepted,
                links: vec![],
                metadata: json!({}),
            }
        )
        .is_err());
    let report = store.reindex().unwrap();
    assert_eq!(
        (report.files, report.records, report.revisions),
        (1, 64, 64)
    );
    assert_eq!(store.status(Some("Cortex")).unwrap().events, 64);
    assert_eq!(
        store.get("Cortex", "event-0").unwrap().unwrap().authority,
        Authority::Observation
    );
}

#[test]
fn conflicting_batch_publishes_nothing_and_in_batch_duplicates_are_idempotent() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let one = event("one");
    let outcome = store.capture_batch(vec![one.clone(), one.clone()]).unwrap();
    assert_eq!((outcome.created, outcome.duplicates), (1, 1));
    assert_eq!(outcome.records[0].record, outcome.records[1].record);
    let error = store
        .capture_batch(vec![
            event("not-published"),
            CaptureRequest {
                body: "different".into(),
                ..one
            },
        ])
        .unwrap_err();
    assert!(matches!(error, LocalError::Conflict(_)));
    assert!(store.get("Cortex", "not-published").unwrap().is_none());
    assert_eq!(paths(root.path(), "jsonl").len(), 1);
    assert_eq!(store.status(None).unwrap().records, 1);
}

#[test]
fn missing_index_rebuilds_memories_events_versions_and_links() {
    let source = TempDir::new().unwrap();
    let restored = TempDir::new().unwrap();
    let store = LocalStore::open(source.path()).unwrap();
    store.capture(event("event")).unwrap();
    let mut request = memory("Cortex", "memory", "recoverymarker");
    request.links.push(TypedLink {
        target_id: "event".into(),
        relation: Relation::DerivedFrom,
    });
    let memory = store.capture(request).unwrap().record;
    copy_tree(
        &source.path().join("records"),
        &restored.path().join("records"),
    );
    copy_tree(
        &source.path().join("segments"),
        &restored.path().join("segments"),
    );
    let recovered = LocalStore::open(restored.path()).unwrap();
    assert_eq!(recovered.get("Cortex", "memory").unwrap().unwrap(), memory);
    assert_eq!(recovered.status(None).unwrap().records, 2);
    let graph = recovered
        .graph(GraphQuery {
            repository: "Cortex".into(),
            id: "memory".into(),
            depth: 1,
            limit: 10,
        })
        .unwrap();
    assert_eq!((graph.records.len(), graph.edges.len()), (2, 1));
    assert_eq!(
        graph.records[1].source.payload["unknown_field"]["preserved"],
        true
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn interrupted_canonical_publication_recovers_on_next_open() {
    let donor = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();
    let donor_store = LocalStore::open(donor.path()).unwrap();
    let event = donor_store
        .capture(event("interrupted-event"))
        .unwrap()
        .record;
    let target_store = LocalStore::open(target.path()).unwrap();
    target_store
        .capture(memory("Cortex", "existing", "existingmarker"))
        .unwrap();
    copy_tree(
        &donor.path().join("segments"),
        &target.path().join("segments"),
    );
    std::fs::write(
        target.path().join("pending"),
        b"interrupted before SQL commit",
    )
    .unwrap();
    let recovered = LocalStore::open(target.path()).unwrap();
    assert!(!target.path().join("pending").exists());
    assert_eq!(
        recovered
            .get("Cortex", "interrupted-event")
            .unwrap()
            .unwrap(),
        event
    );
    assert!(recovered.get("Cortex", "existing").unwrap().is_some());
    assert!(
        !recovered
            .capture(CaptureRequest {
                id: Some(event.id.clone()),
                repository: event.repository.clone(),
                kind: event.kind,
                authority: event.authority,
                title: event.title.clone(),
                body: event.body.clone(),
                source: event.source.clone(),
                links: event.links.clone(),
                metadata: event.metadata.clone(),
            })
            .unwrap()
            .created
    );
}

#[test]
fn invalid_canonical_file_rolls_back_reindex_and_keeps_searchable_projection() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let first = store
        .capture(memory("Cortex", "first", "validmarker"))
        .unwrap()
        .record;
    store.capture(event("event")).unwrap();
    let path = paths(root.path(), "md").remove(0);
    std::fs::write(path, b"---json\n{invalid JSON}\n---\ncorruption").unwrap();
    assert!(store.reindex().is_err());
    assert_eq!(store.get("Cortex", "first").unwrap().unwrap(), first);
    assert_eq!(
        store.search(query("Cortex", "validmarker")).unwrap()[0].record,
        first
    );
    assert_eq!(store.status(None).unwrap().records, 2);
}

#[test]
fn graph_cycles_dangling_targets_inactive_records_and_scopes_are_bounded() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for (id, target) in [("a", "b"), ("b", "c"), ("c", "a")] {
        store
            .capture(CaptureRequest {
                links: vec![TypedLink {
                    target_id: target.into(),
                    relation: Relation::Supports,
                }],
                ..memory("Cortex", id, "graphmarker")
            })
            .unwrap();
    }
    store
        .capture(CaptureRequest {
            links: vec![TypedLink {
                target_id: "foreign-only".into(),
                relation: Relation::RelatedTo,
            }],
            ..memory("Cortex", "dangling", "graphmarker")
        })
        .unwrap();
    store
        .capture(memory("cortex", "foreign-only", "graphmarker"))
        .unwrap();
    store
        .capture(CaptureRequest {
            authority: Authority::Deprecated,
            links: vec![TypedLink {
                target_id: "a".into(),
                relation: Relation::Contradicts,
            }],
            ..memory("Cortex", "inactive", "graphmarker")
        })
        .unwrap();
    let graph = store
        .graph(GraphQuery {
            repository: "Cortex".into(),
            id: "a".into(),
            depth: 5,
            limit: 10,
        })
        .unwrap();
    assert_eq!((graph.records.len(), graph.edges.len()), (3, 3));
    assert!(graph
        .records
        .iter()
        .all(|record| record.repository == "Cortex"));
    let limited = store
        .graph(GraphQuery {
            repository: "Cortex".into(),
            id: "a".into(),
            depth: 5,
            limit: 2,
        })
        .unwrap();
    assert_eq!(limited.records.len(), 2);
    assert!(limited.truncated);
    let dangling = store
        .graph(GraphQuery {
            repository: "Cortex".into(),
            id: "dangling".into(),
            depth: 5,
            limit: 10,
        })
        .unwrap();
    assert_eq!(dangling.records.len(), 1);
    assert!(dangling.edges.is_empty());
    assert_eq!(store.reindex().unwrap().dangling_links, 1);
    assert_eq!(
        store.search(query("Cortex", "graphmarker")).unwrap().len(),
        4
    );
    let mut all = query("Cortex", "graphmarker");
    all.include_inactive = true;
    assert_eq!(store.search(all).unwrap().len(), 5);
}

#[test]
fn other_repositories_cannot_change_ranking_scores_or_scoped_counts() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for id in ["older", "newer"] {
        let mut input = memory("Cortex", id, "scopemarker");
        input.source.captured_at = Some(
            if id == "older" {
                "2020-01-01T00:00:00Z"
            } else {
                "2025-01-01T00:00:00Z"
            }
            .into(),
        );
        store.capture(input).unwrap();
    }
    let before = store.search(query("Cortex", "scopemarker")).unwrap();
    assert_eq!(before[0].record.id, "newer");
    for i in 0..24 {
        store
            .capture(memory(
                "cortex",
                &format!("foreign-{i}"),
                &"scopemarker ".repeat(i + 1),
            ))
            .unwrap();
    }
    let after = store.search(query("Cortex", "scopemarker")).unwrap();
    assert_eq!(
        before
            .iter()
            .map(|hit| (&hit.record.id, hit.score))
            .collect::<Vec<_>>(),
        after
            .iter()
            .map(|hit| (&hit.record.id, hit.score))
            .collect::<Vec<_>>()
    );
    let scoped = store.status(Some("Cortex")).unwrap();
    assert_eq!((scoped.records, scoped.repositories), (2, 1));
    assert_eq!(store.status(None).unwrap().records, 26);
    assert!(store.get("CORTEX", "older").unwrap().is_none());
}

#[test]
fn payload_and_batch_bounds_reject_before_any_canonical_publication() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let oversized_body = CaptureRequest {
        body: "x".repeat(MAX_BODY_BYTES + 1),
        ..event("body")
    };
    assert!(store.capture(oversized_body).is_err());
    let oversized_payload = CaptureRequest {
        source: Source {
            payload: json!("x".repeat(MAX_RECORD_BYTES)),
            ..Default::default()
        },
        ..event("payload")
    };
    assert!(store.capture(oversized_payload).is_err());
    assert!(store.capture_batch(Vec::new()).is_err());
    assert!(store
        .capture_batch(
            (0..MAX_BATCH_RECORDS + 1)
                .map(|id| event(&id.to_string()))
                .collect()
        )
        .is_err());
    let large_batch = (0..5)
        .map(|id| CaptureRequest {
            source: Source {
                payload: json!("x".repeat(2 * 1024 * 1024)),
                ..Default::default()
            },
            ..event(&id.to_string())
        })
        .collect();
    assert!(store.capture_batch(large_batch).is_err());
    assert_eq!(store.status(None).unwrap().records, 0);
    assert!(paths(root.path(), "jsonl").is_empty());
    // A full 1 MiB original payload plus its readable projection fits the cap.
    let valid = CaptureRequest {
        body: "x".repeat(MAX_BODY_BYTES),
        source: Source {
            payload: json!("x".repeat(MAX_BODY_BYTES)),
            ..Default::default()
        },
        ..event("maximum-source")
    };
    store.capture(valid).unwrap();
    store.reindex().unwrap();
    assert_eq!(store.status(None).unwrap().records, 1);
}

#[test]
fn newer_index_schema_is_rejected_without_rewriting_it() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    store
        .capture(memory("Cortex", "existing", "preservedmarker"))
        .unwrap();
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    connection.close().unwrap();
    assert!(matches!(
        LocalStore::open(root.path()),
        Err(LocalError::Invalid(_))
    ));
    let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 99);
    let records: i64 = connection
        .query_row("SELECT count(*) FROM current_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(records, 1);
}

#[test]
fn incomplete_revision_chain_does_not_become_a_new_valid_store() {
    let source = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();
    let store = LocalStore::open(source.path()).unwrap();
    store
        .capture(memory("Cortex", "record", "initial"))
        .unwrap();
    store
        .revise(
            "Cortex",
            "record",
            1,
            RevisionRequest {
                title: "revision".into(),
                body: "later".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: json!({}),
            },
        )
        .unwrap();
    let revision = paths(source.path(), "md")
        .into_iter()
        .find(|path| path.file_name().unwrap() == "00000000000000000002.md")
        .unwrap();
    let relative = revision.strip_prefix(source.path()).unwrap();
    let destination = target.path().join(relative);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(revision, &destination).unwrap();
    assert!(LocalStore::open(target.path()).is_err());
    // Failed initialization retains its recovery marker; a retry must not expose
    // the empty SQLite file created by the first failed reconstruction.
    assert!(target.path().join("pending").exists());
    assert!(LocalStore::open(target.path()).is_err());
}

#[test]
fn zero_byte_and_empty_sqlite_indexes_rebuild_existing_canonical_records() {
    let source = TempDir::new().unwrap();
    let store = LocalStore::open(source.path()).unwrap();
    let original = store
        .capture(memory("Cortex", "preserved", "recoverymarker"))
        .unwrap()
        .record;
    for empty_sqlite in [false, true] {
        let target = TempDir::new().unwrap();
        copy_tree(
            &source.path().join("records"),
            &target.path().join("records"),
        );
        let index_path = target.path().join("index.sqlite3");
        if empty_sqlite {
            let connection = rusqlite::Connection::open(index_path).unwrap();
            connection.execute_batch("PRAGMA user_version=0;").unwrap();
            connection.close().unwrap();
        } else {
            std::fs::write(index_path, []).unwrap();
        }
        assert!(!target.path().join("pending").exists());
        let recovered = LocalStore::open(target.path()).unwrap();
        assert_eq!(
            recovered.get("Cortex", "preserved").unwrap().unwrap(),
            original
        );
        assert_eq!(
            recovered
                .search(query("Cortex", "recoverymarker"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(recovered.status(None).unwrap().records, 1);
    }
}
