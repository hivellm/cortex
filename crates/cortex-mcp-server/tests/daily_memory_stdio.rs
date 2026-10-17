//! Production binary/stdin/stdout contract, not proof of an installed model client's hooks.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::extract::Request;
use axum::middleware::{from_fn, Next};
use cortex_api::storage::api_keys::ApiKeyStore;
use cortex_api::{
    build_router_with_auth_and_cfg, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, PrincipalStore, QueryService, RoleBinding,
};
use cortex_storage::daily_memory::{MemoryDeliveryStage, MemoryScope, OriginAttribution};
use cortex_storage::MetadataStore;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const REPO: &str = "Exact/Repo";
const MEMORY_ID: &str = "STDIO-DECISION";
const BODY: &str = "stdiorecall: preserve committed decisions across process restarts.";
const WRONG_TOKEN: &str = "cortex_dash_invalid_stdio_fixture_credential";
const IO_DEADLINE: Duration = Duration::from_secs(15);

struct RunningApi {
    url: String,
    requests: Arc<Mutex<Vec<(String, bool)>>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl RunningApi {
    async fn start(database: &Path, keys: Arc<ApiKeyStore>, key_id: &str, token: &str) -> Self {
        let mut principals = PrincipalStore::deny_by_default();
        principals.upsert_binding(
            "reader",
            RoleBinding {
                clearance_level: 1,
                compartments: vec![],
            },
        );
        let mut service = QueryService::with_memory_defaults(Orchestrator::new(
            Arc::new(MemoryVectorLane::new().with_fail()),
            Arc::new(MemoryKeywordLane::new().with_fail()),
            Arc::new(MemoryGraphLane::new().with_fail()),
        ));
        service.principal_store = Some(Arc::new(RwLock::new(principals)));
        service.acl.allow(key_id, REPO);
        let mut cfg = cortex_config::Config::default();
        cfg.ingestion.metadata_db = Some(database.to_string_lossy().into_owned());
        cfg.ingestion.ingestion_url = Some("http://127.0.0.1:1".into());
        cfg.access_control.enabled = true;
        cfg.access_control.deny_on_missing_principal = true;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let authorization = format!("Bearer {token}");
        let router =
            build_router_with_auth_and_cfg(Arc::new(service), None, Some(keys), Arc::new(cfg))
                .layer(from_fn(move |request: Request, next: Next| {
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
                }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (shutdown, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = receiver.await;
                })
                .await
                .unwrap();
        });
        Self {
            url,
            requests,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        let mut task = self.task.take().unwrap();
        match tokio::time::timeout(IO_DEADLINE, &mut task).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                task.abort();
                panic!("API did not stop before the restart deadline");
            }
        }
    }
}

impl Drop for RunningApi {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct StdioClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: JoinHandle<Vec<u8>>,
    transcript: String,
    next_id: u64,
}

impl StdioClient {
    fn start(cwd: &Path, config: &Path, env_token: Option<&str>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex-mcp-server"));
        command
            .arg("serve")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Keep platform process essentials, but isolate all product configuration.
        for (name, _) in std::env::vars_os() {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("CORTEX_") || upper.starts_with("TYPESAFE_") || upper == "RUST_LOG"
            {
                command.env_remove(name);
            }
        }
        command
            .env("CORTEX_CONFIG_FILE", config)
            .env("TYPESAFE_API_KEY", "")
            .env("RUST_LOG", "info");
        if let Some(token) = env_token {
            command.env("CORTEX_API_TOKEN", token);
        }
        let mut child = command.spawn().expect("spawn production MCP binary");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stderr_pipe = child.stderr.take().unwrap();
        let stderr = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr_pipe.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        Self {
            child,
            stdin,
            stdout,
            stderr,
            transcript: String::new(),
            next_id: 0,
        }
    }

    async fn send(&mut self, frame: Value) {
        let mut bytes = serde_json::to_vec(&frame).unwrap();
        bytes.push(b'\n');
        let stdin = self.stdin.as_mut().unwrap();
        tokio::time::timeout(IO_DEADLINE, async {
            stdin.write_all(&bytes).await.unwrap();
            stdin.flush().await.unwrap();
        })
        .await
        .expect("MCP stdin write deadline");
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await;
        let mut line = String::new();
        let bytes = tokio::time::timeout(IO_DEADLINE, self.stdout.read_line(&mut line))
            .await
            .expect("MCP stdout response deadline")
            .expect("read MCP stdout");
        assert!(bytes > 0, "MCP closed stdout without a response");
        let response: Value =
            serde_json::from_str(&line).expect("stdout must contain only JSON-RPC");
        self.transcript.push_str(&line);
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(
            response["id"], id,
            "notification emitted an unexpected response"
        );
        assert!(response.get("error").is_none(), "RPC failed: {response}");
        response["result"].clone()
    }

