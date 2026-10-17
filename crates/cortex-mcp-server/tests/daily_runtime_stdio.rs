//! Daily production stdio advertises and dispatches one coherent local registry.

use std::{process::Stdio, sync::Arc, time::Duration};

use cortex_api::{
    build_router_with_auth_and_cfg, storage::api_keys::ApiKeyStore, MemoryGraphLane,
    MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryService,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
};

const DAILY_TOOLS: [&str; 12] = [
    "cortex_acl_grant",
    "cortex_acl_whoami",
    "cortex_active_work",
    "cortex_audit",
    "cortex_capabilities",
    "cortex_capture_memory",
    "cortex_feedback_record",
    "cortex_feedback_signals",
    "cortex_memory_read",
    "cortex_pre_thinking",
    "cortex_query",
    "cortex_status",
];

async fn rpc(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = serde_json::to_vec(&json!({
            "jsonrpc":"2.0", "id":id, "method":method, "params":params
        }))
        .unwrap();
        bytes.push(b'\n');
        stdin.write_all(&bytes).await.unwrap();
        stdin.flush().await.unwrap();
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).await.unwrap() > 0);
        let response: Value = serde_json::from_str(&line).expect("stdout is JSON-RPC only");
        assert_eq!(response["id"], id);
        response
    })
    .await
    .expect("daily MCP stdio deadline")
}

fn payload(response: &Value) -> Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn names(tools: &Value) -> Vec<&str> {
    let mut names: Vec<_> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn daily_stdio_lists_only_supported_tools_and_keeps_authenticated_capture_read() {
    let directory = tempfile::tempdir().unwrap();
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let key = keys.issue("memory", "daily-stdio").unwrap();
    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        Arc::new(MemoryKeywordLane::new().with_fail()),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ));
    let mut cfg = cortex_config::Config::default();
    cfg.runtime.profile = cortex_config::RuntimeProfile::Daily;
    cfg.ingestion.metadata_db = Some(
        directory
            .path()
            .join("metadata.sqlite")
            .to_string_lossy()
            .into_owned(),
    );
    let requests = Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
    let observed = requests.clone();
    let authorization = format!("Bearer {}", key.cleartext);
    let router = build_router_with_auth_and_cfg(Arc::new(service), None, Some(keys), Arc::new(cfg))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let observed = observed.clone();
                let valid = request
                    .headers()
                    .get("authorization")
                    .and_then(|header| header.to_str().ok())
                    == Some(authorization.as_str());
                async move {
                    observed
                        .lock()
                        .unwrap()
                        .push((request.uri().path().to_owned(), valid));
                    next.run(request).await
                }
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let api = tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
    let config = directory.path().join("runtime.toml");
    std::fs::write(
        &config,
        format!(
            "[runtime]\nprofile='full'\n[dashboard]\napi_url='{api_url}'\napi_token='{}'\n",
            key.cleartext
        ),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex-mcp-server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("CORTEX_") {
            command.env_remove(name);
        }
    }
    let mut child = command
        .arg("serve")
        .env("CORTEX_CONFIG_FILE", &config)
        // Prove the binary uses the final env-overlaid profile, not TOML alone.
        .env("CORTEX_RUNTIME_PROFILE", "daily")
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let initialized = rpc(
        &mut stdin,
        &mut stdout,
        1,
        "initialize",
        json!({"clientInfo":{"name":"daily-stdio-test","version":"1"}}),
    )
    .await;
    assert!(initialized.get("error").is_none(), "{initialized}");
    let listed = rpc(&mut stdin, &mut stdout, 2, "tools/list", json!({})).await;
    assert_eq!(names(&listed["result"]["tools"]), DAILY_TOOLS);
    let discovered = payload(
        &rpc(
            &mut stdin,
            &mut stdout,
            3,
            "tools/call",
            json!({"name":"cortex_capabilities","arguments":{}}),
        )
        .await,
    );
    assert_eq!(names(&discovered["tools"]), DAILY_TOOLS);
    assert_eq!(discovered["total"], DAILY_TOOLS.len());
    // An omitted known tool fails at registry lookup, before tool argument
    // parsing or HTTP. Existing JSON-RPC method-not-found behavior is retained.
    let disabled = rpc(
        &mut stdin,
        &mut stdout,
        4,
        "tools/call",
        json!({"name":"cortex_vector_search","arguments":{"invalid":"payload"}}),
    )
    .await;
    assert_eq!(disabled["error"]["code"], -32601);
    assert!(requests.lock().unwrap().is_empty());
    let captured = payload(
        &rpc(
            &mut stdin,
            &mut stdout,
            5,
            "tools/call",
            json!({"name":"cortex_capture_memory","arguments":{
                "repo":"Daily/Exact", "kind":"decision", "memory_id":"daily-stdio-decision",
                "body":"dailyregistrytoken: preserve exact local decisions."
            }}),
        )
        .await,
    );
    assert_eq!(captured["durability"], "local_committed");
    let read = payload(
        &rpc(
            &mut stdin,
            &mut stdout,
            6,
            "tools/call",
            json!({"name":"cortex_memory_read","arguments":{
                "repo":"Daily/Exact", "query":"dailyregistrytoken"
            }}),
        )
        .await,
    );
    assert_eq!(read["memories"][0]["id"], "daily-stdio-decision");
    assert!(read["context"]
        .as_str()
        .unwrap()
        .contains("preserve exact local decisions"));
    assert_eq!(
        *requests.lock().unwrap(),
        [
            ("/v1/ingest".to_owned(), true),
            ("/v1/memory/query".to_owned(), true)
        ]
    );
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    api.abort();
    let _ = api.await;
}
