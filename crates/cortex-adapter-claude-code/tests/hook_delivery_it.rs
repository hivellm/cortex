//! Execute the compiled native hook and packaged Bash shims over local IPC.
//! Peers are hermetic protocol fixtures; this does not claim an installed
//! user daemon or a model consumed context. No production API is contacted.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

const NOTICE: &str =
    "Cortex: local memory is unavailable. This prompt continues without saved Cortex memory.";

struct Endpoint {
    flag: &'static str,
    env: &'static str,
    address: String,
}

fn missing_endpoint(root: &Path) -> Endpoint {
    #[cfg(windows)]
    {
        let _ = root;
        Endpoint {
            flag: "--pipe",
            env: "CORTEX_ADAPTER_PIPE",
            address: format!(r"\\.\pipe\cortex-hook-it-{}", ulid::Ulid::new()),
        }
    }
    #[cfg(unix)]
    Endpoint {
        flag: "--sock",
        env: "CORTEX_ADAPTER_SOCK",
        address: root.join("missing.sock").display().to_string(),
    }
}

async fn protocol_peer(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    reply: Option<Vec<u8>>,
    captured: oneshot::Sender<Value>,
) {
    let mut line = String::new();
    BufReader::new(&mut stream)
        .read_line(&mut line)
        .await
        .unwrap();
    let _ = captured.send(serde_json::from_str(&line).unwrap());
    if let Some(reply) = reply {
        stream.write_all(&reply).await.unwrap();
        stream.flush().await.unwrap();
    }
    // The hook must finish on a response newline or its own deadline, not EOF.
    std::future::pending::<()>().await;
}

async fn peer(
    root: &Path,
    reply: Option<Value>,
) -> (
    Endpoint,
    oneshot::Receiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let (send, receive) = oneshot::channel();
    let reply = reply.map(|value| format!("{value}\n").into_bytes());
    #[cfg(windows)]
    {
        let endpoint = missing_endpoint(root);
        let server = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(&endpoint.address)
            .unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            protocol_peer(server, reply, send).await;
        });
        (endpoint, receive, task)
    }
    #[cfg(unix)]
    {
        let endpoint = Endpoint {
            flag: "--sock",
            env: "CORTEX_ADAPTER_SOCK",
            address: root
                .join(format!("{}.sock", ulid::Ulid::new()))
                .display()
                .to_string(),
        };
        let listener = tokio::net::UnixListener::bind(&endpoint.address).unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            protocol_peer(stream, reply, send).await;
        });
        (endpoint, receive, task)
    }
}

fn hook_command(
    home: &Path,
    endpoint: &Endpoint,
    event: &str,
    packaged: bool,
    timeout_ms: u64,
) -> Command {
    let mut command = if packaged {
        let file = match event {
            "SessionStart" => "cortex-session-start.sh",
            "UserPromptSubmit" => "cortex-user-prompt.sh",
            _ => panic!("unsupported packaged test event"),
        };
        #[cfg(windows)]
        let bash = "C:/Program Files/Git/bin/bash.exe";
        #[cfg(not(windows))]
        let bash = "bash";
        let mut command = Command::new(bash);
        command.arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../packages/cortex-claude-plugin/hooks")
                .join(file),
        );
        command
    } else {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex-hook"));
        command.args([
            event,
            endpoint.flag,
            &endpoint.address,
            "--timeout-ms",
            &timeout_ms.to_string(),
        ]);
        command
    };
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CORTEX_ADAPTER_DISABLE", "false")
        .env("CORTEX_CONFIG_FILE", home.join("test-cortex.toml"))
        .env("CORTEX_HOOK_STATE_DIR", home.join("notice state"))
        .env("CLAUDE_SESSION_ID", "stale-env-session")
        .env(
            "CORTEX_HOOK_BIN",
            env!("CARGO_BIN_EXE_cortex-hook").replace('\\', "/"),
        )
        .env(endpoint.env, &endpoint.address)
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn invoke(
    home: &Path,
    endpoint: &Endpoint,
    event: &str,
    payload: &[u8],
    packaged: bool,
    timeout_ms: u64,
) -> Value {
    invoke_command(
        hook_command(home, endpoint, event, packaged, timeout_ms),
        payload,
    )
    .await
}