    async fn initialize(&mut self, name: &str) {
        let response = self
            .rpc(
                "initialize",
                json!({"protocolVersion":"2024-11-05", "capabilities":{},
                    "clientInfo":{"name":name,"version":"stdio-fixture"}}),
            )
            .await;
        assert_eq!(response["serverInfo"]["name"], "cortex-mcp-server");
        self.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
            .await;
        let tools = self.rpc("tools/list", json!({})).await;
        for name in [
            "cortex_capture_memory",
            "cortex_memory_read",
            "cortex_pre_thinking",
        ] {
            assert!(tools["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name));
        }
    }

    async fn tool(&mut self, name: &str, arguments: Value, expected_error: bool) -> Value {
        let result = self
            .rpc("tools/call", json!({"name":name,"arguments":arguments}))
            .await;
        assert_eq!(result["isError"], expected_error, "{name}: {result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    async fn stop(mut self, secrets: &[&str]) {
        drop(self.stdin.take());
        let status = tokio::time::timeout(IO_DEADLINE, self.child.wait())
            .await
            .expect("MCP did not exit after stdin EOF")
            .unwrap();
        assert!(status.success(), "MCP child exited unsuccessfully");
        let mut extra = String::new();
        self.stdout.read_to_string(&mut extra).await.unwrap();
        assert!(
            extra.is_empty(),
            "unexpected trailing stdout after final response"
        );
        let stderr = tokio::time::timeout(IO_DEADLINE, self.stderr)
            .await
            .expect("MCP stderr did not close")
            .unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("cortex-mcp-server starting on stdio"));
        for secret in secrets {
            assert!(
                !self.transcript.contains(secret),
                "credential leaked to stdout"
            );
            assert!(!stderr.contains(secret), "credential leaked to stderr");
        }
    }
}

fn config(path: &Path, url: &str, token: &str) {
    std::fs::write(
        path,
        format!("[dashboard]\napi_url = {url:?}\napi_token = {token:?}\n"),
    )
    .unwrap();
}

fn read_args() -> Value {
    json!({"repo":REPO,"query":"stdiorecall"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_stdio_preserves_authenticated_memory_provenance_and_receipts_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("metadata.sqlite");
    let config_path = directory.path().join("mcp-config.toml");
    let repo = directory.path().join("repository");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(repo.join("cortex.toml"), "[cortex]\nid = 'Exact/Repo'\n").unwrap();
    let keys = Arc::new(ApiKeyStore::open(directory.path().join("keys.sqlite")).unwrap());
    let key = keys
        .issue_with_role("memory", "stdio-fixture", Some("reader"))
        .unwrap();
    let secrets = [key.cleartext.as_str(), WRONG_TOKEN];
    let api = RunningApi::start(&database, keys.clone(), &key.id, &key.cleartext).await;
    config(&config_path, &api.url, &key.cleartext);
    let mut first = StdioClient::start(&repo, &config_path, None);
    first.initialize("observed-stdio-client").await;
    let capture = first
        .tool(
            "cortex_capture_memory",
            json!({
                "kind":"decision", "repo":REPO, "memory_id":MEMORY_ID, "body":BODY,
                "agent":"forged-admin", "caller":"admin", "attribution":"trusted_transport"
            }),
            false,
        )
        .await;
    assert_eq!(capture["durability"], "local_committed");
    assert_eq!(capture["index_complete"], false);
    let initial = first.tool("cortex_memory_read", read_args(), false).await;
    let initial_delivery = initial["delivery_id"].as_str().unwrap().to_owned();
    assert_eq!(initial["memories"][0]["id"], MEMORY_ID);
    let scope = MemoryScope::Repository(REPO.to_owned());
    {
        let store = MetadataStore::open(&database).unwrap();
        let record = store.daily_memory(&scope, MEMORY_ID).unwrap().unwrap();
        assert_eq!(
            record.origin.agent.as_deref(),
            Some("observed-stdio-client")
        );
        assert_eq!(record.origin.attribution, OriginAttribution::ClientReported);
        assert!(record.origin.session_id.is_none());
        assert!(record.origin.turn_id.is_none());
    }
    // A reported client name/argument cannot turn the reader credential into an admin.
    let denied_global = first
        .tool(
            "cortex_memory_read",
            json!({
                "repo":REPO,"query":"stdiorecall","include_global":true
            }),
            true,
        )
        .await;
    assert_eq!(denied_global["reason"], "scope_forbidden");
    assert!(api.requests.lock().unwrap().iter().all(|(_, valid)| *valid));
    first.stop(&secrets).await;
    api.stop().await;

    let restarted = RunningApi::start(&database, keys, &key.id, &key.cleartext).await;
    // Environment token must override the deliberately wrong token in the config file.
    config(&config_path, &restarted.url, WRONG_TOKEN);
    let mut second = StdioClient::start(&repo, &config_path, Some(&key.cleartext));
    second.initialize("fresh-stdio-client").await;
    let read = second.tool("cortex_memory_read", read_args(), false).await;
    assert_eq!(read["context"], initial["context"]);
    assert_ne!(read["delivery_id"], initial_delivery);
    assert!(read["context"].as_str().unwrap().len() <= 1536);
    let prethinking = second
        .tool(
            "cortex_pre_thinking",
            json!({
                "user_prompt":"Implement stdiorecall", "cwd":repo, "budget_ms":600
            }),
            false,
        )
        .await;
    assert!(
        prethinking["bundle"].as_str().unwrap().contains(MEMORY_ID),
        "{prethinking}"
    );
    assert!(prethinking["bundle"].as_str().unwrap().contains(BODY));
    let prepared_id = prethinking["memory_delivery_id"].as_str().unwrap();
    let empty = second
        .tool(
            "cortex_pre_thinking",
            json!({"user_prompt":"unmatchednoncexyz", "cwd":repo, "budget_ms":600}),
            true,
        )
        .await;
    assert_eq!(empty["details"]["memory_outcome"], "empty");
    assert_eq!(empty["details"]["memory_revision"], read["revision"]);
    assert!(empty["details"]["memory_cache_hit"].is_boolean());
    let empty_delivery = empty["details"]["memory_delivery_id"].as_str().unwrap();
    {
        let store = MetadataStore::open(&database).unwrap();
        let receipt = store
            .daily_memory_delivery(REPO, empty_delivery)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.stage, MemoryDeliveryStage::Prepared);
        assert!(receipt.request.memories.is_empty());
        assert_eq!(receipt.request.context_bytes, 0);
    }
    assert!(!empty.to_string().contains("cortex: timeout"));
    let feedback = second
        .tool(
            "cortex_feedback_record",
            json!({
                "query_id":initial_delivery, "helpful":true, "rating":5,
                "free_text":"Observed exact context in the subprocess tool result."
            }),
            false,
        )
        .await;
    assert_eq!(feedback["query_id"], initial_delivery);
    {
        let store = MetadataStore::open(&database).unwrap();
        let receipt = store
            .daily_memory_delivery(REPO, prepared_id)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.stage, MemoryDeliveryStage::Prepared);
        assert_eq!(receipt.request.memories.len(), 1);
        assert_eq!(receipt.request.memories[0].id, MEMORY_ID);
        assert_eq!(receipt.request.memories[0].version, 1);
        assert!(receipt.request.context_bytes <= 1536);
        let feedback = store
            .pre_thinking_feedback(&initial_delivery)
            .unwrap()
            .unwrap();
        assert!(feedback.2);
        assert_eq!(feedback.4, Some(5));
    }
    let requests = restarted.requests.lock().unwrap().clone();
    assert!(requests.iter().all(|(_, valid)| *valid));
    for path in ["/v1/memory/query", "/v1/pre-thinking/feedback"] {
        assert!(requests.iter().any(|(observed, _)| observed == path));
    }
    second.stop(&secrets).await;

    config(&config_path, &restarted.url, &key.cleartext);
    let mut rejected = StdioClient::start(&repo, &config_path, Some(WRONG_TOKEN));
    rejected.initialize("acl_admin").await;
    let error = rejected.tool("cortex_memory_read", read_args(), true).await;
    assert_eq!(error["reason"], "authentication_required");
    let error = rejected
        .tool(
            "cortex_pre_thinking",
            json!({"user_prompt":"stdiorecall", "cwd":repo, "budget_ms":600}),
            true,
        )
        .await;
    assert_eq!(error["details"]["memory_outcome"], "unavailable");
    assert!(error["details"]["memory_delivery_id"].is_null());
    assert!(error["details"]["memory_revision"].is_null());
    assert_eq!(error["details"]["fail_open"], true);
    assert!(!error.to_string().contains("cortex: timeout"));
    let error = rejected
        .tool(
            "cortex_capture_memory",
            json!({
                "kind":"decision","repo":REPO,"memory_id":"UNAUTHORIZED-STDIO",
                "body":"Must never persist","agent":"admin"
            }),
            true,
        )
        .await;
    assert_eq!(error["reason"], "authentication_required");
    rejected.stop(&secrets).await;
    restarted.stop().await;
    let store = MetadataStore::open(&database).unwrap();
    assert!(store
        .daily_memory(&scope, "UNAUTHORIZED-STDIO")
        .unwrap()
        .is_none());
    assert!(store
        .pre_thinking_feedback(&initial_delivery)
        .unwrap()
        .is_some());
}
