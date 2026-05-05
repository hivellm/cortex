use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures::stream::Stream;

use super::DashboardState;

/// `GET /v1/dashboard/stream` — SSE stream of dashboard delta events
/// (spec 21). On connect emits a single `event: hello` frame with
/// `{ server_ts, lost_window }`; subsequent frames carry one
/// [`cortex_core::DashboardEvent`] each, with the SSE `event:` field
/// set to the kind tag (`task.changed`, `handoff.appended`, …).
///
/// Lossy: when the per-subscriber broadcast lags, the handler emits a
/// fresh `hello { lost_window: true }` so the GUI can fire a global
/// `invalidateQueries` to resync, then resumes streaming.
pub(super) async fn dashboard_stream(
    State(state): State<DashboardState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let mut rx = state.events_bus.subscribe();
    let stream = async_stream::stream! {
        // First frame: hello with no lost-window — the subscription
        // started cleanly. Carries the server clock so the GUI can
        // detect skew when correlating timestamps.
        yield encode_dashboard_hello(false);

        loop {
            match rx.recv().await {
                Ok(event) => {
                    yield encode_dashboard_event(&event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Slow subscriber dropped some events. Tell the GUI
                    // it must resync, then keep streaming. The broadcast
                    // receiver auto-recovers — no resubscribe needed.
                    yield encode_dashboard_hello(true);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // Bus is gone — daemon shutting down. End the stream.
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn encode_dashboard_hello(lost_window: bool) -> Result<SseEvent, Infallible> {
    let body = serde_json::json!({
        "server_ts": chrono::Utc::now().to_rfc3339(),
        "lost_window": lost_window,
    });
    Ok(SseEvent::default().event("hello").data(body.to_string()))
}

fn encode_dashboard_event(event: &cortex_core::DashboardEvent) -> Result<SseEvent, Infallible> {
    let kind_tag = match event.kind {
        cortex_core::DashboardEventKind::TaskChanged => "task.changed",
        cortex_core::DashboardEventKind::HandoffAppended => "handoff.appended",
        cortex_core::DashboardEventKind::DecisionChanged => "decision.changed",
        cortex_core::DashboardEventKind::MemoryAppended => "memory.appended",
        cortex_core::DashboardEventKind::KnowledgeAdded => "knowledge.added",
        cortex_core::DashboardEventKind::LearningAdded => "learning.added",
    };
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    Ok(SseEvent::default()
        .id(event.event_id.clone())
        .event(kind_tag)
        .data(payload))
}
