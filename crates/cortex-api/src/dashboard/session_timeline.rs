//! Phase19 §1.2 — `GET /v1/sessions/{session_id}/timeline` handler.
//!
//! Returns every envelope captured for one `session_id`, ordered by
//! `occurred_at asc`. The host (or the `cortex_session_timeline` MCP
//! tool) uses this to rebuild "what happened in session X" without
//! a full archive walk or a `cortex_query` fusion call.
//!
//! Reads from the on-disk Parquet archive (`<archive_root>/events/`)
//! via `cortex_storage::archive::scan_envelopes_by_session`. The
//! scan is O(archive_size), so the handler clamps to `limit <= 200`
//! events per call to keep the response under the MCP transport
//! ceiling; callers paginate by re-issuing with a tighter
//! `since`/`until` window on the next request.

use std::path::PathBuf;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default cap when caller does not specify `limit`.
const LIMIT_DEFAULT: u32 = 100;
/// Hard ceiling on events the handler returns in one call.
const LIMIT_MAX: u32 = 200;

/// Query params for the route.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionTimelineQuery {
    /// Cap on emitted rows. Defaults to [`LIMIT_DEFAULT`]; clamped
    /// to [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional kind filter (`turn` / `tool_call` / ...). Empty
    /// returns every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// One row in the timeline projection.
#[derive(Debug, Clone, Serialize)]
pub struct SessionTimelineRow {
    /// RFC3339 timestamp.
    pub ts: String,
    /// Envelope kind discriminator (snake_case: `turn`, `tool_call`, ...).
    pub kind: String,
    /// Short title or summary for the row. Truncated to 240 bytes.
    pub title: String,
    /// Source envelope id so the host can pivot to `cortex_audit`.
    pub event_id: String,
    /// Per-kind delta blob carrying envelope-specific extras
    /// (`tool_name` for tool_call, `model` for turn, ...).
    pub deltas: Value,
}

/// Response body for the route.
#[derive(Debug, Clone, Serialize)]
pub struct SessionTimelineResponse {
    /// Echo of the requested session.
    pub session_id: String,
    /// Number of rows the handler emitted.
    pub count: u32,
    /// `true` when the underlying scan returned more than `limit`
    /// rows and the result is clipped — the host can request a
    /// narrower window to see the rest.
    pub truncated: bool,
    /// Timeline rows, ordered by `ts asc`.
    pub events: Vec<SessionTimelineRow>,
}

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

#[allow(clippy::manual_clamp)]
fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn truncate_title(s: &str) -> String {
    // 240 bytes guard so the JSON response fits even when every
    // event carries a long summary. UTF-8 safe — never splits a
    // multi-byte codepoint.
    if s.len() <= 240 {
        return s.to_string();
    }
    let mut idx = 240;
    while !s.is_char_boundary(idx) && idx > 0 {
        idx -= 1;
    }
    format!("{}…", &s[..idx])
}

/// Project an `Envelope` to its `SessionTimelineRow` shape. Kept
/// `pub(crate)` so unit tests can drive it directly without
/// touching the archive filesystem.
pub(crate) fn build_row(env: &cortex_core::events::Envelope) -> SessionTimelineRow {
    let title = title_for(env);
    let deltas = deltas_for(env);
    SessionTimelineRow {
        ts: env.occurred_at.clone(),
        kind: env.kind.schema_stem().to_string(),
        title: truncate_title(&title),
        event_id: env.event_id.clone(),
        deltas,
    }
}

fn title_for(env: &cortex_core::events::Envelope) -> String {
    // Best-effort summary derivation — each kind has a different
    // canonical "title" field. The host should treat this as a
    // human-friendly preview, not a load-bearing identifier.
    let payload = &env.payload;
    if let Some(s) = payload.get("user_message").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(s) = payload.get("summary").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(s) = payload.get("title").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(s) = payload.get("tool").and_then(Value::as_str) {
        return format!("tool: {s}");
    }
    if let Some(s) = payload.get("tool_name").and_then(Value::as_str) {
        return format!("tool: {s}");
    }
    String::new()
}

