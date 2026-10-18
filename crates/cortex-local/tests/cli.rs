use cortex_local::{runtime, CaptureRequest, LocalStore};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn command(home: &Path, cwd: &Path, args: &[&str], input: Option<Value>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env_remove("TYPESAFE_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&input).unwrap())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn cli_capture_search_revise_and_graph_share_exact_identity() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::write(root.path().join("cortex.toml"), "[cortex]\nid = 'Cortex'\n").unwrap();
    let first = success(command(
        &home,
        root.path(),
        &["capture", "--input", "-"],
        Some(json!({
            "id":"local-memory", "title":"Archive recovery", "body":"Canonical history survives index rebuild", "authority":"accepted",
            "source":{"kind":"user_instruction","locator":"docs/decision.md"}
        })),
    ));
    assert_eq!(first["record"]["repository"], "Cortex");
    assert_eq!(first["record"]["source"]["locator"], "docs/decision.md");
    assert_eq!(first["record"]["source"]["payload"], Value::Null);
    let found = success(command(
        &home,
        root.path(),
        &["search", "Canonical history"],
        None,
    ));
    assert_eq!(found[0]["record"]["id"], "local-memory");
    assert_eq!(
        success(command(
            &home,
            root.path(),
            &["--repository", "cortex", "search", "Canonical"],
            None
        )),
        json!([])
    );
    let revision = json!({"title":"Archive recovery","body":"Canonical history preserves source hashes","authority":"accepted","links":[],"metadata":{}});
    success(command(
        &home,
        root.path(),
        &["revise", "local-memory", "--expected", "1", "--input", "-"],
        Some(revision.clone()),
    ));
    let stale = command(
        &home,
        root.path(),
        &["revise", "local-memory", "--expected", "1", "--input", "-"],
        Some(revision),
    );
    assert!(!stale.status.success());
    assert_eq!(
        success(command(
            &home,
            root.path(),
            &["get", "local-memory", "--revision", "1"],
            None
        ))["revision"],
        1
    );
    let graph = success(command(
        &home,
        root.path(),
        &["graph", "local-memory"],
        None,
    ));
    assert_eq!(graph["records"][0]["revision"], 2);
    success(command(&home, root.path(), &["reindex"], None));
    assert_eq!(
        success(command(&home, root.path(), &["get", "local-memory"], None))["revision"],
        2
    );
}

#[test]
fn configuration_walk_is_exact_and_stops_at_repository_boundary() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("cortex.toml"),
        "[cortex]\nid='Mixed Case'\n",
    )
    .unwrap();
    let nested = root.path().join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        runtime::resolve_repository(&nested, None).unwrap(),
        "Mixed Case"
    );
    std::fs::create_dir_all(nested.join(".git")).unwrap();
    assert!(runtime::resolve_repository(&nested, None).is_err());
    assert_eq!(
        runtime::resolve_repository(&nested, Some("Explicit")).unwrap(),
        "Explicit"
    );
    std::fs::write(nested.join("cortex.toml"), "[cortex]\nid=42").unwrap();
    assert!(runtime::resolve_repository(&nested, None).is_err());
    std::fs::write(nested.join("cortex.toml"), "X".repeat(65_537)).unwrap();
    assert!(runtime::resolve_repository(&nested, None).is_err());
}

#[test]
fn cli_rejects_cross_scope_capture_and_keeps_classification_offline() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let store = LocalStore::open(home.join("local")).unwrap();
    let record = store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            id: Some("one".into()),
            body: "A reusable lesson".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    let mismatch = command(
        &home,
        root.path(),
        &["--repo", "Cortex", "capture", "--input", "-"],
        Some(json!({"repository":"other","title":"Wrong","body":"Should reject"})),
    );
    assert!(!mismatch.status.success());
    assert_eq!(store.status(Some("other")).unwrap().records, 0);
    let job = success(command(
        &home,
        root.path(),
        &["--repo", "Cortex", "classify", "enqueue", "one"],
        None,
    ));
    let report = success(command(
        &home,
        root.path(),
        &["--repo", "Cortex", "classify", "run"],
        None,
    ));
    assert_eq!(report["attempted"], 0);
    let jobs = success(command(
        &home,
        root.path(),
        &["--repo", "Cortex", "classify", "jobs"],
        None,
    ));
    assert_eq!(jobs["jobs"][0]["key"], job["key"]);
    assert_eq!(jobs["jobs"][0]["attempts"], 0);
    assert_eq!(store.get("Cortex", "one").unwrap().unwrap(), record);
    let doctor = success(command(&home, root.path(), &["doctor"], None));
    assert_eq!(doctor["runtime"], "standalone");
    assert_eq!(doctor["required_services"], json!([]));
    assert_eq!(doctor["classification_configured"], false);
}

