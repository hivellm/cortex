//! Actual daemon, legacy_adapter hook, native shim, HTTP API and durable SQLite.
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use cortex_api::{
    build_router_with_auth_and_cfg, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, PrincipalStore, QueryService, RoleBinding,
};
use cortex_storage::daily_memory::{
    MemoryCapture, MemoryCategory, MemoryDelivery, MemoryDeliveryStage, MemoryScope,
};
use cortex_storage::MetadataStore;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

fn isolated(command: &mut Command, home: &Path, config: &Path) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("CORTEX_") {
            command.env_remove(name);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CORTEX_CONFIG_FILE", config)
        .current_dir(home)
        .kill_on_drop(true);
}

async fn hook(
    home: &Path,
    config: &Path,
    binding: &str,
    repo: &Path,
    event: &str,
    session: &str,
    prompt: &str,
) -> Value {
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks");
    let script = if event == "SessionStart" {
        "cortex-session-start.sh"
    } else {
        "cortex-user-prompt.sh"
    };
    #[cfg(windows)]
    let bash = "C:/Program Files/Git/bin/bash.exe";
    #[cfg(not(windows))]
    let bash = "bash";
    let mut command = Command::new(bash);
    isolated(&mut command, home, config);
    command
        .arg(package.join(script))
        .env("CORTEX_HOOK_BIN", env!("CARGO_BIN_EXE_cortex-hook"))
        .env("CORTEX_ADAPTER_PIPE", binding)
        .env("CORTEX_ADAPTER_SOCK", binding)
        .env("CORTEX_HOOK_STATE_DIR", home.join("notice-state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("legacy_adapter hook requires Bash");
    let input = json!({"session_id": session, "cwd": repo, "prompt": prompt, "source":"startup"});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout must be one JSON object")
}

#[tokio::test]
async fn legacy_adapter_lifecycle_returns_pinned_memory_and_corrected_prompt_from_real_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    let repo = home.join("project");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join("cortex.toml"), "[cortex]\nid='Lifecycle/Exact'\n").unwrap();
    let database = home.join("metadata.sqlite");
    let mut capture = MemoryCapture::new(
        "Lifecycle/Exact",
        "Retry policy",
        "Lifecycle canary: retry only idempotent requests.",
        MemoryCategory::Decision,
    );
    capture.id = Some("lifecycle-pinned".into());
    capture.pinned = true;
    let receipt = MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&capture)
        .unwrap();
    let mut cfg = cortex_config::Config::default();
    cfg.ingestion.metadata_db = Some(database.to_string_lossy().into_owned());
    cfg.ingestion.ingestion_url = Some("http://127.0.0.1:1".into());
    cfg.access_control.enabled = true;
    cfg.access_control.deny_on_missing_principal = true;
    let keys = Arc::new(cortex_api::storage::api_keys::ApiKeyStore::open_in_memory().unwrap());
    let key = keys
        .issue_with_role("memory", "lifecycle-client", Some("reader"))
        .unwrap();
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
    service.acl.allow(&key.id, "Lifecycle/Exact");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let authorization = format!("Bearer {}", key.cleartext);
    let router = build_router_with_auth_and_cfg(Arc::new(service), None, Some(keys), Arc::new(cfg))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let observed = observed.clone();
                let valid = request
                    .headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
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
    let config = home.join("runtime.toml");
    std::fs::write(
        &config,
        format!(
            "[runtime]\nprofile='daily'\n[adapter]\nadapter_admin_port=0\n[dashboard]\napi_token='{}'\n",
            key.cleartext
        ),
    )
    .unwrap();
    let wal_path = home.join(".cortex").join("overflow.wal");
    std::fs::create_dir_all(wal_path.parent().unwrap()).unwrap();
    let backlog = b"preserve dormant bulk backlog\n";
    std::fs::write(&wal_path, backlog).unwrap();
    let adapter_config = home.join("adapter.toml");
    std::fs::write(
        &adapter_config,
        format!("[adapter]\napi_endpoint='{api_url}'\nendpoint='http://127.0.0.1:1'\n"),
    )
    .unwrap();
    #[cfg(windows)]
    let binding = format!(r"\\.\pipe\cortex-lifecycle-{}", ulid::Ulid::new());
    #[cfg(not(windows))]
    let binding = home.join("adapter.sock").to_string_lossy().into_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex-adapter-claude"));
    isolated(&mut command, home, &config);
    let mut daemon = command
        .arg("--config")
        .arg(&adapter_config)
        .arg("--bind")
        .arg(&binding)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(
                daemon.try_wait().unwrap().is_none(),
                "adapter exited at startup"
            );
            #[cfg(windows)]
            let connected = tokio::net::windows::named_pipe::ClientOptions::new()
                .open(&binding)
                .is_ok();
            #[cfg(not(windows))]
            let connected = tokio::net::UnixStream::connect(&binding).await.is_ok();
            if connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("adapter did not bind");

    let start = hook(
        home,
        &config,
        &binding,
        &repo,
        "SessionStart",
        "fresh-a",
        "",
    )
    .await;
    assert_eq!(start["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = start["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("lifecycle-pinned"), "{start}");
    assert!(context.contains(&capture.body));
    assert!(context.len() <= 1536);
    assert!(start.get("systemMessage").is_none());
    let mut correction = MemoryCapture::new(
        "Lifecycle/Exact",
        "Retry policy corrected",
        "Lifecycle canary: retry only requests carrying an idempotency key.",
        MemoryCategory::Decision,
    );
    correction.id = Some("lifecycle-successor".into());
    correction.pinned = true;
    correction.supersedes = capture.id.clone();
    correction.expected_revision = Some(receipt.scope_revision);
    let corrected_receipt = MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&correction)
        .unwrap();
    let prompt = hook(
        home,
        &config,
        &binding,
        &repo,
        "UserPromptSubmit",
        "fresh-b",
        "Implement request retries",
    )
    .await;
    let context = prompt["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert_eq!(
        prompt["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert!(
        context.contains("lifecycle-successor") && context.contains(&correction.body),
        "{prompt}"
    );
    assert!(!context.contains("lifecycle-pinned"));
    assert!(!context.contains("cortex: timeout"));
    assert!(
        prompt.get("systemMessage").is_none(),
        "optional outage must not imply local outage"
    );
    let store = MetadataStore::open(&database).unwrap();
    let mut statement = store
        .conn()
        .prepare("SELECT data FROM daily_memory_deliveries ORDER BY prepared_at,id")
        .unwrap();
    let prepared: Vec<MemoryDelivery> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect();
    assert_eq!(
        prepared.len(),
        2,
        "each hook must prepare its own durable receipt"
    );
    for (delivery, expected) in prepared.iter().zip([&receipt, &corrected_receipt]) {
        assert_eq!(delivery.stage, MemoryDeliveryStage::Prepared);
        assert_eq!(delivery.request.repo, "Lifecycle/Exact");
        assert_eq!(delivery.request.memories.len(), 1);
        let item = &delivery.request.memories[0];
        assert_eq!(
            item.scope,
            MemoryScope::Repository("Lifecycle/Exact".into())
        );
        assert_eq!(item.id, expected.id);
        assert_eq!(item.event_id, expected.event_id);
        assert_eq!(item.version, expected.version);
    }
    let observed = requests.lock().unwrap().clone();
    for path in ["/v1/memory/query", "/v1/dashboard/active-work"] {
        assert!(
            observed
                .iter()
                .any(|(requested, valid)| requested == path && *valid),
            "production credential missing for {path}"
        );
    }
    daemon.kill().await.unwrap();
    daemon.wait().await.unwrap();
    assert_eq!(std::fs::read(&wal_path).unwrap(), backlog);
    api.abort();
    let _ = api.await;
    let outage = hook(
        home,
        &config,
        &binding,
        &repo,
        "SessionStart",
        "outage-c",
        "",
    )
    .await;
    assert!(outage["systemMessage"]
        .as_str()
        .unwrap()
        .contains("local memory is unavailable"));
    let repeated = hook(
        home,
        &config,
        &binding,
        &repo,
        "UserPromptSubmit",
        "outage-c",
        "Continue",
    )
    .await;
    assert_eq!(repeated, json!({}));
}
