//! Integration tests for `cortex_workers::ingestion::router`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_workers::ingestion::archive::InMemoryArchive;
use cortex_workers::ingestion::{
    build_router, AppState, ArchiveWriter, MemoryPublisher, Metrics,
};
use cortex_storage::{STREAM_EVENTS_BOOTSTRAP, STREAM_EVENTS_RAW};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tower::util::ServiceExt;

fn good_envelope() -> Value {
    json!({
        "event_id": "01HXYZABCDEF0123456789ABCD",
        "schema_version": "1",
        "occurred_at": "2026-04-17T12:34:56.789Z",
        "session_id": "01HXYZABCDEF0123456789ABCE",
        "stream": "live",
        "tool": "claude-code",
        "kind": "tool_call",
        "context": { "platform": "linux" },
        "payload": {
            "tool_name": "Bash",
            "input": { "command": "echo hi" },
            "outcome": "success"
        },
        "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    })
}

fn build_app() -> (Router, Arc<InMemoryArchive>, Arc<MemoryPublisher>, Arc<Metrics>) {
    let archive: Arc<InMemoryArchive> = Arc::new(InMemoryArchive::default());
    let publisher: Arc<MemoryPublisher> = Arc::new(MemoryPublisher::default());
    let metrics = Arc::new(Metrics::default());
    let archive_dyn: Arc<dyn ArchiveWriter> = archive.clone();
    let pub_dyn: Arc<dyn cortex_workers::ingestion::Publisher> = publisher.clone();
    let state = AppState::new(archive_dyn, pub_dyn, metrics.clone());
    (build_router(state), archive, publisher, metrics)
}

#[tokio::test]
async fn healthz_ok() {
    let (app, _, _, _) = build_app();
    let resp = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_one_event_accepted() {
    let (app, archive, publisher, metrics) = build_app();
    let body = serde_json::to_vec(&good_envelope()).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert_eq!(publisher.len(), 1);
    assert_eq!(publisher.calls()[0].0, STREAM_EVENTS_RAW);
    assert_eq!(archive.rows().len(), 1);
    let archived = &archive.rows()[0].1;
    assert!(archived.get("ingested_at").is_some());
    assert_eq!(metrics.events_received.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn bootstrap_header_routes_to_bootstrap_stream() {
    let (app, _, publisher, _) = build_app();
    let body = serde_json::to_vec(&good_envelope()).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .header("x-cortex-stream", "bootstrap")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert_eq!(publisher.calls()[0].0, STREAM_EVENTS_BOOTSTRAP);
}

#[tokio::test]
async fn invalid_envelope_is_rejected() {
    let (app, _, publisher, metrics) = build_app();
    let mut bad = good_envelope();
    bad.as_object_mut().unwrap().remove("content_hash");
    let body = serde_json::to_vec(&bad).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(publisher.len(), 0);
    assert_eq!(metrics.events_rejected.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn redacts_secrets_and_records_tokens() {
    let (app, _, publisher, metrics) = build_app();
    let mut env = good_envelope();
    env["payload"]["input"]["command"] =
        json!("curl -H 'Authorization: ghp_abcdefghijklmnopqrstuvwxyz0123456789'");
    let body = serde_json::to_vec(&env).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let published = &publisher.calls()[0].1;
    let cmd = published["payload"]["input"]["command"].as_str().unwrap();
    assert!(cmd.contains("[REDACTED:github_token]"));
    assert!(published["redactions"].is_array());
    assert!(metrics.redaction_hits.load(Ordering::Relaxed) >= 1);
}

#[tokio::test]
async fn batch_accepts_spec04_wrapped_events_object() {
    // Spec 04: `POST /v1/events/batch` body is `{ "events": [...] }`.
    // The cortex-adapter-claude publisher uses this exact shape.
    let (app, _, publisher, _) = build_app();
    let batch = json!({ "events": [good_envelope(), good_envelope()] });
    let body = serde_json::to_vec(&batch).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events/batch")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert_eq!(publisher.len(), 2);
}

#[tokio::test]
async fn batch_mixes_accept_and_reject() {
    let (app, _, publisher, _) = build_app();
    let bad = {
        let mut b = good_envelope();
        b.as_object_mut().unwrap().remove("content_hash");
        b
    };
    let batch = json!([good_envelope(), bad, good_envelope()]);
    let body = serde_json::to_vec(&batch).unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events/batch")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert_eq!(publisher.len(), 2);
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_text() {
    let (app, _, _, _) = build_app();
    let body = serde_json::to_vec(&good_envelope()).unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("cortex_events_received"));
}