#[test]
fn cli_dry_run_does_not_create_destination_and_hook_fails_open() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("not-created");
    let archive = root.path().join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let report = success(command(
        &home,
        root.path(),
        &["import", "archive", archive.to_str().unwrap(), "--dry-run"],
        None,
    ));
    assert_eq!(report["imported"], 0);
    assert!(!home.exists());
    let output = success(command(
        &home,
        root.path(),
        &["hook", "--client", "codex", "--event", "Stop"],
        Some(json!({"cwd":root.path(),"last_assistant_message":"finished"})),
    ));
    assert_eq!(output, json!({}));
    assert!(!home.exists());
}

#[test]
fn installed_shape_mcp_stdio_matches_local_cli_without_server() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home with spaces");
    let store = LocalStore::open(home.join("local")).unwrap();
    store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            id: Some("stdio-record".into()),
            body: "Historical retrieval".into(),
            ..Default::default()
        })
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("--home")
        .arg(&home)
        .args(["--repository", "Cortex", "mcp"])
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cortex_get","arguments":{"id":"stdio-record"}}}),
    ] {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(
        responses[1]["result"]["tools"],
        cortex_local::mcp::descriptors()
    );
    assert_eq!(responses[2]["result"]["isError"], false);
    let record: Value = serde_json::from_str(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["id"], "stdio-record");
    assert_eq!(record["repository"], "Cortex");
    assert_eq!(
        record,
        success(command(
            &home,
            root.path(),
            &["--repo", "Cortex", "get", "stdio-record"],
            None
        ))
    );
}

#[test]
fn offline_install_and_update_keep_binary_assets_and_existing_data_together() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("custom home with spaces");
    let info = success(command(&home, root.path(), &["build-info"], None));
    assert_eq!(info["runtime"], "standalone");
    assert_eq!(info["install_protocol"], 1);
    assert!(!home.exists(), "build-info must not initialize storage");
    let report = success(command(&home, root.path(), &["install"], None));
    assert_eq!(report["changed"], true);
    let installed = Path::new(report["binary"].as_str().unwrap());
    assert!(installed.is_file());
    let stored = success(command(
        &home,
        root.path(),
        &[
            "--repo",
            "Cortex",
            "capture",
            "--body",
            "Preserve original memory",
        ],
        None,
    ));
    let second = success(command(
        &home,
        root.path(),
        &["update", "--binary", env!("CARGO_BIN_EXE_cortex")],
        None,
    ));
    assert_eq!(second["changed"], false);
    assert_eq!(second["files"], report["files"]);
    assert_eq!(second["binary_sha256"], report["binary_sha256"]);
    let output = Command::new(installed)
        .arg("--home")
        .arg(&home)
        .args([
            "--repo",
            "Cortex",
            "get",
            stored["record"]["id"].as_str().unwrap(),
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(success(output), stored["record"]);
}

#[test]
fn invalid_update_candidate_does_not_initialize_or_change_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("not-created");
    let candidate = root.path().join("not-an-executable");
    std::fs::write(&candidate, "invalid binary").unwrap();
    let output = command(
        &home,
        root.path(),
        &["update", "--binary", candidate.to_str().unwrap()],
        None,
    );
    assert!(!output.status.success());
    assert!(!home.exists());
}

#[cfg(windows)]
#[test]
fn rejected_redirected_install_home_does_not_create_store_in_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let alias = temp.path().join("redirected-home");
    assert!(Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success());
    assert!(!command(&alias, temp.path(), &["install"], None)
        .status
        .success());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn update_accepts_candidate_under_legitimate_ancestor_junction() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("release");
    std::fs::create_dir(&source).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_cortex"), source.join("cortex.exe")).unwrap();
    let alias = temp.path().join("alias");
    assert!(Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&source)
        .output()
        .unwrap()
        .status
        .success());
    let candidate = alias.join("cortex.exe");
    let home = temp.path().join("home");
    let report = success(command(
        &home,
        temp.path(),
        &["update", "--binary", candidate.to_str().unwrap()],
        None,
    ));
    assert_eq!(report["changed"], true);
    assert!(home.join("local/index.sqlite3").is_file());
}
