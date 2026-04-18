//! Axum HTTP router.
//!
//! Endpoints:
//! - `POST /v1/events` / `POST /v1/events/batch` — validates → redacts →
//!   archives → publishes. The incoming `X-Cortex-Stream` header selects
//!   between `cortex.events.raw` (live, default) and `cortex.events.bootstrap`.
//! - `GET /healthz` — liveness probe (no backend calls).
//! - `GET /metrics` — Prometheus text format.

use crate::archive::ArchiveWriter;
use crate::metrics::Metrics;
use crate::publisher::Publisher;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use cortex_storage::{STREAM_EVENTS_BOOTSTRAP, STREAM_EVENTS_INVALID, STREAM_EVENTS_RAW};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Handler state shared across requests.
#[derive(Clone)]
pub struct AppState {
    /// Durable archive writer.
    pub archive: Arc<dyn ArchiveWriter>,
    /// Bus publisher.
    pub publisher: Arc<dyn Publisher>,
    /// Metrics.
    pub metrics: Arc<Metrics>,
}

impl AppState {
    /// Build a new state. All handlers are `Clone + Send`.
    pub fn new(
        archive: Arc<dyn ArchiveWriter>,
        publisher: Arc<dyn Publisher>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            archive,
            publisher,
            metrics,
        }
    }
}

/// Build the Axum app.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .route("/v1/events", post(ingest_one))
        .route("/v1/events/batch", post(ingest_batch))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics(State(state): State<AppState>) -> Response {
    let body = state.metrics.render();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        body,
    )
        .into_response()
}

#[derive(Debug, Serialize)]
struct IngestResponse {
    event_id: String,
    stream: &'static str,
    redaction_hits: usize,
}

#[derive(Debug, Serialize)]
struct BatchResponse {
    accepted: usize,
    rejected: usize,
    errors: Vec<BatchError>,
}

#[derive(Debug, Serialize)]
struct BatchError {
    index: usize,
    event_id: Option<String>,
    errors: Vec<String>,
}

async fn ingest_one(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut envelope): Json<Value>,
) -> Response {
    match process_event(&state, &headers, &mut envelope).await {
        Ok((stream, hits)) => {
            let event_id = envelope
                .get("event_id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            (
                StatusCode::ACCEPTED,
                Json(IngestResponse {
                    event_id,
                    stream,
                    redaction_hits: hits,
                }),
            )
                .into_response()
        }
        Err(err) => err.into_response(state.clone()).await,
    }
}

async fn ingest_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(envelopes): Json<Vec<Value>>,
) -> Response {
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut errors = Vec::new();
    for (i, mut envelope) in envelopes.into_iter().enumerate() {
        match process_event(&state, &headers, &mut envelope).await {
            Ok(_) => accepted += 1,
            Err(err) => {
                rejected += 1;
                errors.push(BatchError {
                    index: i,
                    event_id: envelope
                        .get("event_id")
                        .and_then(|s| s.as_str())
                        .map(String::from),
                    errors: err.messages(),
                });
            }
        }
    }
    (
        StatusCode::ACCEPTED,
        Json(BatchResponse {
            accepted,
            rejected,
            errors,
        }),
    )
        .into_response()
}

#[derive(Debug)]
enum IngestError {
    InvalidJson(Vec<String>),
    ArchiveFailure(String),
    PublisherFailure(String),
}

impl IngestError {
    fn messages(&self) -> Vec<String> {
        match self {
            IngestError::InvalidJson(v) => v.clone(),
            IngestError::ArchiveFailure(s) => vec![format!("archive: {s}")],
            IngestError::PublisherFailure(s) => vec![format!("publisher: {s}")],
        }
    }

    async fn into_response(self, state: AppState) -> Response {
        match self {
            IngestError::InvalidJson(errors) => {
                state.metrics.events_rejected.fetch_add(1, Ordering::Relaxed);
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "stream": STREAM_EVENTS_INVALID, "errors": errors })),
                )
                    .into_response()
            }
            IngestError::ArchiveFailure(e) => {
                state.metrics.archive_errors.fetch_add(1, Ordering::Relaxed);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "archive_failure", "detail": e })),
                )
                    .into_response()
            }
            IngestError::PublisherFailure(e) => {
                state
                    .metrics
                    .publisher_errors
                    .fetch_add(1, Ordering::Relaxed);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "publisher_failure", "detail": e })),
                )
                    .into_response()
            }
        }
    }
}

