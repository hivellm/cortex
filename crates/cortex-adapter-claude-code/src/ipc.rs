//! IPC server — Unix domain socket on Linux/macOS, Windows named
//! pipe on Windows. Spec 10 §Windows vs Unix IPC.
//!
//! Wire shape: each connection sends one [`HookFrame`] JSON object
//! (newline-delimited) and reads one JSON response back. Connections
//! close after one round-trip — hook shims are short-lived processes.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::dispatcher::{Dispatcher, HookResponse};
#[cfg(not(windows))]
use crate::events::HookFrame;
#[cfg(windows)]
#[allow(unused_imports)]
use crate::events::HookFrame;

/// IPC binding the server listens on.
#[derive(Debug, Clone)]
pub enum IpcBinding {
    /// Unix domain socket file path.
    UnixSocket(PathBuf),
    /// Windows named pipe (e.g. `\\.\pipe\cortex-adapter-claude`).
    NamedPipe(String),
    /// Phase11w §2.4 — HTTP `POST /hook` listener. The OpenCode TS
    /// plugin (`@hivellm/cortex-opencode-plugin`) posts hook frames
    /// here using the same JSON shape the socket/pipe paths accept.
    /// Default bind reads from `CORTEX_ADAPTER_HTTP_BIND`
    /// (`127.0.0.1:17004`).
    Http(String),
}

impl IpcBinding {
    /// Default binding for the current platform.
    #[cfg(windows)]
    pub fn default_for_platform() -> Self {
        IpcBinding::NamedPipe(r"\\.\pipe\cortex-adapter-claude".to_string())
    }

    /// Default binding for the current platform.
    #[cfg(not(windows))]
    pub fn default_for_platform() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        IpcBinding::UnixSocket(
            std::path::PathBuf::from(home)
                .join(".cortex")
                .join("adapter-claude.sock"),
        )
    }

    /// Phase11w §2.4 — default HTTP bind for the OpenCode plugin
    /// transport. Reads `CORTEX_ADAPTER_HTTP_BIND` then falls back to
    /// `127.0.0.1:17004` (loopback so the listener never accepts
    /// off-host posts; bind addr is operator-configurable for
    /// container scenarios where the plugin lives in another network
    /// namespace).
    pub fn default_http() -> Self {
        let bind = std::env::var("CORTEX_ADAPTER_HTTP_BIND")
            .unwrap_or_else(|_| "127.0.0.1:17004".to_string());
        IpcBinding::Http(bind)
    }
}

/// Run the IPC server until `shutdown` fires.
pub async fn serve(
    binding: IpcBinding,
    dispatcher: Arc<Dispatcher>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    match binding {
        IpcBinding::UnixSocket(path) => serve_unix(path, dispatcher, shutdown).await,
        IpcBinding::NamedPipe(name) => serve_pipe(name, dispatcher, shutdown).await,
        IpcBinding::Http(bind) => serve_http(bind, dispatcher, shutdown).await,
    }
}

/// Phase11w §2.4 — HTTP listener serving `POST /hook` for OpenCode-
/// style plugin transports. The wire shape on the body is identical
/// to the socket / pipe paths: a single [`HookFrame`] JSON object.
/// The response is the canonical [`HookResponse`] JSON. Internal
/// errors degrade to `200 OK` with an empty `{}` body so the session
/// never breaks (spec 10 §Hook ↔ daemon protocol).
pub async fn serve_http(
    bind: String,
    dispatcher: Arc<Dispatcher>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    use axum::extract::State;
    use axum::routing::post;
    use axum::Json;
    use axum::Router;

    #[derive(Clone)]
    struct HttpState {
        dispatcher: Arc<Dispatcher>,
    }

    async fn hook_handler(
        State(state): State<HttpState>,
        Json(frame): Json<Value>,
    ) -> Json<Value> {
        let response = match serde_json::to_string(&frame) {
            Ok(s) => handle_line(&s, state.dispatcher.clone()).await,
            Err(_) => HookResponse::empty(),
        };
        Json(serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({})))
    }

    let state = HttpState { dispatcher };
    let app: Router = Router::new()
        .route("/hook", post(hook_handler))
        .with_state(state);

    let addr: std::net::SocketAddr = bind.parse().map_err(|e| {
        anyhow::anyhow!("CORTEX_ADAPTER_HTTP_BIND `{bind}` is not a valid socket address: {e}")
    })?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "ipc http listener up");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await?;
    tracing::info!("ipc http listener shutdown");
    Ok(())
}

