use cortex_local::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn fixture(home: &Path) -> (LocalStore, Value) {
    let store = LocalStore::open(home.join("local")).unwrap();
    store
        .capture(CaptureRequest {
            id: Some("source".into()),
            repository: "Cortex".into(),
            title: "Retain sources".into(),
            body: "Keep historical evidence".into(),
            kind: RecordKind::Lesson,
            ..Default::default()
        })
        .unwrap();
    let prepared = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repository":"Cortex"}),
        None,
    )
    .unwrap();
    let delivery = store
        .get_audit("Cortex", prepared["delivery_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    let AuditPayload::Delivery(payload) = delivery.payload else {
        panic!("delivery")
    };
    let request = json!({"id":"feedback-one","repository":"Cortex",
        "source":{"kind":"test","reference":"fixture:judgment","actor":"declared-user"},
        "evidence":"explicit","judgment":"helpful","query_id":payload.query_id,
        "delivery_id":delivery.id,"reference":payload.references[0],"comment":"<script>literal</script>"});
    (store, request)
}
fn cli(home: &Path, scope: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("--home")
        .arg(home)
        .args(["--repository", scope])
        .args(args)
        .output()
        .unwrap()
}
fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn feedback_cli_mcp_share_immutable_identity_scope_and_pagination() {
    let home = tempfile::tempdir().unwrap();
    let (store, request) = fixture(home.path());
    let input = home.path().join("request.json");
    fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
    assert!(!cli(
        home.path(),
        "cortex",
        &["feedback", "record", "--input", input.to_str().unwrap()]
    )
    .status
    .success());
    let written = success(cli(
        home.path(),
        "Cortex",
        &["feedback", "record", "--input", input.to_str().unwrap()],
    ));
    assert_eq!(written["created"], true);
    assert_eq!(written["entry"]["eligibility"], "eligible");
    let replay = mcp::call(&store, "cortex_feedback_record", request.clone(), None).unwrap();
    assert_eq!(replay["created"], false);
    assert_eq!(replay["entry"], written["entry"]);
    assert_eq!(
        success(cli(
            home.path(),
            "Cortex",
            &["feedback", "get", "feedback-one"]
        )),
        written["entry"]
    );
    assert!(mcp::call(
        &store,
        "cortex_feedback_get",
        json!({"repository":"cortex","id":"feedback-one"}),
        None
    )
    .unwrap()
    .is_null());
    let mut changed = request.clone();
    changed["judgment"] = json!("unhelpful");
    assert!(mcp::call(&store, "cortex_feedback_record", changed, None).is_err());
    let mut second = request;
    second["id"] = json!("feedback-two");
    second["evidence"] = json!("inferred");
    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .arg("--home")
        .arg(home.path())
        .args([
            "--repository",
            "Cortex",
            "feedback",
            "record",
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&second).unwrap())
        .unwrap();
    assert_eq!(
        success(child.wait_with_output().unwrap())["entry"]["eligibility"],
        "inferred"
    );
    let first = success(cli(
        home.path(),
        "Cortex",
        &["feedback", "list", "--limit", "1"],
    ));
    assert_eq!(first["entries"].as_array().unwrap().len(), 1);
    let next = mcp::call(
        &store,
        "cortex_feedback_list",
        json!({"repo":"Cortex","limit":1,"cursor":first["next_cursor"]}),
        None,
    )
    .unwrap();
    assert_eq!(next["entries"].as_array().unwrap().len(), 1);
    assert_ne!(
        first["entries"][0]["record"]["request"]["id"],
        next["entries"][0]["record"]["request"]["id"]
    );
}

