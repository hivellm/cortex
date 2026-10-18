use cortex_local::*;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(path: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !path.exists() {
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&root.join("records"), root, &mut out);
    walk(&root.join("segments"), root, &mut out);
    out
}

fn populate(root: &Path) -> LocalStore {
    let store = LocalStore::open(root).unwrap();
    for (id, kind) in [("memory", RecordKind::Lesson), ("event", RecordKind::Event)] {
        store
            .capture(CaptureRequest {
                id: Some(id.into()),
                repository: "Cortex".into(),
                kind,
                body: "recover searchable originals".into(),
                ..Default::default()
            })
            .unwrap();
    }
    store
}

#[test]
fn explicit_repair_retains_corrupt_database_sidecars_pending_and_canonical_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = populate(root.path());
    let originals = snapshot(root.path());
    let names = [
        "index.sqlite3",
        "index.sqlite3-wal",
        "index.sqlite3-shm",
        "index.sqlite3-journal",
        "pending",
    ];
    for name in names {
        fs::write(
            root.path().join(name),
            format!("original corrupt bytes {name}"),
        )
        .unwrap();
    }
    assert!(LocalStore::open_for_hook(root.path(), Duration::ZERO).is_err());
    let report = LocalStore::repair(root.path()).unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(snapshot(root.path()), originals);
    let backup = fs::read_dir(root.path().join("index-backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    for name in names {
        assert_eq!(
            fs::read(backup.join("originals").join(name)).unwrap(),
            format!("original corrupt bytes {name}").as_bytes()
        );
    }
    assert!(!root.path().join("pending").exists());
    assert_eq!(store.recent("Cortex", 5, false).unwrap().len(), 2);
    assert!(LocalStore::open_for_hook(root.path(), Duration::ZERO).is_ok());
}

#[test]
fn invalid_canonical_source_aborts_before_touching_original_index() {
    let root = tempfile::tempdir().unwrap();
    populate(root.path());
    fs::write(
        root.path().join("segments").join("invalid.jsonl"),
        b"invalid canonical bytes\n",
    )
    .unwrap();
    let originals = snapshot(root.path());
    let index = fs::read(root.path().join("index.sqlite3")).unwrap();
    fs::write(root.path().join("pending"), b"original marker").unwrap();
    assert!(LocalStore::repair(root.path()).is_err());
    assert_eq!(fs::read(root.path().join("index.sqlite3")).unwrap(), index);
    assert_eq!(
        fs::read(root.path().join("pending")).unwrap(),
        b"original marker"
    );
    assert_eq!(snapshot(root.path()), originals);
}

#[test]
fn repair_missing_index_recovers_existing_canonical_history_without_deleting_old_copy() {
    let root = tempfile::tempdir().unwrap();
    populate(root.path());
    fs::rename(
        root.path().join("index.sqlite3"),
        root.path().join("previous-index.sqlite3"),
    )
    .unwrap();
    let previous = fs::read(root.path().join("previous-index.sqlite3")).unwrap();
    assert_eq!(LocalStore::repair(root.path()).unwrap().records, 2);
    assert_eq!(
        fs::read(root.path().join("previous-index.sqlite3")).unwrap(),
        previous
    );
    assert_eq!(
        LocalStore::open(root.path())
            .unwrap()
            .status(Some("Cortex"))
            .unwrap()
            .records,
        2
    );
}

#[test]
fn inspect_constructor_and_rejected_writes_preserve_every_file_byte() {
    fn all_files(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                out.extend(all_files(&entry.path()));
            } else {
                out.insert(entry.path(), fs::read(entry.path()).unwrap());
            }
        }
        out
    }
    let root = tempfile::tempdir().unwrap();
    populate(root.path());
    let before = all_files(root.path());
    let inspection = LocalStore::inspect(root.path()).unwrap();
    assert!(inspection
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            ..Default::default()
        })
        .is_err());
    assert!(inspection.reindex().is_err());
    assert_eq!(all_files(root.path()), before);
    fs::write(root.path().join("pending"), b"unrecovered").unwrap();
    let pending = all_files(root.path());
    assert!(LocalStore::inspect(root.path()).is_err());
    assert_eq!(all_files(root.path()), pending);
    assert!(LocalStore::inspect(root.path().join("missing")).is_err());
    assert!(!root.path().join("missing").exists());
}
