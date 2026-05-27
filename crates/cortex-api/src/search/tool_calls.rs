//! Phase19 §1.3 — `POST /v1/search/tool-calls` handler.
//!
//! Specialised version of `events_by_kind` for the
//! `cortex_tool_calls` Meili index. Adds tool-call-specific
//! filters (`tool_name`, `outcome`) on top of the shared
//! repo / time / free-text shape.
//!
//! Filter coverage matches the index settings at
//! `crates/cortex-storage/schemas/meili/cortex_tool_calls.settings.v1.json`:
//! `tool_name` / `outcome` / `repo` / `occurred_at` / `topics` are
//! filterable. `session_id` is NOT — callers wanting per-session
//! tool-call lists should pivot through `cortex_session_timeline`
//! with `kind=tool_call` instead.

use axum::{extract::State, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Hits cap when caller does not specify `limit`.
const LIMIT_DEFAULT: u32 = 20;
/// Maximum hits the handler returns in a single call.
const LIMIT_MAX: u32 = 50;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Outcome discriminator the worker stamps on every ToolCall
/// document. Matches the writer at
/// `crates/cortex-workers/src/fulltext/worker.rs` lines 497-564.
const OUTCOMES_ALLOWED: &[&str] = &["ok", "transient", "rejected", "task_failed", "error"];

/// Request body for `POST /v1/search/tool-calls`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallsRequest {
    /// Optional `tool_name` filter (e.g. `Bash`, `Read`, `Edit`).
    /// Matches the literal value the adapter stamped on the
    /// envelope's payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Optional outcome filter — one of `ok` / `transient` /
    /// `rejected` / `task_failed` / `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Optional repo filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Lower bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Upper bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Free-text Meili query. Empty string returns every match.
    #[serde(default)]
    pub q: String,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/search/tool-calls`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallsResponse {
    /// Raw Meili documents (native ToolCall shape preserved).
    pub hits: Vec<Value>,
    /// `processingTimeMs` echoed from Meili.
    pub processing_time_ms: u64,
    /// `estimatedTotalHits` echoed from Meili.
    pub estimated_total_hits: u64,
}

fn meili_base_url() -> String {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_url)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| MEILI_URL_DEFAULT.to_string())
}

fn meili_api_key() -> Option<String> {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_api_key)
        .filter(|s| !s.trim().is_empty())
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
fn clamp_limit(req: &ToolCallsRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Build the Meili filter expression from the request fields.
/// Returns `None` when no constraints apply.
pub(crate) fn build_filter(req: &ToolCallsRequest) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(tn) = req
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!("tool_name = \"{}\"", meili_escape(tn)));
    }
    if let Some(out) = req
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!("outcome = \"{}\"", meili_escape(out)));
    }
    if let Some(repo) = req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("repo = \"{}\"", meili_escape(repo)));
    }
    if let Some(since) = req
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ts_ms = parse_rfc3339_to_ms(since)?;
        clauses.push(format!("ts >= {ts_ms}"));
    }
    if let Some(until) = req
        .until
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ts_ms = parse_rfc3339_to_ms(until)?;
        clauses.push(format!("ts <= {ts_ms}"));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Validate the outcome discriminator against the writer's vocab.
pub(crate) fn outcome_is_valid(outcome: &str) -> bool {
    OUTCOMES_ALLOWED.contains(&outcome)
}

/// Handler — `POST /v1/search/tool-calls`. Forwards the request
/// to the `cortex_tool_calls` Meili index with the assembled
/// tool-call filter set.
pub async fn handle_tool_calls(
    State(_state): State<ApiState>,
    Json(req): Json<ToolCallsRequest>,
) -> Response {
    use axum::response::IntoResponse;

    if let Some(out) = req
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !outcome_is_valid(out) {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!(
                    "unknown outcome `{out}`; allowed: {}",
                    OUTCOMES_ALLOWED.join(", ")
                ),
            );
        }
    }
    for ts in [req.since.as_deref(), req.until.as_deref()]
        .into_iter()
        .flatten()
    {
        let trimmed = ts.trim();
        if !trimmed.is_empty() && parse_rfc3339_to_ms(trimmed).is_none() {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!("`{ts}` is not a valid RFC3339 timestamp"),
            );
        }
    }

    let limit = clamp_limit(&req);
    let filter = build_filter(&req);
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        // Live Meili: tool_call envelopes route to the per-repo
        // `cortex-<slug>-code` family (per
        // `cortex_workers::graph::routing::family_for`), NOT a
        // dedicated `cortex_tool_calls` global. Resolve per-repo
        // when `repo` is set and fall back to the global only when
        // no repo is supplied (will 404 today but stays
        // forward-compatible).
        super::resolve_family_index(
            req.repo.as_deref(),
            "code",
            cortex_storage::names::INDEX_TOOL_CALLS,
        )
    );
    let mut body = json!({ "q": req.q, "limit": limit });
    if let Some(f) = &filter {
        body["filter"] = Value::String(f.clone());
    }
    body["sort"] = json!(["ts:desc"]);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("build http client: {e}"),
            );
        }
    };
    let mut request = client.post(&url).json(&body);
    if let Some(key) = meili_api_key() {
        request = request.bearer_auth(key);
    }
    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "api_unreachable",
                format!("meili unreachable: {e}"),
            );
        }
    };
    let status = resp.status();
    let parsed = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "api_http_error",
                format!("meili body parse: {e}"),
            );
        }
    };
    if !status.is_success() {
        let detail = parsed
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("meili rejected the search")
            .to_string();
        return json_err(StatusCode::BAD_GATEWAY, "api_http_error", detail);
    }

    let hits = parsed
        .get("hits")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let processing_time_ms = parsed
        .get("processingTimeMs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let estimated_total_hits = parsed
        .get("estimatedTotalHits")
        .and_then(Value::as_u64)
        .unwrap_or(hits.len() as u64);

    Json(ToolCallsResponse {
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ToolCallsRequest {
        ToolCallsRequest {
            tool_name: None,
            outcome: None,
            repo: None,
            since: None,
            until: None,
            q: String::new(),
            limit: None,
        }
    }

    #[test]
    fn outcome_is_valid_covers_writer_vocab() {
        for o in ["ok", "transient", "rejected", "task_failed", "error"] {
            assert!(outcome_is_valid(o), "writer outcome `{o}` must be allowed");
        }
        assert!(!outcome_is_valid("success"));
        assert!(!outcome_is_valid(""));
    }

    #[test]
    fn build_filter_returns_none_for_empty_request() {
        assert!(build_filter(&req()).is_none());
    }

    #[test]
    fn build_filter_assembles_tool_name_outcome_repo() {
        let mut r = req();
        r.tool_name = Some("Bash".into());
        r.outcome = Some("error".into());
        r.repo = Some("cortex".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("tool_name = \"Bash\""));
        assert!(f.contains("outcome = \"error\""));
        assert!(f.contains("repo = \"cortex\""));
        // AND-joined.
        assert_eq!(f.matches(" AND ").count(), 2);
    }

    #[test]
    fn build_filter_renders_since_until_as_epoch_ms() {
        let since_str = "2026-05-26T00:00:00Z";
        let until_str = "2026-05-26T23:59:59Z";
        let since_ms = chrono::DateTime::parse_from_rfc3339(since_str)
            .unwrap()
            .timestamp_millis();
        let until_ms = chrono::DateTime::parse_from_rfc3339(until_str)
            .unwrap()
            .timestamp_millis();
        let mut r = req();
        r.since = Some(since_str.into());
        r.until = Some(until_str.into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains(&format!("ts >= {since_ms}")));
        assert!(f.contains(&format!("ts <= {until_ms}")));
    }

    #[test]
    fn build_filter_escapes_double_quotes() {
        let mut r = req();
        r.tool_name = Some("evil\"name".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("tool_name = \"evil\"\"name\""));
    }

    #[test]
    fn clamp_limit_floors_at_one_caps_at_fifty() {
        let mut r = req();
        r.limit = Some(0);
        assert_eq!(clamp_limit(&r), 1);
        r.limit = Some(99_999);
        assert_eq!(clamp_limit(&r), LIMIT_MAX);
        r.limit = None;
        assert_eq!(clamp_limit(&r), LIMIT_DEFAULT);
    }
}