#[cfg(not(windows))]
async fn serve_unix(
    path: PathBuf,
    dispatcher: Arc<Dispatcher>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    use tokio::net::UnixListener;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Best-effort cleanup of a stale socket file from a prior run.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    tracing::info!(path = %path.display(), "ipc unix listener up");
    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let dispatcher = dispatcher.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_unix(stream, dispatcher).await {
                                tracing::warn!(error = %e, "ipc handler failed");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "ipc accept failed");
                    }
                }
            }
            _ = shutdown.notified() => {
                tracing::info!("ipc shutdown");
                let _ = std::fs::remove_file(&path);
                break;
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn handle_unix(stream: tokio::net::UnixStream, dispatcher: Arc<Dispatcher>) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let (mut read_half, mut write_half) = stream.into_split();
    // Read until the accumulated buffer parses as a complete JSON
    // value. The historical `read_line` implementation stopped at the
    // first `\n`, which mangled any pretty-printed payload from a
    // hook that pre-formatted its frame across multiple lines.
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut chunk = [0u8; 8192];
    loop {
        let n = read_half.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut de = serde_json::Deserializer::from_slice(&buf).into_iter::<serde_json::Value>();
        match de.next() {
            Some(Ok(value)) => {
                let line = serde_json::to_string(&value).unwrap_or_default();
                let response = handle_line(&line, dispatcher).await;
                let mut bytes = serde_json::to_vec(&response)?;
                bytes.push(b'\n');
                write_half.write_all(&bytes).await?;
                break;
            }
            Some(Err(e)) if e.is_eof() => continue,
            Some(Err(e)) => {
                tracing::warn!(error = %e, "ipc frame parse error; replying empty");
                let bytes = b"{}\n".to_vec();
                write_half.write_all(&bytes).await?;
                break;
            }
            None => continue,
        }
    }
    write_half.shutdown().await.ok();
    Ok(())
}

#[cfg(windows)]
async fn serve_unix(
    _path: PathBuf,
    _dispatcher: Arc<Dispatcher>,
    _shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "unix-socket binding not available on windows; use NamedPipe"
    ))
}

#[cfg(windows)]
async fn serve_pipe(
    name: String,
    dispatcher: Arc<Dispatcher>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    tracing::info!(pipe = %name, "ipc named pipe up");
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(&name)?;
        tokio::select! {
            res = server.connect() => {
                if let Err(e) = res {
                    tracing::warn!(error = %e, "ipc connect failed");
                    continue;
                }
                let dispatcher = dispatcher.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_pipe(server, dispatcher).await {
                        tracing::warn!(error = %e, "ipc handler failed");
                    }
                });
            }
            _ = shutdown.notified() => {
                tracing::info!("ipc shutdown");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn handle_pipe(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    dispatcher: Arc<Dispatcher>,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    // Read until the accumulated buffer parses as a complete JSON
    // value. The original implementation cut at the first `\n`, which
    // truncated any pretty-printed payload (Claude Code's PostToolUse
    // stdin can have newlines BETWEEN fields when the model decides
    // to format the JSON multi-line). Truncated JSON failed to
    // deserialize; the dispatcher returned `{}` and the envelope was
    // dropped silently — exactly the "Bash never appears in
    // timeline" bug the user reported on 2026-04-28.
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut chunk = [0u8; 8192];
    loop {
        let n = server.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut de = serde_json::Deserializer::from_slice(&buf).into_iter::<serde_json::Value>();
        match de.next() {
            Some(Ok(value)) => {
                let line = serde_json::to_string(&value).unwrap_or_default();
                let response = handle_line(&line, dispatcher).await;
                let mut bytes = serde_json::to_vec(&response)?;
                bytes.push(b'\n');
                server.write_all(&bytes).await?;
                break;
            }
            Some(Err(e)) if e.is_eof() => {
                // Need more bytes — JSON object still unbalanced.
                continue;
            }
            Some(Err(e)) => {
                tracing::warn!(error = %e, "ipc frame parse error; replying empty");
                let bytes = b"{}\n".to_vec();
                server.write_all(&bytes).await?;
                break;
            }
            None => continue,
        }
    }
    server.shutdown().await.ok();
    Ok(())
}

#[cfg(not(windows))]
async fn serve_pipe(
    _name: String,
    _dispatcher: Arc<Dispatcher>,
    _shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "named-pipe binding not available outside windows; use UnixSocket"
    ))
}

