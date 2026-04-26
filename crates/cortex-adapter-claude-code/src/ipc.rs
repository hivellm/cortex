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
#[cfg(not(windows))]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
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
    }
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
async fn handle_unix(
    stream: tokio::net::UnixStream,
    dispatcher: Arc<Dispatcher>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }
    let response = handle_line(&line, dispatcher).await;
    let mut bytes = serde_json::to_vec(&response)?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;
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
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 4096];
    loop {
        let n = server.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(idx) = buf.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..idx]).into_owned();
            let response = handle_line(&line, dispatcher).await;
            let mut bytes = serde_json::to_vec(&response)?;
            bytes.push(b'\n');
            server.write_all(&bytes).await?;
            break;
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
            tracing::warn!(error = %e, "malformed hook frame; replying empty");
            return HookResponse::empty();
        }
    };
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
