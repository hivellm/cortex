use cortex_local::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn request(repository: &str, id: &str) -> CaptureRequest {
    CaptureRequest {
        repository: repository.into(),
        id: Some(id.into()),
        title: "needle".into(),
        body: "original evidence".into(),
        ..Default::default()
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

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn memory_path(store: &LocalStore, repository: &str, id: &str) -> PathBuf {
    store
        .root()
        .join("records")
        .join(hash(repository))
        .join(hash(id))
        .join("00000000000000000001.md")
}

#[test]
fn tampered_canonical_memory_cannot_replace_working_index() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    let original = store.capture(request("repo", "one")).unwrap().record;
    let path = memory_path(&store, "repo", "one");
    let original_bytes = fs::read(&path).unwrap();
    let tampered = String::from_utf8(original_bytes.clone())
        .unwrap()
        .replace("original evidence", "forged evidence");
    fs::write(&path, tampered).unwrap();

    assert!(matches!(store.reindex(), Err(LocalError::Invalid(_))));
    assert_eq!(store.get("repo", "one").unwrap(), Some(original));
    assert_eq!(store.search(query("repo", "original")).unwrap().len(), 1);
    assert!(store.search(query("repo", "forged")).unwrap().is_empty());

    fs::write(path, original_bytes).unwrap();
    assert_eq!(store.reindex().unwrap().records, 1);
}

#[test]
fn truncated_segment_rolls_back_records_projected_before_error() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    let mut event = request("repo", "event");
    event.kind = RecordKind::Event;
    let original = store.capture(event).unwrap().record;
    let path = fs::read_dir(store.root().join("segments"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let bytes = fs::read(&path).unwrap();
    let mut truncated = bytes.clone();
    truncated.extend_from_slice(b"{\"schema_version\":");
    fs::write(&path, truncated).unwrap();

    assert!(store.reindex().is_err());
    assert_eq!(store.get("repo", "event").unwrap(), Some(original));
    assert_eq!(store.status(Some("repo")).unwrap().events, 1);
    fs::write(path, bytes).unwrap();
    assert_eq!(store.reindex().unwrap().records, 1);
}

#[test]
fn foreign_corpus_cannot_change_scoped_scores_counts_or_graph() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    let mut root = request("Repo", "shared");
    root.links.push(TypedLink {
        target_id: "foreign-only".into(),
        relation: Relation::RelatedTo,
    });
    store.capture(root).unwrap();
    let before = store.search(query("Repo", "needle")).unwrap();
    let counts = serde_json::to_value(store.status(Some("Repo")).unwrap()).unwrap();
    for id in 0..40 {
        store
            .capture(request("repo", &format!("foreign-{id}")))
            .unwrap();
    }
    store.capture(request("repo", "foreign-only")).unwrap();
    store.capture(request("repo", "shared")).unwrap();
    let after = store.search(query("Repo", "needle")).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0].record, before[0].record);
    assert_eq!(after[0].score.to_bits(), before[0].score.to_bits());
    assert_eq!(
        serde_json::to_value(store.status(Some("Repo")).unwrap()).unwrap(),
        counts
    );
    assert!(store.get("Repo", "foreign-only").unwrap().is_none());
    assert!(store.search(query("REPO", "needle")).unwrap().is_empty());
    let graph = store
        .graph(GraphQuery {
            repository: "Repo".into(),
            id: "shared".into(),
            depth: 5,
            limit: 200,
        })
        .unwrap();
    assert_eq!(graph.records.len(), 1);
    assert_eq!(graph.records[0].repository, "Repo");
    assert!(graph.edges.is_empty());
    store.reindex().unwrap();
    assert_eq!(store.status(Some("Repo")).unwrap().records, 1);
}

