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
    /// Phase8a — process start instant + writable archive root the
    /// `/healthz` handler reports on.
    pub started_at: Arc<std::time::Instant>,
    /// Archive root path (best-effort writability probe in `/healthz`).
    pub archive_root: Arc<std::path::PathBuf>,
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
            started_at: Arc::new(std::time::Instant::now()),
            archive_root: Arc::new(std::path::PathBuf::new()),
        }
    }

    /// Phase8a — record the resolved archive root so `/healthz` can
    /// surface a writability probe extra. The boot path in `main.rs`
    /// chains `with_archive_root(cfg.archive_root)` after `new()`.
    pub fn with_archive_root(mut self, root: std::path::PathBuf) -> Self {
        self.archive_root = Arc::new(root);
        self
    }
}

/// Build the Axum app.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .route("/v1/events", post(ingest_one))
        .route("/v1/events/batch", post(ingest_batch))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Response {
    use cortex_health::{HealthState, SubsystemStatus, DEFAULT_FRESHNESS_DEGRADED_SECS};
    let started_at = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::milliseconds(
            i64::try_from(state.started_at.elapsed().as_millis()).unwrap_or(0),
        ))
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let mut status = SubsystemStatus::ok(
        "cortex-ingestion",
        env!("CARGO_PKG_VERSION"),
        started_at,
    );
    // Archive root writability — the readwrite property of the
    // mount that backs the events archive. A non-writable root is
    // hard-down: every ingest call would fail.
    let root = state.archive_root.as_ref().clone();
    let archive_writable = if root.as_os_str().is_empty() {
        // No root configured (test harness path) — report as
        // unknown rather than failing the probe. The metric still
        // surfaces in extras so the operator can see the gap.
        false
    } else {
        // Probe by trying to create + remove a tiny file. Avoids
        // the "directory exists" trap of `metadata()` (read-only
        // mounts still return success there).
        let probe = root.join(".cortex-ingestion-healthz");
        let writable = std::fs::write(&probe, b"ok").is_ok();
        if writable {
            let _ = std::fs::remove_file(&probe);
        }
        writable
    };
    status.extras.insert(
        "archive_root".into(),
        serde_json::Value::String(root.display().to_string()),
    );
    status.extras.insert(
        "archive_writable".into(),
        serde_json::Value::Bool(archive_writable),
    );
    // Last-batch-accepted timestamp — Metrics tracks the last
    // successful ingest. Stale > 60s flips the state to degraded
    // (the upstream caller went silent).
    let last_batch_ts_ms = state.metrics.last_batch_accepted_ts_ms();
    status.extras.insert(
        "last_batch_accepted_ts_ms".into(),
        serde_json::json!(last_batch_ts_ms),
    );
    // Phase8b — surface the per-kind throughput + freshness so the
    // cortex-api freshness aggregator can pivot per kind without a
    // separate /metrics scrape. Each map is a JSON object keyed by
    // the canonical kind (`turn`, `tool_call`, …).
    status.extras.insert(
        "events_received_total".into(),
        serde_json::to_value(state.metrics.events_received_snapshot()).unwrap_or_default(),
    );
    status.extras.insert(
        "events_archived_total".into(),
        serde_json::to_value(state.metrics.events_archived_snapshot()).unwrap_or_default(),
    );
    status.extras.insert(
        "events_rejected_total".into(),
        serde_json::to_value(state.metrics.events_rejected_snapshot()).unwrap_or_default(),
    );
    status.extras.insert(
        "last_archive_write_ts_ms".into(),
        serde_json::to_value(state.metrics.last_archive_write_ts_snapshot())
            .unwrap_or_default(),
    );
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let stale = last_batch_ts_ms > 0
        && now_ms.saturating_sub(last_batch_ts_ms)
            > DEFAULT_FRESHNESS_DEGRADED_SECS * 1_000;
    if !archive_writable && !root.as_os_str().is_empty() {
        status.state = HealthState::Down;
        status.last_error = Some(format!(
            "archive root not writable: {}",
            root.display()
        ));
    } else if stale {
        status.state = HealthState::Degraded;
        status.last_error = Some(format!(
            "no batch accepted in last {} secs",
            DEFAULT_FRESHNESS_DEGRADED_SECS
        ));
    }
    let http_status = match status.state {
        HealthState::Ok | HealthState::Degraded => StatusCode::OK,
        HealthState::Down => StatusCode::SERVICE_UNAVAILABLE,
    };
    (http_status, axum::Json(status)).into_response()
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
    Json(payload): Json<Value>,
) -> Response {
    // Spec-04 §POST /v1/events/batch: `{ "events": [Envelope, ...] }`.
    // Tolerate a bare array too — older callers + tests rely on it.
    let envelopes: Vec<Value> = match payload {
        Value::Array(a) => a,
        Value::Object(mut m) => match m.remove("events") {
            Some(Value::Array(a)) => a,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(BatchResponse {
                        accepted: 0,
                        rejected: 0,
                        errors: vec![BatchError {
                            index: 0,
                            event_id: None,
                            errors: vec!["expected `events` array (spec 04)".into()],
                        }],
                    }),
                )
                    .into_response();
            }
        },
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(BatchResponse {
                    accepted: 0,
                    rejected: 0,
                    errors: vec![BatchError {
                        index: 0,
                        event_id: None,
                        errors: vec!["batch body must be an object or array".into()],
                    }],
                }),
            )
                .into_response();
        }
    };
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut errors = Vec::new();
    let total = envelopes.len();
    for (i, mut envelope) in envelopes.into_iter().enumerate() {
        match process_event(&state, &headers, &mut envelope).await {
            Ok(_) => accepted += 1,
            Err(err) => {
                rejected += 1;
                state.metrics.events_rejected.fetch_add(1, Ordering::Relaxed);
                let event_id = envelope
                    .get("event_id")
                    .and_then(|s| s.as_str())
                    .map(String::from);
                let kind = envelope
                    .get("kind")
                    .and_then(|s| s.as_str())
                    .unwrap_or("?")
                    .to_string();
                let session_id = envelope
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("?")
                    .to_string();
                let messages = err.messages();
                let reason_class = match &err {
                    IngestError::InvalidJson(_) => "invalid_json",
                    IngestError::ArchiveFailure(_) => "archive_failure",
                    IngestError::PublisherFailure(_) => "publisher_failure",
                };
                tracing::warn!(
                    batch_index = i,
                    event_id = event_id.as_deref().unwrap_or("?"),
                    kind = %kind,
                    session_id = %session_id,
                    reason_class,
                    errors = ?messages,
                    "ingest_batch rejected envelope"
                );
                errors.push(BatchError {
                    index: i,
                    event_id,
                    errors: messages,
                });
            }
        }
    }
    if rejected > 0 {
        tracing::warn!(
            total,
            accepted,
            rejected,
            "ingest_batch finished with rejections (response is 202 Accepted — clients ignoring the body will see silent loss)"
        );
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
                state.metrics.incr_events_rejected_reason("invalid_json");
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "stream": STREAM_EVENTS_INVALID, "errors": errors })),
                )
                    .into_response()
            }
            IngestError::ArchiveFailure(e) => {
                state.metrics.archive_errors.fetch_add(1, Ordering::Relaxed);
                state.metrics.incr_events_rejected_reason("archive_failure");
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
                state
                    .metrics
                    .incr_events_rejected_reason("publisher_failure");
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
        // Phase8b — track validation drops by coarse reason so the
        // freshness aggregator's `ingestion.rejected.validation` row
        // surfaces schema drift the moment it lands.
        state.metrics.incr_events_rejected_reason("validation");
        return Err(IngestError::InvalidJson(msgs));
    }

    // Phase8b — pivot the per-kind received counter BEFORE the
    // archive write so a hard archive failure (disk full, permission)
    // still leaves a footprint in the freshness aggregator's
    // `ingestion.received.<kind>` row. The pair with
    // `events_archived_total{kind}` after the write is the smoking
    // gun for archive backpressure.
    let kind_label = envelope
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    state.metrics.incr_events_received_kind(&kind_label);

    // Archive first (durability contract), then publish.
    state
        .archive
        .write(stream.tag(), envelope)
        .map_err(|e| IngestError::ArchiveFailure(e.to_string()))?;

    // Bump the durability counter as soon as the archive accepts the
    // envelope. The publisher (synap) call below is best-effort: a
    // failed publish does NOT undo the archive write, so a post-archive
    // increment is the honest "this many envelopes survived to disk"
    // signal. Without this split, a flapping synap connection looked
    // like total ingestion silence in /metrics even though the archive
    // (which is what the dashboard reads) was healthy.
    state.metrics.events_received.fetch_add(1, Ordering::Relaxed);
    // Phase8b — per-kind durability stamp + last_archive_write_ts.
    state.metrics.incr_events_archived_kind(&kind_label);

    if let Err(e) = state.publisher.publish(stream_name, envelope).await {
        // Emit a structured WARN here so the failure is visible in
        // logs the moment the underlying bus (synap) misbehaves.
        // The batch path also reports rejected:N back to the caller,
        // but adapter publishers historically ignore the 202 body —
        // surfacing it here gives operators a single grep target.
        let event_id = envelope
            .get("event_id")
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .to_string();
        let kind = envelope
            .get("kind")
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .to_string();
        tracing::warn!(
            event_id = %event_id,
            kind = %kind,
            stream = stream_name,
            error = %e,
            "publisher publish failed (envelope IS already archived); returning rejected to caller"
        );
        return Err(IngestError::PublisherFailure(e.to_string()));
    }

    match stream {
        StreamPick::Raw => state.metrics.events_routed_raw.fetch_add(1, Ordering::Relaxed),
        StreamPick::Bootstrap => state
            .metrics
            .events_routed_bootstrap
            .fetch_add(1, Ordering::Relaxed),
    };
    // Phase8a — stamp the most recent successful ingest so
    // /healthz can detect "no batch in last 60s" and downgrade.
    state.metrics.record_batch_accepted_now();
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

