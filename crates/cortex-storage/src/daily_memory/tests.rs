use super::*;

fn capture(id: &str, body: &str) -> MemoryCapture {
    let mut request = MemoryCapture::new("repo", "Explicit lesson", body, MemoryCategory::Lesson);
    request.id = Some(id.to_owned());
    request
}

fn file_store() -> (tempfile::TempDir, MetadataStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MetadataStore::open(&dir.path().join("metadata.sqlite")).unwrap();
    (dir, store)
}

fn scope() -> MemoryScope {
    MemoryScope::Repository("repo".to_owned())
}

#[test]
fn file_restart_preserves_exact_content_provenance_evidence_and_fts() {
    let (dir, mut store) = file_store();
    let mut request = capture("lesson-1", "Preserve original source, ação and 记忆.");
    request.origin = MemoryOrigin {
        agent: Some("codex".to_owned()),
        session_id: Some("observed-session".to_owned()),
        turn_id: Some("observed-turn".to_owned()),
        attribution: OriginAttribution::TrustedTransport,
    };
    request.sources = vec![MemorySource {
        event_id: "legacy-event".to_owned(),
        kind: Some(Kind::Learning),
        reference: Some("src/lib.rs".to_owned()),
        content_hash: Some("sha256:source".to_owned()),
        revision: Some("commit-123".to_owned()),
    }];
    let receipt = store.capture_daily_memory(&request).unwrap();
    drop(store);

    let store = MetadataStore::open(&dir.path().join("metadata.sqlite")).unwrap();
    let record = store.daily_memory(&scope(), "lesson-1").unwrap().unwrap();
    assert_eq!(record.body, request.body);
    assert_eq!(record.origin, request.origin);
    assert_eq!(record.sources, request.sources);
    assert_eq!(record.event_id, receipt.event_id);
    assert_eq!(record.content_hash, receipt.content_hash);
    assert_eq!(record.captured_at, receipt.captured_at);
    assert_eq!(store.daily_memory_revision(&scope()).unwrap(), 1);
    let evidence = store.daily_memory_evidence(&scope(), "lesson-1").unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].kind, Kind::Decision);
    assert_eq!(evidence[0].payload.status, "accepted");
    assert_eq!(evidence[0].payload.body, request.body);
    assert_eq!(evidence[0].payload.tags, ["daily-memory", "lesson"]);
    assert_eq!(evidence[0].origin, request.origin);
    assert_eq!(evidence[0].sources, request.sources);
    for term in ["original", "acao", "记忆"] {
        let result = store
            .query_daily_memories(&MemoryQuery::new("repo", term))
            .unwrap();
        assert_eq!(
            result.records.as_slice(),
            std::slice::from_ref(&record),
            "term: {term}"
        );
        assert_eq!(result.revision, receipt.scope_revision);
    }
}

#[test]
fn unicode_combining_marks_preserve_whole_word_lookup_without_fts_operators() {
    let (_dir, mut store) = file_store();
    let decomposed = "ac\u{0327}a\u{0303}o";
    for (repo, body) in [("precomposed", "ação"), ("decomposed", decomposed)] {
        let mut request = capture("unicode", body);
        request.scope = MemoryScope::Repository(repo.to_owned());
        store.capture_daily_memory(&request).unwrap();
        for query in [
            "ação",
            decomposed,
            "acao",
            &format!("\"{decomposed}\" OR NOT * : NEAR( )"),
        ] {
            let result = store
                .query_daily_memories(&MemoryQuery::new(repo, query))
                .unwrap();
            assert_eq!(result.records.len(), 1, "repo: {repo}, query: {query}");
            assert_eq!(result.records[0].id, "unicode");
            assert_eq!(result.records[0].body, body);
        }
        let marks_only = store
            .query_daily_memories(&MemoryQuery::new(repo, "\u{0327}\u{0303}\u{20dd}\u{093e}"))
            .unwrap();
        assert!(marks_only.records.is_empty());
    }

    // Non-Latin marks use the complete Unicode categories as well.
    store
        .capture_daily_memory(&capture("hebrew", "שָׁלוֹם"))
        .unwrap();
    let result = store
        .query_daily_memories(&MemoryQuery::new("repo", "שָׁלוֹם"))
        .unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].id, "hebrew");
}