#[test]
fn feedback_mcp_rejects_legacy_spoofing_extra_fields_and_bad_types() {
    let home = tempfile::tempdir().unwrap();
    let (store, request) = fixture(home.path());
    for (key, value) in [
        ("evidence", json!("legacy_unknown")),
        ("repository", json!("legacy-feedback-unattributed")),
        ("query_id", Value::Null),
        ("reference", Value::Null),
        ("source", json!({"kind":"test","reference":"x"})),
        ("comment", json!(false)),
        ("legacy", json!({})),
        ("recorded_at", json!("2020-01-01T00:00:00Z")),
    ] {
        let mut invalid = request.clone();
        invalid[key] = value;
        assert!(
            mcp::call(&store, "cortex_feedback_record", invalid, None).is_err(),
            "{key}"
        );
    }
    for args in [
        json!({"repository":"Cortex","limit":0}),
        json!({"repository":"Cortex","limit":101}),
        json!({"repository":"Cortex","limit":true}),
        json!({"repository":"Cortex","cursor":42}),
        json!({"repository":"Cortex","path":"local.sqlite"}),
    ] {
        assert!(mcp::call(&store, "cortex_feedback_list", args, None).is_err());
    }
    let mut alias = request;
    alias.as_object_mut().unwrap().remove("repository");
    alias["scope"] = json!({"repo":"Cortex"});
    assert_eq!(
        mcp::call(&store, "cortex_feedback_record", alias, None).unwrap()["created"],
        true
    );
    let tools = mcp::descriptors();
    let tool = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "cortex_feedback_record")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["evidence"]["enum"],
        json!(["explicit", "inferred"])
    );
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
}