#[test]
fn caller_paths_become_hashes_not_filesystem_components() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path().join("store")).unwrap();
    let repository = "../../outside:repo";
    let id = "../escape\\CON:payload";
    let original = store.capture(request(repository, id)).unwrap().record;
    let path = memory_path(&store, repository, id);
    assert!(path.is_file());
    assert!(path.starts_with(store.root()));
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
    store.reindex().unwrap();
    assert_eq!(store.get(repository, id).unwrap(), Some(original));
}

#[test]
fn malformed_and_oversized_inputs_publish_nothing() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    let mut cases = vec![request("", "id"), request("repo", "\0id")];
    let mut oversized = request("repo", "large-body");
    oversized.body = "x".repeat(MAX_BODY_BYTES + 1);
    cases.push(oversized);
    let mut metadata = request("repo", "large-metadata");
    metadata.metadata = serde_json::Value::String("x".repeat(MAX_RECORD_BYTES));
    cases.push(metadata);
    let mut invalid_link = request("repo", "bad-link");
    invalid_link.links.push(TypedLink {
        target_id: String::new(),
        relation: Relation::Supports,
    });
    cases.push(invalid_link);
    let mut authoritative_event = request("repo", "bad-event");
    authoritative_event.kind = RecordKind::Event;
    authoritative_event.authority = Authority::Accepted;
    cases.push(authoritative_event);
    for invalid in cases {
        assert!(matches!(
            store.capture(invalid),
            Err(LocalError::Invalid(_))
        ));
        assert_eq!(store.status(None).unwrap().records, 0);
        for name in ["records", "segments", "pending"] {
            assert!(!store.root().join(name).exists(), "published {name}");
        }
    }
    let mut valid = request("repo", "valid-event");
    valid.kind = RecordKind::Event;
    let mut invalid = valid.clone();
    invalid.id = Some("bad-event".into());
    invalid.body = "x".repeat(MAX_BODY_BYTES + 1);
    assert!(store.capture_batch(vec![valid, invalid]).is_err());
    assert!(!store.root().join("segments").exists());
    assert_eq!(store.status(None).unwrap().records, 0);
}

#[test]
fn query_syntax_is_literal_and_invalid_bounds_do_not_broaden_scope() {
    let home = tempfile::tempdir().unwrap();
    let store = LocalStore::open(home.path()).unwrap();
    store.capture(request("repo", "one")).unwrap();
    store.capture(request("foreign", "two")).unwrap();
    for text in [
        "needle OR secret",
        "\" OR *",
        "title:needle",
        "needle NOT secret",
    ] {
        assert!(store.search(query("repo", text)).unwrap().is_empty());
    }
    let mut invalid = query("repo", "needle");
    invalid.limit = MAX_SEARCH_RESULTS + 1;
    assert!(matches!(store.search(invalid), Err(LocalError::Invalid(_))));
    assert!(store.search(query("", "needle")).is_err());
    assert!(store.status(Some("")).is_err());
}

#[cfg(any(unix, windows))]
fn directory_symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(target, link);
    match result {
        Ok(()) => true,
        // Windows may require developer mode or SeCreateSymbolicLinkPrivilege.
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!("symlink test unavailable: Windows privilege 1314");
            false
        }
        Err(error) => panic!("unexpected symlink creation failure: {error}"),
    }
}

