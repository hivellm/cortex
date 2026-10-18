use cortex_local::*;
use serde_json::json;

fn capture(store: &LocalStore, repository: &str, id: &str, kind: RecordKind) -> Record {
    store
        .capture(CaptureRequest {
            id: Some(id.into()),
            repository: repository.into(),
            kind,
            title: id.into(),
            body: "dashboard fixture".into(),
            source: Source {
                kind: "fixture".into(),
                locator: "original source".into(),
                payload: json!({"original":true}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}

fn query(repository: &str) -> BrowseQuery {
    BrowseQuery {
        repository: repository.into(),
        kind: None,
        limit: 2,
        cursor: None,
        include_inactive: false,
    }
}

#[test]
fn keyset_pagination_preserves_ties_scope_kind_and_source() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    for id in ["c", "a", "b", "d", "e"] {
        capture(&store, "Cortex", id, RecordKind::Event);
    }
    capture(&store, "cortex", "a", RecordKind::Event);
    capture(&store, "Cortex", "note", RecordKind::Note);
    // Pin projection times to exercise a tie spanning multiple page boundaries.
    let connection = rusqlite::Connection::open(home.path().join("index.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE revisions SET record=json_set(record,'$.updated_at','2026-10-18T12:00:00Z')",
            [],
        )
        .unwrap();
    drop(connection);
    let mut request = query("Cortex");
    request.kind = Some(RecordKind::Event);
    let mut ids = Vec::new();
    loop {
        let page = store.browse(request.clone()).unwrap();
        assert!(page.records.len() <= 2);
        for record in page.records {
            assert_eq!(record.source.locator, "original source");
            assert_eq!(record.source.payload, json!({"original":true}));
            ids.push(record.id);
        }
        request.cursor = page.next_cursor;
        if request.cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids, ["a", "b", "c", "d", "e"]);
}

#[test]
fn cursor_is_bound_to_filters_and_limits_are_checked() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    for id in ["a", "b", "c"] {
        capture(&store, "Cortex", id, RecordKind::Note);
    }
    let cursor = store.browse(query("Cortex")).unwrap().next_cursor.unwrap();
    for change in 0..3 {
        let mut request = query("Cortex");
        request.cursor = Some(cursor.clone());
        match change {
            0 => request.repository = "cortex".into(),
            1 => request.kind = Some(RecordKind::Event),
            _ => request.include_inactive = true,
        }
        assert!(store.browse(request).is_err());
    }
    for bad in ["not JSON", "{}", &"x".repeat(8193)] {
        let mut request = query("Cortex");
        request.cursor = Some(bad.into());
        assert!(store.browse(request).is_err());
    }
    for limit in [0, 101, usize::MAX] {
        let mut request = query("Cortex");
        request.limit = limit;
        assert!(store.browse(request).is_err());
    }
    let defaults: BrowseQuery = serde_json::from_value(json!({"repository":"Cortex"})).unwrap();
    assert_eq!(defaults.limit, 50);
}

#[test]
fn inventory_counts_all_current_records_and_links_without_sampling() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    assert!(store.repositories().unwrap().repositories.is_empty());
    capture(&store, "Cortex", "event", RecordKind::Event);
    capture(&store, "cortex", "event", RecordKind::Event);
    let note = capture(&store, "Cortex", "note", RecordKind::Note);
    store
        .revise(
            "Cortex",
            "note",
            note.revision,
            RevisionRequest {
                title: "updated".into(),
                body: "updated".into(),
                authority: Authority::Deprecated,
                links: vec![TypedLink {
                    target_id: "event".into(),
                    relation: Relation::DerivedFrom,
                }],
                metadata: json!({}),
            },
        )
        .unwrap();
    for number in 0..101 {
        capture(
            &store,
            "Cortex",
            &format!("entry-{number}"),
            RecordKind::Note,
        );
    }
    let inventory = store.repositories().unwrap();
    assert_eq!(inventory.repositories.len(), 2);
    let first = &inventory.repositories[0];
    assert_eq!(first.repository, "Cortex");
    assert_eq!(
        (first.records, first.memories, first.events, first.links),
        (103, 102, 1, 1)
    );
    assert_eq!(inventory.repositories[1].repository, "cortex");
    assert_eq!(inventory.repositories[1].records, 1);
}

#[test]
fn active_browsing_matches_context_applicability_rules() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    capture(&store, "Cortex", "active", RecordKind::Note);
    for (id, authority, metadata) in [
        ("deprecated", Authority::Deprecated, json!({})),
        ("superseded", Authority::Superseded, json!({})),
        (
            "stale",
            Authority::Accepted,
            json!({"applicability":{"stale":true}}),
        ),
        (
            "branch",
            Authority::Accepted,
            json!({"applicability":{"branch":"main"}}),
        ),
        (
            "path",
            Authority::Accepted,
            json!({"applicability":{"path":"src"}}),
        ),
    ] {
        store
            .capture(CaptureRequest {
                id: Some(id.into()),
                repository: "Cortex".into(),
                title: id.into(),
                body: "fixture".into(),
                authority,
                metadata,
                ..Default::default()
            })
            .unwrap();
    }
    let page = store.browse(query("Cortex")).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].id, "active");
    assert!(page.next_cursor.is_none());
    let mut request = query("Cortex");
    request.limit = 100;
    request.include_inactive = true;
    assert_eq!(store.browse(request).unwrap().records.len(), 6);
    assert_eq!(store.repositories().unwrap().repositories[0].records, 6);
}
