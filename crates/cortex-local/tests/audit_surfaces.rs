use cortex_local::*;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn setup(home: &Path) -> (LocalStore, String) {
    let store = LocalStore::open(home.join("local")).unwrap();
    store
        .capture(CaptureRequest {
            id: Some("rule".into()),
            repository: "Cortex".into(),
            body: "Retain original evidence".into(),
            ..Default::default()
        })
        .unwrap();
    let response = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repository":"Cortex"}),
        None,
    )
    .unwrap();
    (store, response["delivery_id"].as_str().unwrap().into())
}
fn run(home: &Path, scope: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("--home")
        .arg(home)
        .args(["--repository", scope])
        .args(args)
        .output()
        .unwrap()
}
fn successful(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for item in fs::read_dir(at).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    for directory in ["audit", "records"] {
        walk(root, &root.join(directory), &mut files);
    }
    files
}

#[test]
fn cli_inspects_and_archives_without_http_or_external_services() {
    let home = tempfile::tempdir().unwrap();
    let (store, id) = setup(home.path());
    let before = snapshot(store.root());
    let list = successful(run(
        home.path(),
        "Cortex",
        &["audit", "list", "--kind", "delivery", "--limit", "1"],
    ));
    assert_eq!(list["entries"][0]["record"]["id"], id);
    assert_eq!(list["next_cursor"], Value::Null);
    let fetched = successful(run(home.path(), "Cortex", &["audit", "get", &id]));
    assert_eq!(fetched["evidence_label"], "prepared_delivery");
    let metrics = successful(run(home.path(), "Cortex", &["audit", "metrics"]));
    assert_eq!(metrics["deliveries"]["prepared"], 1);
    assert_eq!(metrics["citation_delivery_rate"], 0.0);
    let preview = successful(run(
        home.path(),
        "Cortex",
        &["audit", "retain", "--cutoff", "9999-01-01T00:00:00Z"],
    ));
    assert_eq!(preview["delivery_ids"], json!([id]));
    assert_eq!(preview["control"], Value::Null);
    assert_eq!(snapshot(store.root()), before);
    assert_eq!(
        successful(run(home.path(), "cortex", &["audit", "list"]))["entries"],
        json!([])
    );
    assert!(!run(home.path(), "cortex", &["audit", "get", &id])
        .status
        .success());
    assert!(!run(
        home.path(),
        "Cortex",
        &["audit", "metrics", "--limit", "10001"]
    )
    .status
    .success());
    assert!(!run(
        home.path(),
        "Cortex",
        &["audit", "list", "--kind", "invented"]
    )
    .status
    .success());
    let applied = successful(run(
        home.path(),
        "Cortex",
        &[
            "audit",
            "retain",
            "--cutoff",
            "9999-01-01T00:00:00Z",
            "--apply",
        ],
    ));
    assert!(applied["control"].is_object());
    assert_eq!(
        successful(run(
            home.path(),
            "Cortex",
            &["audit", "list", "--kind", "delivery"]
        ))["entries"],
        json!([])
    );
    let all = successful(run(
        home.path(),
        "Cortex",
        &["audit", "list", "--include-archived", "--kind", "delivery"],
    ));
    assert_eq!(all["entries"][0]["archived"], true);
    assert_eq!(
        successful(run(home.path(), "Cortex", &["audit", "get", &id])),
        fetched
    );
    for (path, bytes) in before {
        assert_eq!(fs::read(store.root().join(path)).unwrap(), bytes);
    }
}

#[test]
fn mcp_audit_contract_is_scoped_strict_and_preview_is_default() {
    let home = tempfile::tempdir().unwrap();
    let (store, id) = setup(home.path());
    let descriptors = mcp::descriptors();
    for (name, max) in [
        ("cortex_audit_list", 100),
        ("cortex_audit_metrics", 10_000),
        ("cortex_audit_retain", 100),
    ] {
        let descriptor = descriptors
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap();
        assert_eq!(
            descriptor["inputSchema"]["properties"]["limit"]["maximum"],
            max
        );
    }
    let list = mcp::call(
        &store,
        "cortex_audit_list",
        json!({"repo":"Cortex","kind":"delivery"}),
        None,
    )
    .unwrap();
    assert_eq!(list["entries"][0]["record"]["id"], id);
    assert_eq!(
        mcp::call(
            &store,
            "cortex_audit_get",
            json!({"scope":{"repo":"Cortex"},"id":id}),
            None
        )
        .unwrap()["id"],
        id
    );
    assert_eq!(
        mcp::call(
            &store,
            "cortex_audit_metrics",
            json!({"repository":"Cortex","limit":10000}),
            None
        )
        .unwrap()["deliveries"]["prepared"],
        1
    );
    let before = snapshot(store.root());
    let preview = mcp::call(
        &store,
        "cortex_audit_retain",
        json!({"repo":"Cortex","cutoff":"9999-01-01T00:00:00Z"}),
        None,
    )
    .unwrap();
    assert_eq!(preview["control"], Value::Null);
    assert_eq!(preview["delivery_ids"], json!([id]));
    assert_eq!(snapshot(store.root()), before);
    for (name, args) in [
        (
            "cortex_audit_list",
            json!({"repo":"Cortex","include_archived":"true"}),
        ),
        (
            "cortex_audit_list",
            json!({"repo":"Cortex","kind":"lesson"}),
        ),
        (
            "cortex_audit_list",
            json!({"repo":"Cortex","cursor":"foreign"}),
        ),
        (
            "cortex_audit_metrics",
            json!({"repo":"Cortex","limit":10001}),
        ),
        (
            "cortex_audit_metrics",
            json!({"repo":"Cortex","include_archived":1}),
        ),
        (
            "cortex_audit_retain",
            json!({"repo":"Cortex","cutoff":"9999-01-01T00:00:00Z","apply":"true"}),
        ),
        ("cortex_audit_retain", json!({"repo":"Cortex","apply":true})),
        (
            "cortex_audit_get",
            json!({"repo":"Cortex","id":id,"unknown":true}),
        ),
    ] {
        assert!(mcp::call(&store, name, args, None).is_err(), "{name}");
    }
    assert_eq!(
        mcp::call(&store, "cortex_audit_list", json!({"repo":"cortex"}), None).unwrap()["entries"],
        json!([])
    );
    let wire = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cortex_audit_metrics","arguments":{"repository":"Cortex"}}}).to_string()+"\n";
    let mut output = Vec::new();
    mcp::run_io(&store, None, wire.as_bytes(), &mut output).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["result"]["isError"], false);
    let applied = mcp::call(
        &store,
        "cortex_audit_retain",
        json!({"repo":"Cortex","cutoff":"9999-01-01T00:00:00Z","apply":true}),
        None,
    )
    .unwrap();
    assert!(applied["control"].is_object());
    for (path, bytes) in before {
        assert_eq!(fs::read(store.root().join(path)).unwrap(), bytes);
    }
}