#[test]
fn retry_conflict_and_existing_id_never_add_evidence_or_revision() {
    let (_dir, mut store) = file_store();
    let mut request = capture("stable", "Persist only once.");
    request.retry = Some(MemoryRetryKey {
        caller: "client".to_owned(),
        key: "retry".to_owned(),
    });
    request.expected_revision = Some(0);
    let first = store.capture_daily_memory(&request).unwrap();
    assert_eq!(store.capture_daily_memory(&request).unwrap(), first);
    request.body.push_str(" Different.");
    assert!(matches!(
        store.capture_daily_memory(&request),
        Err(MemoryError::IdempotencyConflict)
    ));
    request.retry = None;
    request.expected_revision = Some(1);
    assert!(matches!(
        store.capture_daily_memory(&request),
        Err(MemoryError::AlreadyExists)
    ));
    assert_eq!(store.daily_memory_revision(&scope()).unwrap(), 1);
    assert_eq!(
        store
            .daily_memory_evidence(&scope(), "stable")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .daily_memory(&scope(), "stable")
            .unwrap()
            .unwrap()
            .body,
        "Persist only once."
    );
}

#[test]
fn stale_marker_excludes_advice_and_preserves_actor_and_version_history() {
    let (_dir, mut store) = file_store();
    let request = capture("source", "Known source advice");
    let first = store.capture_daily_memory(&request).unwrap();
    let actor = MemoryOrigin {
        agent: Some("source-validator".to_owned()),
        ..MemoryOrigin::default()
    };
    let stale = store
        .mark_daily_memory_stale(&scope(), "source", true, first.scope_revision, &actor)
        .unwrap();
    assert_eq!(stale.version, 2);
    assert_eq!(stale.scope_revision, 2);
    assert_ne!(first.content_hash, stale.content_hash);
    assert!(store
        .query_daily_memories(&MemoryQuery::new("repo", "source"))
        .unwrap()
        .records
        .is_empty());
    let unchanged = store
        .mark_daily_memory_stale(&scope(), "source", true, 2, &actor)
        .unwrap();
    assert_eq!(unchanged, stale);
    assert!(matches!(
        store.mark_daily_memory_stale(&scope(), "source", false, 1, &actor),
        Err(MemoryError::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    let fresh = store
        .mark_daily_memory_stale(&scope(), "source", false, 2, &actor)
        .unwrap();
    assert_eq!(fresh.version, 3);
    assert_eq!(fresh.content_hash, first.content_hash);
    let record = store.daily_memory(&scope(), "source").unwrap().unwrap();
    assert_eq!(record.origin, MemoryOrigin::default());
    assert_eq!(record.event_id, first.event_id);
    let history = store.daily_memory_evidence(&scope(), "source").unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[1].origin, actor);
    assert!(history[1].stale);
    assert!(!history[2].stale);
    assert_eq!(
        store
            .query_daily_memories(&MemoryQuery::new("repo", "source"))
            .unwrap()
            .records,
        [record]
    );
}

#[test]
fn invalid_supersession_cannot_change_lifecycle() {
    let (_dir, mut store) = file_store();
    store
        .capture_daily_memory(&capture("original", "Keep accepted"))
        .unwrap();
    let mut correction = capture("successor", "Explicit correction");
    correction.supersedes = Some("original".to_owned());
    assert!(matches!(
        store.capture_daily_memory(&correction),
        Err(MemoryError::InvalidInput(_))
    ));
    correction.expected_revision = Some(1);
    correction.status = MemoryStatus::Proposed;
    assert!(matches!(
        store.capture_daily_memory(&correction),
        Err(MemoryError::InvalidInput(_))
    ));
    correction.status = MemoryStatus::Accepted;
    correction.supersedes = correction.id.clone();
    assert!(matches!(
        store.capture_daily_memory(&correction),
        Err(MemoryError::InvalidInput(_))
    ));
    correction.supersedes = Some("missing".to_owned());
    assert!(matches!(
        store.capture_daily_memory(&correction),
        Err(MemoryError::NotFound)
    ));
    assert_eq!(store.daily_memory_revision(&scope()).unwrap(), 1);
    assert_eq!(
        store
            .daily_memory(&scope(), "original")
            .unwrap()
            .unwrap()
            .status,
        MemoryStatus::Accepted
    );
    assert!(store.daily_memory(&scope(), "successor").unwrap().is_none());
}

#[test]
fn lifecycle_filter_pinned_lookup_and_literal_input_have_bounded_results() {
    let (_dir, mut store) = file_store();
    for (id, status) in [
        ("accepted", MemoryStatus::Accepted),
        ("proposed", MemoryStatus::Proposed),
        ("deprecated", MemoryStatus::Deprecated),
    ] {
        let mut request = capture(id, "alpha searchable");
        request.status = status;
        store.capture_daily_memory(&request).unwrap();
    }
    let mut pinned = capture("pinned", "Scope-wide instruction");
    pinned.pinned = true;
    store.capture_daily_memory(&pinned).unwrap();
    for query in [
        "alpha",
        "alpha OR NOT \"* : NEAR( )",
        "alpha'; DROP TABLE daily_memory_records;--",
    ] {
        let result = store
            .query_daily_memories(&MemoryQuery::new("repo", query))
            .unwrap();
        assert_eq!(
            result
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["pinned", "accepted"]
        );
    }
    let empty = store
        .query_daily_memories(&MemoryQuery::new("repo", "***"))
        .unwrap();
    assert_eq!(empty.records.len(), 1);
    assert_eq!(empty.records[0].id, "pinned");
    let mut zero = MemoryQuery::new("repo", "alpha");
    zero.limit = 0;
    let result = store.query_daily_memories(&zero).unwrap();
    assert!(result.records.is_empty());
    assert_eq!(result.revision, 4);
}

#[test]
fn invalid_and_oversized_input_never_becomes_global_or_commits() {
    let (_dir, mut store) = file_store();
    let valid = capture("valid", "body");
    for repo in ["", " repo", "repo ", "\0"] {
        let mut request = valid.clone();
        request.scope = MemoryScope::Repository(repo.to_owned());
        assert!(matches!(
            store.capture_daily_memory(&request),
            Err(MemoryError::InvalidInput(_))
        ));
        assert!(matches!(
            store.query_daily_memories(&MemoryQuery::new(repo, "body")),
            Err(MemoryError::InvalidInput(_))
        ));
    }
    for path in [
        "/absolute",
        "../escape",
        "src//file",
        "src/./file",
        "C:/file",
        "src\\file",
    ] {
        let mut request = valid.clone();
        request.path = Some(path.to_owned());
        assert!(matches!(
            store.capture_daily_memory(&request),
            Err(MemoryError::InvalidInput(_))
        ));
        let mut query = MemoryQuery::new("repo", "body");
        query.paths.push(path.to_owned());
        assert!(matches!(
            store.query_daily_memories(&query),
            Err(MemoryError::InvalidInput(_))
        ));
    }
    let mut large = valid.clone();
    large.body = "x".repeat(8193);
    assert!(matches!(
        store.capture_daily_memory(&large),
        Err(MemoryError::InvalidInput(_))
    ));
    let mut large = MemoryQuery::new("repo", &"x".repeat(4097));
    assert!(matches!(
        store.query_daily_memories(&large),
        Err(MemoryError::InvalidInput(_))
    ));
    large.query = (0..33)
        .map(|i| format!("term{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(matches!(
        store.query_daily_memories(&large),
        Err(MemoryError::InvalidInput(_))
    ));
    assert_eq!(store.daily_memory_revision(&scope()).unwrap(), 0);
    assert_eq!(
        store.daily_memory_revision(&MemoryScope::Global).unwrap(),
        0
    );
}

#[test]
fn late_transaction_failure_rolls_back_predecessor_evidence_fts_and_revision() {
    let (dir, mut store) = file_store();
    let original = store
        .capture_daily_memory(&capture("original", "predecessorunique"))
        .unwrap();
    let before = store.daily_memory(&scope(), "original").unwrap().unwrap();
    store.conn().execute_batch("CREATE TRIGGER fail_daily_memory_retry BEFORE INSERT ON daily_memory_retries BEGIN SELECT RAISE(ABORT,'injected late failure'); END;").unwrap();
    let mut correction = capture("successor", "successorunique");
    correction.supersedes = Some("original".to_owned());
    correction.expected_revision = Some(original.scope_revision);
    correction.retry = Some(MemoryRetryKey {
        caller: "client".to_owned(),
        key: "fails".to_owned(),
    });
    assert!(matches!(
        store.capture_daily_memory(&correction),
        Err(MemoryError::Sqlite(_))
    ));
    drop(store);

    let store = MetadataStore::open(&dir.path().join("metadata.sqlite")).unwrap();
    assert_eq!(
        store.daily_memory(&scope(), "original").unwrap().unwrap(),
        before
    );
    assert!(store.daily_memory(&scope(), "successor").unwrap().is_none());
    assert_eq!(
        store
            .daily_memory_evidence(&scope(), "original")
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .daily_memory_evidence(&scope(), "successor")
        .unwrap()
        .is_empty());
    assert_eq!(store.daily_memory_revision(&scope()).unwrap(), 1);
    assert_eq!(
        store
            .query_daily_memories(&MemoryQuery::new("repo", "predecessorunique"))
            .unwrap()
            .records,
        [before]
    );
    assert!(store
        .query_daily_memories(&MemoryQuery::new("repo", "successorunique"))
        .unwrap()
        .records
        .is_empty());
    let retries: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM daily_memory_retries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(retries, 0);
}
