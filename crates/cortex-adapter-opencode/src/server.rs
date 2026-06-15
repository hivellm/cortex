//! HTTP hook listener for the OpenCode plugin transport.
//!
//! Binds an axum server on `POST /hook`. Every inbound `HookFrame`
//! is parsed, converted to an `Envelope` via
//! `cortex_adapter_claude_code::events::build_event`, then forwarded
//! to the `OpenCodeProducer` via an MPSC channel.
//!
//! The handler is fail-open: every internal error returns
//! `Json(HookResponse::empty())` so the plugin session never breaks.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::Json;
use axum::Router;
use cortex_adapter_claude_code::events::{build_event, HookFrame, HookKind};
use cortex_adapter_claude_code::session::SessionManager;
use cortex_adapter_claude_code::HookResponse;
use cortex_core::events::Envelope;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::Notify;

/// State shared with every axum handler invocation.
#[derive(Clone)]
struct ServerState {
    tx: mpsc::Sender<(String, Envelope)>,
    sessions: Arc<SessionManager>,
    pid: u32,
}

/// Start the HTTP hook listener.
///
/// Spawns a tokio task that runs until `shutdown` is notified.
/// Inbound frames are forwarded to `tx` as `(session_id, Envelope)`
/// pairs. Non-publishing hooks (PreToolUse, SessionStart, etc.) are
/// handled but produce no channel message.
///
/// The returned `JoinHandle` completes when the server finishes its
/// graceful shutdown.
pub fn start_server(
    bind: String,
    tx: mpsc::Sender<(String, Envelope)>,
    shutdown: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run_server(bind, tx, shutdown).await {
            tracing::warn!(error = %e, "opencode http server exited with error");
        }
    })
}

async fn run_server(
    bind: String,
    tx: mpsc::Sender<(String, Envelope)>,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    let state = ServerState {
        tx,
        sessions: Arc::new(SessionManager::new()),
        pid: std::process::id(),
    };

    let app: Router = Router::new()
        .route("/hook", post(hook_handler))
        .with_state(state);

    let addr: std::net::SocketAddr = bind.parse().map_err(|e| {
        anyhow::anyhow!("CORTEX_ADAPTER_HTTP_BIND `{bind}` is not a valid socket address: {e}")
    })?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "opencode http listener up");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await?;

    tracing::info!("opencode http listener shutdown");
    Ok(())
}

/// POST /hook handler.
///
/// Parses the body as a `HookFrame`, builds the canonical envelope,
/// and forwards it to the producer channel. Always returns
/// `HookResponse::empty()` — never 500, never blocks the session.
async fn hook_handler(State(state): State<ServerState>, Json(body): Json<Value>) -> Json<Value> {
    let response = process_frame(body, &state).await;
    Json(serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({})))
}

async fn process_frame(body: Value, state: &ServerState) -> HookResponse {
    let frame: HookFrame = match serde_json::from_value(body) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "opencode: malformed hook frame; ignoring");
            return HookResponse::empty();
        }
    };

    let hook_kind = match HookKind::parse(&frame.hook) {
        Some(k) => k,
        None => {
            tracing::warn!(hook = %frame.hook, "opencode: unknown hook kind; ignoring");
            return HookResponse::empty();
        }
    };

    let session_id = frame
        .session_id
        .clone()
        .unwrap_or_else(|| format!("oc-{}", std::process::id()));

    let envelope = match build_event(hook_kind, &frame, &state.sessions, state.pid) {
        Some(env) => env,
        None => {
            // Non-publishing hook (PreToolUse, SessionStart, etc.) —
            // sync path only, no envelope produced.
            return HookResponse::empty();
        }
    };

    if let Err(e) = state.tx.try_send((session_id, envelope)) {
        tracing::warn!(error = %e, "opencode: channel send failed; dropping envelope");
    }

    HookResponse::empty()
}