fn deltas_for(env: &cortex_core::events::Envelope) -> Value {
    let payload = &env.payload;
    let mut out = serde_json::Map::new();
    for key in [
        "tool",
        "tool_name",
        "exit_code",
        "model",
        "grain",
        "decision_id",
        "status",
        "topic_id",
        "law_id",
        "severity",
        "parent_event_id",
    ] {
        if let Some(v) = payload.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

/// Resolve the archive root from the typed Config snapshot the
/// daemon carries on `ApiState`. Falls back to
/// `<HOME|USERPROFILE>/.cortex/archive` when nothing is configured —
/// matches the convention every other archive consumer (ingestion
/// router, doctor, archive_loader) uses.
fn archive_root(state: &ApiState) -> PathBuf {
    if let Some(root) = state.cfg.ingestion.archive_root.as_deref() {
        if !root.trim().is_empty() {
            return PathBuf::from(root);
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cortex").join("archive")
}

/// Handler — `GET /v1/sessions/{session_id}/timeline?limit=N&kind=K`.
pub async fn handle_session_timeline(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Query(params): Query<SessionTimelineQuery>,
) -> Response {
    use axum::response::IntoResponse;

    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "session_id path segment is required",
        );
    }

    let limit = clamp_limit(params.limit);
    let kind_filter = params
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let root = archive_root(&state);
    let session_for_blocking = session_id.clone();
    let scan = tokio::task::spawn_blocking(move || {
        cortex_storage::archive::scan_envelopes_by_session(&root, &session_for_blocking)
    })
    .await;
    let mut envs = match scan {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("archive scan: {e}"),
            );
        }
        Err(join_err) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("blocking task panic: {join_err}"),
            );
        }
    };
    envs.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at));

    if let Some(kf) = kind_filter {
        envs.retain(|e| build_row(e).kind == kf);
    }

    let total = envs.len() as u32;
    let truncated = total > limit;
    let events: Vec<SessionTimelineRow> = envs.iter().take(limit as usize).map(build_row).collect();
    let count = events.len() as u32;
    Json(SessionTimelineResponse {
        session_id,
        count,
        truncated,
        events,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Envelope, Kind, Stream};

    fn mk_context() -> Context {
        Context {
            repo: None,
            branch: None,
            commit: None,
            cwd: None,
            user: None,
            platform: "linux".into(),
            ide: None,
            extras: Default::default(),
        }
    }

    fn mk_env(kind: Kind, ts: &str, payload: Value) -> Envelope {
        Envelope {
            schema_version: "1".into(),
            event_id: "01HZ00000000000000000000ZZ".into(),
            kind,
            tool: "claude-code".into(),
            stream: Stream::Live,
            model: None,
            session_id: "01HZSESS0000000000000000ZZ".into(),
            occurred_at: ts.to_string(),
            ingested_at: None,
            content_hash: "sha256:0".into(),
            context: mk_context(),
            payload,
            redactions: Vec::new(),
            parent_event_id: None,
        }
    }

    #[test]
    fn clamp_limit_floors_at_one_and_caps_at_max() {
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(10_000)), LIMIT_MAX);
        assert_eq!(clamp_limit(None), LIMIT_DEFAULT);
        assert_eq!(clamp_limit(Some(50)), 50);
    }

    #[test]
    fn truncate_title_respects_utf8_boundaries() {
        let s = "x".repeat(241);
        let t = truncate_title(&s);
        assert!(t.len() <= 241 + 3, "truncated must add at most an ellipsis");
        assert!(t.ends_with('…'));
        let short = "ok".to_string();
        assert_eq!(truncate_title(&short), "ok");
    }

    #[test]
    fn build_row_prefers_user_message_then_summary_then_title() {
        let env = mk_env(
            Kind::Turn,
            "2026-05-26T07:00:00Z",
            json!({"user_message": "hello", "summary": "ignored"}),
        );
        let row = build_row(&env);
        assert_eq!(row.kind, "turn");
        assert_eq!(row.title, "hello");
        assert_eq!(row.ts, env.occurred_at);

        let env2 = mk_env(
            Kind::Consolidation,
            "2026-05-26T07:05:00Z",
            json!({"summary": "weekly digest"}),
        );
        assert_eq!(build_row(&env2).title, "weekly digest");

        let env3 = mk_env(
            Kind::ToolCall,
            "2026-05-26T07:10:00Z",
            json!({"tool_name": "Bash", "exit_code": 0}),
        );
        let row3 = build_row(&env3);
        assert_eq!(row3.title, "tool: Bash");
        assert_eq!(row3.deltas["tool_name"], "Bash");
        assert_eq!(row3.deltas["exit_code"], 0);
    }

    #[test]
    fn build_row_maps_every_kind_to_snake_case() {
        for (kind, expected) in [
            (Kind::Turn, "turn"),
            (Kind::ToolCall, "tool_call"),
            (Kind::AgentCall, "agent_call"),
            (Kind::Consolidation, "consolidation"),
            (Kind::Decision, "decision"),
            (Kind::Analysis, "analysis"),
            (Kind::Memory, "memory"),
            (Kind::LawViolation, "law_violation"),
            (Kind::Knowledge, "knowledge"),
            (Kind::Learning, "learning"),
            (Kind::TopicCard, "topic_card"),
            (Kind::Artifact, "artifact"),
        ] {
            let env = mk_env(kind, "2026-05-26T07:00:00Z", json!({}));
            assert_eq!(build_row(&env).kind, expected);
        }
    }

    #[test]
    fn deltas_for_collects_only_known_keys_no_payload_leak() {
        let env = mk_env(
            Kind::ToolCall,
            "2026-05-26T07:00:00Z",
            json!({
                "tool_name": "Read",
                "exit_code": 0,
                "secret_password": "leaked"
            }),
        );
        let row = build_row(&env);
        assert!(row.deltas.get("tool_name").is_some());
        assert!(
            row.deltas.get("secret_password").is_none(),
            "deltas allow-list must not echo unknown payload fields"
        );
    }
}
