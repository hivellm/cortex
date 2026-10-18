use cortex_local::{mcp, CaptureRequest, LocalStore};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_string_lossy().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    for name in ["records", "audit", "feedback"] {
        if root.join(name).exists() {
            walk(root, &root.join(name), &mut out);
        }
    }
    out
}

#[test]
fn schema_two_requires_explicit_upgrade_and_keeps_canonical_sources() {
    let temp = tempfile::tempdir().unwrap();
    let store = LocalStore::open(temp.path()).unwrap();
    store
        .capture(CaptureRequest {
            id: Some("original".into()),
            repository: "Cortex".into(),
            body: "Original source survives schema changes".into(),
            ..Default::default()
        })
        .unwrap();
    let delivery = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repository":"Cortex","query":"Original"}),
        None,
    )
    .unwrap();
    let before = snapshot(temp.path());
    let baseline = store.get("Cortex", "original").unwrap();
    drop(store);
    {
        let connection = rusqlite::Connection::open(temp.path().join("index.sqlite3")).unwrap();
        connection
            .execute_batch("DROP TABLE feedback_records; PRAGMA user_version=2;")
            .unwrap();
    }
    assert!(LocalStore::open_for_hook(temp.path(), Duration::from_millis(20)).is_err());
    assert_eq!(snapshot(temp.path()), before);
    let upgraded = LocalStore::open(temp.path()).unwrap();
    assert_eq!(upgraded.get("Cortex", "original").unwrap(), baseline);
    assert!(upgraded
        .get_audit("Cortex", delivery["delivery_id"].as_str().unwrap())
        .unwrap()
        .is_some());
    assert_eq!(snapshot(temp.path()), before);
    assert!(LocalStore::open_for_hook(temp.path(), Duration::from_millis(20)).is_ok());
    upgraded.reindex().unwrap();
    assert_eq!(snapshot(temp.path()), before);
}