#[cfg(any(unix, windows))]
#[test]
fn symlinked_ancestor_resolves_before_store_safety_checks() {
    let home = tempfile::tempdir().unwrap();
    let actual = tempfile::tempdir().unwrap();
    let alias = home.path().join("home-alias");
    if !directory_symlink(actual.path(), &alias) {
        return;
    }
    let store = LocalStore::open(alias.join("store")).unwrap();
    assert_eq!(
        store.root(),
        fs::canonicalize(actual.path().join("store")).unwrap()
    );
    store.capture(request("repo", "ancestor-alias")).unwrap();
    assert!(store.get("repo", "ancestor-alias").unwrap().is_some());
    assert!(LocalStore::inspect(alias.join("store")).is_ok());
    assert!(LocalStore::open_for_hook(alias.join("store"), std::time::Duration::ZERO).is_ok());
    assert!(LocalStore::inspect(&alias).is_err());
    assert!(LocalStore::open_for_hook(&alias, std::time::Duration::ZERO).is_err());
    assert!(matches!(
        LocalStore::open(&alias),
        Err(LocalError::Invalid(_))
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn symlinked_store_and_canonical_subtree_are_rejected() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let alias = home.path().join("alias");
    if !directory_symlink(outside.path(), &alias) {
        return;
    }
    assert!(matches!(
        LocalStore::open(&alias),
        Err(LocalError::Invalid(_))
    ));
    let store = LocalStore::open(home.path().join("store")).unwrap();
    assert!(directory_symlink(
        outside.path(),
        &store.root().join("records")
    ));
    assert!(matches!(store.reindex(), Err(LocalError::Invalid(_))));
    assert_eq!(store.status(None).unwrap().records, 0);
    assert!(store.capture(request("repo", "escape")).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[cfg(windows)]
fn directory_junction(target: &Path, link: &Path) {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:CORTEX_TEST_JUNCTION_LINK -Target $env:CORTEX_TEST_JUNCTION_TARGET | Out-Null",
        ])
        .env("CORTEX_TEST_JUNCTION_LINK", link)
        .env("CORTEX_TEST_JUNCTION_TARGET", target)
        .output()
        .expect("start Windows PowerShell to create isolated test junction");
    assert!(
        output.status.success(),
        "junction creation failed: status={}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    use std::os::windows::fs::MetadataExt;
    assert_ne!(
        fs::symlink_metadata(link).unwrap().file_attributes() & 0x400,
        0,
        "fixture must actually be a Windows reparse point"
    );
}

#[cfg(windows)]
#[test]
fn windows_junction_ancestor_resolves_to_canonical_store() {
    let home = tempfile::tempdir().unwrap();
    let actual = tempfile::tempdir().unwrap();
    let alias = home.path().join("home-junction");
    directory_junction(actual.path(), &alias);
    let store = LocalStore::open(alias.join("store")).unwrap();
    assert_eq!(
        store.root(),
        fs::canonicalize(actual.path().join("store")).unwrap()
    );
    store.capture(request("repo", "junction-ancestor")).unwrap();
    assert!(store.get("repo", "junction-ancestor").unwrap().is_some());
    assert!(LocalStore::inspect(alias.join("store")).is_ok());
    assert!(LocalStore::open_for_hook(alias.join("store"), std::time::Duration::ZERO).is_ok());
    assert!(LocalStore::inspect(&alias).is_err());
    assert!(LocalStore::open_for_hook(&alias, std::time::Duration::ZERO).is_err());
    assert!(matches!(
        LocalStore::open(&alias),
        Err(LocalError::Invalid(_))
    ));
}

#[cfg(windows)]
#[test]
fn windows_junctions_cannot_redirect_root_or_canonical_publication() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.txt");
    fs::write(&sentinel, b"untouched external target").unwrap();
    let alias = home.path().join("root-junction");
    directory_junction(outside.path(), &alias);
    assert!(matches!(
        LocalStore::open(&alias),
        Err(LocalError::Invalid(_))
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);

    for subtree in ["records", "segments"] {
        let store = LocalStore::open(home.path().join(format!("store-{subtree}"))).unwrap();
        directory_junction(outside.path(), &store.root().join(subtree));
        assert!(
            matches!(store.reindex(), Err(LocalError::Invalid(_))),
            "{subtree} junction accepted during rebuild"
        );
        let mut input = request("repo", "junction-escape");
        if subtree == "segments" {
            input.kind = RecordKind::Event;
        }
        assert!(
            matches!(store.capture(input), Err(LocalError::Invalid(_))),
            "{subtree} junction accepted during publication"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched external target");
    }
}
