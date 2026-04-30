//! Phase10j — `POST /v1/ingest` proxy.
//!
//! Wraps the small subset of `cortex-ingestion`'s `POST /v1/events`
//! the agent capture flow needs (memory / knowledge / learning kinds)
//! behind a stable URL on `cortex-api`. Rationale: the MCP server
//! already knows where to find `cortex-api` via its `api_url`; making
//! the agent learn a second URL for ingestion would mean every host
//! has two configs to keep in sync.
//!
//! The proxy:
//! 1. Accepts `{ kind, body, repo, topic?, severity? }`.
//! 2. Validates `body` is non-empty and ≤ `MAX_BODY_BYTES`.
//! 3. Builds a canonical envelope (spec-04) with `event_id`,
//!    `kind`, `payload.body`, `context.repo`, optional
//!    `context.topic`, optional `severity`.
//! 4. Forwards it to `cortex-ingestion` via `POST /v1/events`. The
//!    upstream URL comes from `CORTEX_INGESTION_URL`
//!    (defaults to `http://127.0.0.1:17010`).
//! 5. Returns `{ event_id, content_hash, indexed_at }` synchronously.
//!
//! When `cortex-ingestion` is unreachable, the proxy returns 503 so
//! the agent can fall back to local capture (`rulebook_memory_save`)
//! without blocking on network failures.

use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::http::ApiState;

/// Spec — capture body is capped at 8 KiB. Larger bodies belong in
/// the file-backed knowledge / decision stores, not the live lane.
pub const MAX_BODY_BYTES: usize = 8 * 1024;

/// Default upstream URL for `cortex-ingestion`. Overridable via
/// the `CORTEX_INGESTION_URL` env var for non-default deployments.
pub const DEFAULT_INGESTION_URL: &str = "http://127.0.0.1:17010";

/// Inbound request shape — the three kinds the operator-curated
/// surface exposes plus the minimum metadata the freshness lane
/// needs to attribute the envelope back to a repo.
#[derive(Debug, Clone, Deserialize)]
pub struct IngestRequest {
    /// Event kind. Restricted to operator-curated kinds; the live
    /// adapter pipelines (Stop hook, classifier worker) write to
    /// `cortex-ingestion` directly with `kind=turn` / `tool_call` /
    /// `agent_call`.
    pub kind: String,
    /// Free-form body text. Capped at [`MAX_BODY_BYTES`].
    pub body: String,
    /// Repo slug — lowercase per phase10d.
    pub repo: String,
    /// Optional topic from the canonical taxonomy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Severity hint (`info` / `notable`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

/// Outbound response shape — mirrors the spec-04 ingestion contract.
#[derive(Debug, Clone, Serialize)]
pub struct IngestResponse {
    /// Server-stamped envelope id (ULID).
    pub event_id: String,
    /// SHA-256 of the body — lets the agent cite a stable handle.
    pub content_hash: String,
    /// RFC-3339 timestamp the envelope was accepted by ingestion.
    pub indexed_at: String,
}

/// Phase10j — capture handler. Validates, builds the envelope, and
/// forwards to `cortex-ingestion`.
pub async fn handle_ingest(
    State(_state): State<ApiState>,
    Json(req): Json<IngestRequest>,
) -> Response {
    // 1. Validate body.
    let body_len = req.body.as_bytes().len();
    if body_len == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "body_empty",
                "message": "body must not be empty",
            })),
        )
            .into_response();
    }
    if body_len > MAX_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": "body_too_large",
                "max_bytes": MAX_BODY_BYTES,
                "received": body_len,
            })),
        )
            .into_response();
    }

    // 2. Validate kind.
    let kind = req.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "memory" | "knowledge" | "learning") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "unsupported_kind",
                "message": "kind must be one of: memory, knowledge, learning",
                "received": req.kind,
            })),
        )
            .into_response();
    }

    // 3. Validate repo.
    let repo = req.repo.trim();
    if repo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "repo_required",
                "message": "repo must be a non-empty lowercase slug",
            })),
        )
            .into_response();
    }
    if repo != repo.to_ascii_lowercase() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "repo_not_lowercase",
                "message": "repo must be lowercase per phase10d",
                "received": repo,
            })),
        )
            .into_response();
    }

    // 4. Build canonical envelope.
    let event_id = cortex_core::event_id();
    let content_hash = sha256_hex(req.body.as_bytes());
    let now = chrono::Utc::now().to_rfc3339();
    let mut context = json!({ "repo": repo });
    if let Some(t) = req.topic.as_deref().filter(|s| !s.is_empty()) {
        context["topic"] = Value::String(t.to_string());
    }
    let mut envelope = json!({
        "event_id": event_id,
        "kind": kind,
        "ts": now,
        "context": context,
        "payload": {
            "body": req.body,
            "content_hash": content_hash.clone(),
            "captured_via": "cortex_capture_memory",
        },
    });
    if let Some(s) = req.severity.as_deref().filter(|s| !s.is_empty()) {
        envelope["severity"] = Value::String(s.to_string());
    }

    // 5. Forward to cortex-ingestion.
    let upstream = std::env::var("CORTEX_INGESTION_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_INGESTION_URL.to_string());
    let url = format!("{}/v1/events", upstream.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "client_build_failed",
                    "message": e.to_string(),
                })),
            )
                .into_response();
        }
    };
    let upstream_resp = match client.post(&url).json(&envelope).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "ingestion_unreachable",
                    "url": url,
                    "message": e.to_string(),
                })),
            )
                .into_response();
        }
    };
    let status = upstream_resp.status();
    let upstream_body: Value = upstream_resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": "ingestion_returned_error",
                "upstream_status": status.as_u16(),
                "upstream_body": upstream_body,
            })),
        )
            .into_response();
    }

    // 6. Return the synchronous receipt.
    let resp = IngestResponse {
        event_id,
        content_hash,
        indexed_at: now,
    };
    (StatusCode::ACCEPTED, Json(resp)).into_response()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn max_body_bytes_is_8kib() {
        assert_eq!(MAX_BODY_BYTES, 8192);
    }
}