async fn process_event(
    state: &AppState,
    headers: &HeaderMap,
    envelope: &mut Value,
) -> Result<(&'static str, usize), IngestError> {
    let stream = pick_stream(envelope, headers);
    let stream_name = match stream {
        StreamPick::Raw => STREAM_EVENTS_RAW,
        StreamPick::Bootstrap => STREAM_EVENTS_BOOTSTRAP,
    };

    // Stamp server-owned fields before anything else touches the envelope.
    stamp_server_fields(envelope, stream);

    // Redact any secrets inside the payload. Defense-in-depth — adapters
    // are expected to redact on their side too.
    let hits = if let Some(payload) = envelope.get_mut("payload") {
        let report = cortex_core::redact(payload);
        if !report.tokens.is_empty() {
            let entry = envelope
                .as_object_mut()
                .expect("envelope is a JSON object")
                .entry("redactions".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(existing) = entry {
                for t in &report.tokens {
                    existing.push(Value::String(t.clone()));
                }
            }
        }
        report.tokens.len()
    } else {
        0
    };
    state
        .metrics
        .redaction_hits
        .fetch_add(hits as u64, Ordering::Relaxed);

    // Validate AFTER redaction so the stored shape is exactly what we ship.
    if let Err(errs) = cortex_core::validate_event(envelope) {
        let msgs = errs.iter().map(|e| e.to_string()).collect();
        return Err(IngestError::InvalidJson(msgs));
    }

    // Archive first (durability contract), then publish.
    state
        .archive
        .write(stream.tag(), envelope)
        .map_err(|e| IngestError::ArchiveFailure(e.to_string()))?;
    state
        .publisher
        .publish(stream_name, envelope)
        .await
        .map_err(|e| IngestError::PublisherFailure(e.to_string()))?;

    state.metrics.events_received.fetch_add(1, Ordering::Relaxed);
    match stream {
        StreamPick::Raw => state.metrics.events_routed_raw.fetch_add(1, Ordering::Relaxed),
        StreamPick::Bootstrap => state
            .metrics
            .events_routed_bootstrap
            .fetch_add(1, Ordering::Relaxed),
    };
    Ok((stream_name, hits))
}

#[derive(Clone, Copy)]
enum StreamPick {
    Raw,
    Bootstrap,
}

impl StreamPick {
    fn tag(self) -> &'static str {
        match self {
            StreamPick::Raw => "raw",
            StreamPick::Bootstrap => "bootstrap",
        }
    }
}

fn pick_stream(envelope: &Value, headers: &HeaderMap) -> StreamPick {
    if let Some(h) = headers.get("x-cortex-stream").and_then(|v| v.to_str().ok()) {
        match h.to_ascii_lowercase().as_str() {
            "bootstrap" => return StreamPick::Bootstrap,
            "live" | "raw" => return StreamPick::Raw,
            _ => {}
        }
    }
    match envelope.get("stream").and_then(|v| v.as_str()) {
        Some("bootstrap") => StreamPick::Bootstrap,
        _ => StreamPick::Raw,
    }
}

fn stamp_server_fields(envelope: &mut Value, stream: StreamPick) {
    if let Some(map) = envelope.as_object_mut() {
        map.insert(
            "ingested_at".into(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
        // Force-align the `stream` field with what the router picked so
        // downstream consumers don't have to consult the header.
        map.insert(
            "stream".into(),
            Value::String(
                match stream {
                    StreamPick::Raw => "live",
                    StreamPick::Bootstrap => "bootstrap",
                }
                .to_string(),
            ),
        );
        if !map.contains_key("event_id") {
            map.insert(
                "event_id".into(),
                Value::String(cortex_core::event_id()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::InMemoryArchive;
    use crate::publisher::MemoryPublisher;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
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
        let state = AppState::new(archive.clone(), publisher.clone(), metrics.clone());
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
        assert_eq!(
            metrics.events_received.load(Ordering::Relaxed),
            1
        );
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
}
