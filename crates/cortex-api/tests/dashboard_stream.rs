//! Integration tests for `/v1/dashboard/stream` (spec 21).
//!
//! Builds the dashboard router against a temp `.rulebook/`, opens an SSE
//! connection through `axum::Router::oneshot`, and asserts:
//!
//! 1. The first frame is `event: hello`.
//! 2. A subsequent `DashboardEvent` published on the bus surfaces on the
//!    stream tagged with the matching SSE `event:` field.
//!
//! The stream handler holds the connection open indefinitely, so we read
//! a bounded prefix of the response body with a timeout instead of
//! draining `to_bytes` (which would block forever).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cortex_api::dashboard_watcher::DashboardEventBus;
use cortex_api::{build_dashboard_router, DashboardState, MemoryKeywordLane, TaskLoader};
use cortex_core::{DashboardEvent, DashboardEventKind, DashboardEventSource};
use futures::StreamExt;
use tokio::time::timeout;
use tower::ServiceExt;

fn build_router(events_bus: DashboardEventBus) -> axum::Router {
    let tmp = std::path::PathBuf::from("__tests_no_rulebook__");
    let state = DashboardState {
        lane: Arc::new(MemoryKeywordLane::new()),
        nexus: None,
        analyzer: Arc::new(cortex_api::analyzer::Analyzer::from_env()),
        tasks: Arc::new(cortex_api::MultiTaskLoader::new(vec![TaskLoader::new(
            tmp,
        )])),
        metadata: None,
        loader_metrics: Arc::new(cortex_api::LoaderMetrics::new()),
        events_bus,
    };
    build_dashboard_router(state)
}

/// Read up to `max_bytes` from the body, stopping early when the buffer
/// already contains every needle. Returns the accumulated bytes.
///
/// This avoids `to_bytes`, which blocks until the body completes — and
/// the SSE stream never completes on a healthy connection.
async fn read_until(
    body: Body,
    needles: &[&str],
    max_bytes: usize,
    deadline: Duration,
) -> Vec<u8> {
    let collected = timeout(deadline, async move {
        let mut acc: Vec<u8> = Vec::new();
        let mut stream = body.into_data_stream();
        while acc.len() < max_bytes {
            match stream.next().await {
                Some(Ok(chunk)) => acc.extend_from_slice(&chunk),
                Some(Err(_)) | None => break,
            }
            let s = String::from_utf8_lossy(&acc);
            if needles.iter().all(|n| s.contains(n)) {
                break;
            }
        }
        acc
    })
    .await;
    collected.unwrap_or_default()
}

#[tokio::test]
async fn stream_emits_hello_frame_then_published_event() {
    let bus = DashboardEventBus::new();
    let router = build_router(bus.clone());

    let req = Request::builder()
        .uri("/v1/dashboard/stream")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/event-stream"),
        "expected SSE content-type, got {content_type:?}"
    );

    let body = resp.into_body();

    // Publish on a delay so the subscription is in place when the event
    // hits the bus. Without the gap, the broadcast send happens before
    // the handler's `subscribe()` call resolves and the event is lost.
    let bus_for_pub = bus.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        bus_for_pub.publish(DashboardEvent {
            event_id: "01J_TEST_TASK_CHANGED".to_string(),
            kind: DashboardEventKind::TaskChanged,
            entity_id: "phase11m_dashboard_push_cache".to_string(),
            summary: Some("status: in-progress".to_string()),
            ts: "2026-05-02T23:50:00Z".to_string(),
            delta: None,
            source: DashboardEventSource::Mcp,
        });
    });

    let bytes = read_until(
        body,
        &["event: hello", "event: task.changed", "01J_TEST_TASK_CHANGED"],
        16 * 1024,
        Duration::from_secs(5),
    )
    .await;
    let text = String::from_utf8_lossy(&bytes);

    assert!(
        text.contains("event: hello"),
        "missing hello frame, got: {text}"
    );
    assert!(
        text.contains("\"lost_window\":false"),
        "hello frame should report a clean window, got: {text}"
    );
    assert!(
        text.contains("event: task.changed"),
        "missing task.changed frame, got: {text}"
    );
    assert!(
        text.contains("phase11m_dashboard_push_cache"),
        "task event payload missing entity_id, got: {text}"
    );
    assert!(
        text.contains("01J_TEST_TASK_CHANGED"),
        "missing event id, got: {text}"
    );
}