#[test]
fn feedback_cli_import_continues_pages_and_rejects_another_source_cursor() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("legacy.sqlite");
    let connection = rusqlite::Connection::open(&source).unwrap();
    connection.execute_batch("CREATE TABLE pre_thinking_feedback(query_id TEXT PRIMARY KEY,intent TEXT,helpful INTEGER,files_cited TEXT,rating INTEGER,free_text TEXT,implicit_score REAL,recorded_at TEXT);
        INSERT INTO pre_thinking_feedback VALUES ('first','test',1,'[]',5,'first',0.5,'2026-01-01T00:00:00Z');
        INSERT INTO pre_thinking_feedback VALUES ('second','test',0,'[]',1,'second',0.0,'2026-01-01T00:00:00Z');").unwrap();
    drop(connection);
    let before = fs::read(&source).unwrap();
    let first = success(cli(
        home.path(),
        "Cortex",
        &[
            "feedback",
            "import",
            source.to_str().unwrap(),
            "--max-records",
            "1",
        ],
    ));
    assert_eq!(first["imported"], 1);
    assert_eq!(first["complete"], false);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = success(cli(
        home.path(),
        "Cortex",
        &[
            "feedback",
            "import",
            source.to_str().unwrap(),
            "--max-records",
            "1",
            "--cursor",
            cursor,
        ],
    ));
    assert_eq!(second["imported"], 1);
    assert_eq!(second["complete"], true);
    assert!(second["next_cursor"].is_null());
    let another = home.path().join("other.sqlite");
    fs::write(&another, &before).unwrap();
    assert!(!cli(
        home.path(),
        "Cortex",
        &[
            "feedback",
            "import",
            another.to_str().unwrap(),
            "--max-records",
            "1",
            "--cursor",
            cursor
        ]
    )
    .status
    .success());
    assert_eq!(fs::read(&source).unwrap(), before);
    let rows = success(cli(
        home.path(),
        "legacy-feedback-unattributed",
        &["feedback", "list"],
    ));
    assert_eq!(rows["entries"].as_array().unwrap().len(), 2);
    assert!(rows["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["eligibility"] == "legacy_unknown"));
}

#[tokio::test]
async fn feedback_web_requires_auth_preserves_literals_and_rejects_forged_provenance() {
    let home = tempfile::tempdir().unwrap();
    let (store, request) = fixture(home.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = "a".repeat(64);
    let router = web::router(store.clone(), port, token.clone()).unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let unauthorized = client
        .post(format!("{base}/api/feedback/record"))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert!(!unauthorized.status().is_success());
    let post = |path: &str, body: Value| {
        client
            .post(format!("{base}/api/{path}"))
            .header("Origin", &base)
            .bearer_auth(&token)
            .json(&body)
            .send()
    };
    let baseline_args = json!({"repository":"Cortex","text":"Retain","limit":20});
    let baseline = post("search", baseline_args.clone())
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(baseline[0].get("feedback").is_none());
    let mut enabled_args = baseline_args.clone();
    enabled_args["feedback"] = json!(true);
    enabled_args["as_of"] = json!("2030-01-01T00:00:00Z");
    let enabled = post("search", enabled_args.clone())
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        enabled,
        mcp::call(&store, "cortex_search", enabled_args, None).unwrap()
    );
    let mut disabled = baseline_args.clone();
    disabled["feedback"] = json!(false);
    assert_eq!(
        post("search", disabled)
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
        baseline
    );
    for extra in [
        json!({"feedback":1}),
        json!({"feedback":false,"as_of":"2030-01-01T00:00:00Z"}),
        json!({"feedback":true,"as_of":false}),
        json!({"feedback":true,"as_of":null}),
    ] {
        let mut invalid = baseline_args.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(!post("search", invalid).await.unwrap().status().is_success());
    }
    let response = post("feedback/record", request.clone()).await.unwrap();
    assert!(response.status().is_success());
    let value: Value = response.json().await.unwrap();
    assert_eq!(
        value["entry"]["record"]["request"]["comment"],
        "<script>literal</script>"
    );
    let fetched = post(
        "feedback/get",
        json!({"repository":"Cortex","id":"feedback-one"}),
    )
    .await
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(fetched, value["entry"]);
    assert_eq!(
        post(
            "feedback/get",
            json!({"repository":"cortex","id":"feedback-one"})
        )
        .await
        .unwrap()
        .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let mut forged = request;
    forged["evidence"] = json!("legacy_unknown");
    assert!(!post("feedback/record", forged)
        .await
        .unwrap()
        .status()
        .is_success());
    assert!(!post(
        "feedback/list",
        json!({"repository":"Cortex","limit":20,"source":"private.sqlite"})
    )
    .await
    .unwrap()
    .status()
    .is_success());
    let list = post("feedback/list", json!({"repository":"Cortex","limit":20}))
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list["entries"][0], fetched);
    let script = client
        .get(format!("{base}/app.js"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(script.contains("feedbackState.target !== target"));
    assert!(!script.contains("innerHTML") && !script.contains("insertAdjacentHTML"));
    task.abort();
    let _ = task.await;
}

#[test]
fn search_feedback_is_explicit_and_cli_mcp_match_with_fixed_time() {
    let home = tempfile::tempdir().unwrap();
    let (store, _) = fixture(home.path());
    let baseline = success(cli(home.path(), "Cortex", &["search", "Retain"]));
    assert!(baseline[0].get("feedback").is_none());
    for tool in ["cortex_search", "cortex_query"] {
        assert_eq!(
            mcp::call(
                &store,
                tool,
                json!({"repository":"Cortex","query":"Retain","limit":20}),
                None
            )
            .unwrap(),
            baseline
        );
        assert_eq!(
            mcp::call(
                &store,
                tool,
                json!({"repository":"Cortex","query":"Retain","limit":20,"feedback":false}),
                None
            )
            .unwrap(),
            baseline
        );
        let enabled = json!({"repository":"Cortex","query":"Retain","limit":20,"feedback":true,"as_of":"2030-01-01T00:00:00Z"});
        let result = mcp::call(&store, tool, enabled, None).unwrap();
        assert!(result[0]["feedback"].is_object());
        assert_eq!(
            result,
            success(cli(
                home.path(),
                "Cortex",
                &[
                    "search",
                    "Retain",
                    "--feedback",
                    "--as-of",
                    "2030-01-01T00:00:00Z"
                ]
            ))
        );
        for extra in [
            json!({"feedback":1}),
            json!({"feedback":"true"}),
            json!({"feedback":false,"as_of":"2030-01-01T00:00:00Z"}),
            json!({"as_of":"2030-01-01T00:00:00Z"}),
            json!({"feedback":true,"as_of":null}),
        ] {
            let mut invalid = json!({"repository":"Cortex","query":"Retain"});
            invalid
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(mcp::call(&store, tool, invalid, None).is_err());
        }
    }
    assert!(!cli(
        home.path(),
        "Cortex",
        &["search", "Retain", "--as-of", "2030-01-01T00:00:00Z"]
    )
    .status
    .success());
}