async fn invoke_command(mut command: Command, payload: &[u8]) -> Value {
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // Oversized input may be rejected before its trailing bytes are written.
    let _ = stdin.write_all(payload).await;
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(4), child.wait_with_output())
        .await
        .expect("hook failed to honor its finite I/O deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("hook stdout must be one JSON object")
}

fn home(root: &Path) -> PathBuf {
    let home = root.join("home with spaces");
    std::fs::create_dir_all(&home).unwrap();
    home
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_invalid_oversized_and_non_file_configuration_fail_open() {
    for config in [
        Some("[adapter\n".to_owned()),
        Some(format!("#{}\n", "x".repeat(64 * 1024))),
        None,
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let home = home(tmp.path());
        let path = home.join("test-cortex.toml");
        if let Some(config) = config {
            std::fs::write(&path, config).unwrap();
        } else {
            std::fs::create_dir(&path).unwrap();
        }
        let missing = missing_endpoint(tmp.path());
        let payload = json!({"session_id":"config-session"}).to_string();
        assert_eq!(
            invoke(
                &home,
                &missing,
                "UserPromptSubmit",
                payload.as_bytes(),
                false,
                200
            )
            .await,
            json!({})
        );
        let mut disabled = hook_command(&home, &missing, "UserPromptSubmit", false, 200);
        disabled.env("CORTEX_ADAPTER_DISABLE", "1");
        assert_eq!(
            invoke_command(disabled, payload.as_bytes()).await,
            json!({})
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packaged_legacy_disable_zero_preserves_configured_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let expected = json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit", "additionalContext":"native context"}});
    let (endpoint, capture, task) = peer(tmp.path(), Some(expected.clone())).await;
    // The packaged shim supplies no endpoint flags: only typed configuration
    // can preserve this environment override while normalizing legacy zero.
    let mut command = hook_command(&home, &endpoint, "UserPromptSubmit", true, 1500);
    command.env("CORTEX_ADAPTER_DISABLE", "0");
    assert_eq!(
        invoke_command(command, br#"{"session_id":"config-session"}"#).await,
        expected
    );
    capture.await.unwrap();
    task.abort();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_fifo_configuration_cannot_block_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    assert!(Command::new("mkfifo")
        .arg(home.join("test-cortex.toml"))
        .status()
        .await
        .unwrap()
        .success());
    let missing = missing_endpoint(tmp.path());
    let started = std::time::Instant::now();
    assert_eq!(
        invoke(
            &home,
            &missing,
            "UserPromptSubmit",
            br#"{"session_id":"fifo-session"}"#,
            false,
            200
        )
        .await,
        json!({})
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packaged_session_start_delivers_exact_context_and_payload_identity_without_eof() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let expected = json!({"hookSpecificOutput":{"hookEventName":"SessionStart", "additionalContext":"[memory ADR-42 v2 event correction-42]\nUse compare-and-swap.\n"}});
    let (endpoint, capture, task) = peer(tmp.path(), Some(expected.clone())).await;
    let payload =
        json!({"session_id":"actual-session", "cwd":"/actual checkout", "source":"resume"});
    let output = invoke(
        &home,
        &endpoint,
        "SessionStart",
        payload.to_string().as_bytes(),
        true,
        1500,
    )
    .await;
    assert_eq!(output, expected);
    let frame = capture.await.unwrap();
    assert_eq!(frame["hook"], "SessionStart");
    assert_eq!(frame["session_id"], "actual-session");
    assert_eq!(frame["cwd"], "/actual checkout");
    assert_eq!(frame["payload"], payload);
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transport_outage_is_once_per_actual_session_across_start_prompt_and_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let endpoint = missing_endpoint(tmp.path());
    let payload =
        json!({"session_id":"actual-session", "cwd":"/actual checkout", "source":"startup"});
    let first = invoke(
        &home,
        &endpoint,
        "SessionStart",
        payload.to_string().as_bytes(),
        false,
        200,
    )
    .await;
    assert_eq!(first, json!({"systemMessage":NOTICE}));
    let prompt = invoke(
        &home,
        &endpoint,
        "UserPromptSubmit",
        payload.to_string().as_bytes(),
        true,
        200,
    )
    .await;
    assert_eq!(prompt, json!({}));
    let resume = json!({"session_id":"actual-session", "cwd":"/other checkout", "source":"resume"});
    assert_eq!(
        invoke(
            &home,
            &endpoint,
            "SessionStart",
            resume.to_string().as_bytes(),
            false,
            200
        )
        .await,
        json!({})
    );
    let fresh = json!({"session_id":"new-actual-session", "cwd":"/actual checkout"});
    assert_eq!(
        invoke(
            &home,
            &endpoint,
            "UserPromptSubmit",
            fresh.to_string().as_bytes(),
            false,
            200
        )
        .await,
        json!({"systemMessage":NOTICE})
    );
    assert_eq!(
        std::fs::read_dir(home.join("notice state"))
            .unwrap()
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_notice_and_transport_failure_share_dedup_and_preserve_real_context() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let payload = json!({"session_id":"shared-session", "cwd":"/actual checkout"}).to_string();
    for (event, context, warn) in [
        ("SessionStart", "saved context first", true),
        ("UserPromptSubmit", "saved context fresh", false),
    ] {
        let response = json!({"systemMessage":NOTICE, "hookSpecificOutput":{"hookEventName":event,"additionalContext":context}});
        let (endpoint, capture, task) = peer(tmp.path(), Some(response)).await;
        let output = invoke(&home, &endpoint, event, payload.as_bytes(), false, 1500).await;
        assert_eq!(output.get("systemMessage").is_some(), warn);
        assert_eq!(output["hookSpecificOutput"]["additionalContext"], context);
        assert!(!output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains(NOTICE));
        capture.await.unwrap();
        task.abort();
        let missing = missing_endpoint(tmp.path());
        assert_eq!(
            invoke(&home, &missing, event, payload.as_bytes(), false, 200).await,
            json!({})
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_read_timeout_bad_input_and_size_limit_never_block_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let payload = json!({"session_id":"timeout-session", "cwd":"/actual checkout"}).to_string();
    let (endpoint, capture, task) = peer(tmp.path(), None).await;
    assert_eq!(
        invoke(
            &home,
            &endpoint,
            "UserPromptSubmit",
            payload.as_bytes(),
            false,
            200
        )
        .await,
        json!({"systemMessage":NOTICE})
    );
    capture.await.unwrap();
    task.abort();
    let missing = missing_endpoint(tmp.path());
    for input in [
        b"invalid json".to_vec(),
        b"[]".to_vec(),
        vec![b'x'; 1024 * 1024 + 64],
    ] {
        assert_eq!(
            invoke(&home, &missing, "UserPromptSubmit", &input, false, 200).await,
            json!({})
        );
    }
    // No reliable session identity means no invented "unknown" marker.
    assert_eq!(
        std::fs::read_dir(home.join("notice state"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_stdin_timeout_and_packaged_missing_binary_return_empty_success() {
    let tmp = tempfile::tempdir().unwrap();
    let home = home(tmp.path());
    let missing = missing_endpoint(tmp.path());
    let mut child = hook_command(&home, &missing, "UserPromptSubmit", false, 100)
        .spawn()
        .unwrap();
    let _held_open = child.stdin.take().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(4), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
    let mut command = hook_command(&home, &missing, "SessionStart", true, 100);
    command.env("CORTEX_HOOK_BIN", home.join("no native hook exists"));
    let output = command.output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
}