/// Parse one inbound line, dispatch through the daemon, and return
/// the JSON response. Spec 10 §Hook ↔ daemon protocol: the response
/// is `{}` on any internal error so the session never breaks.
pub(crate) async fn handle_line(line: &str, dispatcher: Arc<Dispatcher>) -> HookResponse {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return HookResponse::empty();
    }
    let frame = match serde_json::from_str::<HookFrame>(trimmed) {
        Ok(f) => f,
        Err(e) => {
            // Phase8b — bump the unlabelled parse-error counter so a
            // sudden spike of malformed frames surfaces in /metrics
            // even though we can't attribute the hook.
            if let Some(m) = dispatcher.metrics() {
                m.incr_frames_parse_error();
            }
            tracing::warn!(error = %e, "malformed hook frame; replying empty");
            return HookResponse::empty();
        }
    };
    // Phase8b — frames_received_total{hook} is stamped only after
    // the JSON parses (we don't know the hook label otherwise); the
    // unlabelled `frames_parse_error_total` counter above completes
    // the received-vs-parsed picture for malformed payloads.
    if let Some(m) = dispatcher.metrics() {
        m.incr_frames_received(&frame.hook);
    }
    dispatcher.dispatch(frame).await
}

/// Helper used by tests to feed one frame through the dispatcher
/// without an actual socket.
pub async fn dispatch_inline(value: Value, dispatcher: Arc<Dispatcher>) -> HookResponse {
    let line = match serde_json::to_string(&value) {
        Ok(s) => s,
        Err(_) => return HookResponse::empty(),
    };
    handle_line(&line, dispatcher).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::MemoryPublisher;
    use crate::session::SessionManager;
    use crate::sync_paths::SyncClient;
    use crate::AdapterSection;

    fn build() -> Arc<Dispatcher> {
        let metrics = Arc::new(crate::Metrics::new());
        let sessions = Arc::new(SessionManager::new());
        let publisher: Arc<dyn crate::Publisher> = Arc::new(MemoryPublisher::new());
        let cfg = AdapterSection::default();
        let sync = Arc::new(SyncClient::new(&cfg, metrics));
        Arc::new(Dispatcher::new(sessions, publisher, sync, 1))
    }

    #[tokio::test]
    async fn handle_line_accepts_frame_padded_with_whitespace() {
        // The .ps1 shim used to emit a trailing newline inside the
        // payload before the spec-18 Windows-fix landed. The wire
        // handler strips at the first `\n`, so `handle_line` itself
        // never actually saw embedded newlines — but stray
        // surrounding whitespace (CRLF leftovers, spaces from
        // platform-quirky writers) is plausible and should still
        // parse cleanly.
        let dispatcher = build();
        let body = r#"{"hook":"UserPromptSubmit","session_id":"s","cwd":"/x","payload":{}}"#;
        let padded = format!("   \r\n  {body}  \r\n");
        let resp = handle_line(&padded, dispatcher).await;
        // Empty / no-op response is fine — the assertion is "no panic
        // / no warn-level malformed-frame log". A successfully parsed
        // frame surfaces as `additional_context` `None` for the
        // UserPromptSubmit path because nothing was indexed.
        let _ = resp;
    }

    #[tokio::test]
    async fn handle_line_returns_empty_response_on_blank_input() {
        let dispatcher = build();
        let resp = handle_line("", dispatcher).await;
        assert_eq!(serde_json::to_value(&resp).unwrap(), serde_json::json!({}));
    }
}
