use std::sync::{Arc, Barrier};

use cortex_storage::daily_memory::{
    MemoryCapture, MemoryCategory, MemoryError, MemoryQuery, MemoryRetryKey, MemoryScope,
    MemoryStatus,
};
use cortex_storage::MetadataStore;

#[test]
fn concurrent_corrections_have_one_winner_and_preserve_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let scope = MemoryScope::Repository("repo/concurrency".into());
    let original = store
        .capture_daily_memory(&MemoryCapture::new(
            "repo/concurrency",
            "sqliteguard original",
            "Original captured decision.",
            MemoryCategory::Decision,
        ))
        .unwrap();
    let left = MetadataStore::open(&path).unwrap();
    let right = MetadataStore::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));

    let results = std::thread::scope(|threads| {
        let mut handles = Vec::new();
        for (mut connection, title) in [(left, "left"), (right, "right")] {
            let barrier = Arc::clone(&barrier);
            let mut correction = MemoryCapture::new(
                "repo/concurrency",
                &format!("sqliteguard {title}"),
                "Use the explicitly accepted correction.",
                MemoryCategory::Decision,
            );
            correction.expected_revision = Some(original.scope_revision);
            correction.supersedes = Some(original.id.clone());
            handles.push(threads.spawn(move || {
                barrier.wait();
                connection.capture_daily_memory(&correction)
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let loser = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert!(
        matches!(
            loser,
            MemoryError::RevisionConflict { expected, actual }
                if *expected == original.scope_revision && *actual == original.scope_revision + 1
        ),
        "competing correction must fail as a revision conflict: {loser}"
    );
    let winner = results
        .iter()
        .find_map(|result| result.as_ref().ok())
        .unwrap();
    drop(store);
    let reopened = MetadataStore::open(&path).unwrap();
    let memories = reopened
        .query_daily_memories(&MemoryQuery::new("repo/concurrency", "sqliteguard"))
        .unwrap();
    assert_eq!(memories.revision, original.scope_revision + 1);
    assert_eq!(memories.records.len(), 1);
    assert_eq!(memories.records[0].id, winner.id);
    let predecessor = reopened
        .daily_memory(&scope, &original.id)
        .unwrap()
        .unwrap();
    assert_eq!(predecessor.status, MemoryStatus::Superseded);
    let evidence = reopened
        .daily_memory_evidence(&scope, &original.id)
        .unwrap();
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0].payload.status, "accepted");
    assert_eq!(evidence[1].payload.status, "superseded");
}

#[test]
fn retry_after_correction_returns_original_receipt_without_resurrection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let scope = MemoryScope::Repository("repo/retry".into());
    let mut request = MemoryCapture::new(
        "repo/retry",
        "retryguard original",
        "Old decision, replaced by an explicit correction.",
        MemoryCategory::Decision,
    );
    request.expected_revision = Some(0);
    request.retry = Some(MemoryRetryKey {
        caller: "transport-session".into(),
        key: "original-attempt".into(),
    });
    let original = store.capture_daily_memory(&request).unwrap();
    let mut correction = MemoryCapture::new(
        "repo/retry",
        "retryguard correction",
        "Follow this corrected decision.",
        MemoryCategory::Decision,
    );
    correction.supersedes = Some(original.id.clone());
    correction.expected_revision = Some(original.scope_revision);
    let successor = store.capture_daily_memory(&correction).unwrap();
    drop(store);

    let mut reopened = MetadataStore::open(&path).unwrap();
    assert_eq!(reopened.capture_daily_memory(&request).unwrap(), original);
    assert_eq!(
        reopened.daily_memory_revision(&scope).unwrap(),
        successor.scope_revision
    );
    assert_eq!(
        reopened
            .daily_memory(&scope, &original.id)
            .unwrap()
            .unwrap()
            .status,
        MemoryStatus::Superseded
    );
    let result = reopened
        .query_daily_memories(&MemoryQuery::new("repo/retry", "retryguard"))
        .unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].id, successor.id);
    request.body = "Different content under the same retry key.".into();
    assert!(matches!(
        reopened.capture_daily_memory(&request),
        Err(MemoryError::IdempotencyConflict)
    ));
    assert_eq!(
        reopened.daily_memory_revision(&scope).unwrap(),
        successor.scope_revision
    );
}

#[test]
fn exact_scopes_survive_slug_collisions_and_globals_require_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&dir.path().join("metadata.sqlite")).unwrap();
    let repos = ["team/repo.one", "team/repo-one"];
    for repo in repos {
        let mut capture = MemoryCapture::new(repo, "isolationtoken", repo, MemoryCategory::Lesson);
        capture.id = Some("DEC-001".into());
        store.capture_daily_memory(&capture).unwrap();
    }
    let mut global = MemoryCapture::new(
        "unused",
        "isolationtoken",
        "Explicit global memory.",
        MemoryCategory::Lesson,
    );
    global.scope = MemoryScope::Global;
    global.id = Some("DEC-001".into());
    store.capture_daily_memory(&global).unwrap();

    for repo in repos {
        let mut query = MemoryQuery::new(repo, "isolationtoken");
        let local = store.query_daily_memories(&query).unwrap();
        assert_eq!(local.records.len(), 1);
        assert_eq!(local.records[0].body, repo);
        assert_eq!(local.global_revision, None);
        query.include_global = true;
        let combined = store.query_daily_memories(&query).unwrap();
        assert_eq!(combined.records.len(), 2);
        assert_eq!(combined.global_revision, Some(1));
        assert!(combined
            .records
            .iter()
            .all(|record| record.scope == MemoryScope::Global
                || record.scope == MemoryScope::Repository(repo.into())));
    }
    let mut unknown = MemoryQuery::new("", "isolationtoken");
    unknown.include_global = true;
    assert!(store.query_daily_memories(&unknown).is_err());
}

#[test]
fn ineligible_high_rank_matches_cannot_starve_applicable_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&dir.path().join("metadata.sqlite")).unwrap();
    for index in 0..60 {
        let mut distractor = MemoryCapture::new(
            "repo/eligibility",
            "needle",
            "needle needle needle",
            MemoryCategory::Lesson,
        );
        distractor.id = Some(format!("early-{index:03}"));
        distractor.branch = Some(if index % 2 == 0 { "other" } else { "main" }.into());
        distractor.path = Some(
            if index % 2 == 0 {
                "src/api"
            } else {
                "src/apix"
            }
            .into(),
        );
        store.capture_daily_memory(&distractor).unwrap();
    }
    let mut applicable = MemoryCapture::new(
        "repo/eligibility",
        "Relevant decision",
        "needle applies to this API path.",
        MemoryCategory::Lesson,
    );
    applicable.branch = Some("main".into());
    applicable.path = Some("src/api".into());
    let expected = store.capture_daily_memory(&applicable).unwrap();
    let mut query = MemoryQuery::new("repo/eligibility", "needle");
    query.branch = Some("main".into());
    query.paths = vec!["src/api/mod.rs".into()];
    query.limit = 1;
    let found = store.query_daily_memories(&query).unwrap();
    assert_eq!(found.records.len(), 1);
    assert_eq!(found.records[0].id, expected.id);
    query.branch = None;
    assert!(store
        .query_daily_memories(&query)
        .unwrap()
        .records
        .is_empty());
}
