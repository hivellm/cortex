//! Phase19 §2.2 — `GET /v1/consolidations/recent` handler.
//!
//! Chronological feed of consolidation envelopes filterable by `repo`,
//! `grain` (one of `session` / `topic` / `decision_trace`), and an
//! `occurred_at` window (`since` / `until` as RFC3339). Drives the
//! dashboard's Consolidations view without a `cortex_query` fusion
//! round-trip — returns the native Meili document so callers see the
//! full `ext.consolidation.*` envelope shape.
//!
//! Filter coverage matches the index settings at
//! `crates/cortex-storage/schemas/meili/cortex_consolidations.settings.v1.json`:
//! `grain` / `repo` / `occurred_at` are filterable; `occurred_at` is
//! sortable. The handler always sorts `occurred_at:desc` so the
//! "recent" semantics match the route name.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Hits cap when caller does not specify `limit`.
const LIMIT_DEFAULT: u32 = 15;
/// Maximum hits the handler returns in a single call.
const LIMIT_MAX: u32 = 30;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Canonical grain discriminator vocab. Matches the writer-side
/// projection in `crates/cortex-workers/src/fulltext/builders.rs`
/// (`apply_extensions` → `ext.consolidation.grain`) and the producer
/// enum `ConsolidationGrain { Session, Topic, DecisionTrace }`.
const GRAINS_ALLOWED: &[&str] = &["session", "topic", "decision_trace"];

/// Query params for `GET /v1/consolidations/recent`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidationsRecentQuery {
    /// Optional repo filter (`repo` field on the consolidation doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Optional grain filter — one of `session` / `topic` /
    /// `decision_trace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    /// Lower bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Upper bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `GET /v1/consolidations/recent`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationsRecentResponse {
    /// Raw Meili documents (native consolidation envelope shape).
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
fn clamp_limit(q: &ConsolidationsRecentQuery) -> u32 {
    q.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Validate the grain discriminator against the producer enum.
pub(crate) fn grain_is_valid(grain: &str) -> bool {
    GRAINS_ALLOWED.contains(&grain)
}

/// Build the Meili filter expression from the request fields.
/// Returns `None` when no constraints apply.
pub(crate) fn build_filter(q: &ConsolidationsRecentQuery) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(repo) = q.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("repo = \"{}\"", meili_escape(repo)));
    }
    if let Some(grain) = q.grain.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("grain = \"{}\"", meili_escape(grain)));
    }
    if let Some(since) = q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let ts_ms = parse_rfc3339_to_ms(since)?;
        clauses.push(format!("occurred_at >= {ts_ms}"));
    }
    if let Some(until) = q.until.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let ts_ms = parse_rfc3339_to_ms(until)?;
        clauses.push(format!("occurred_at <= {ts_ms}"));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Handler — `GET /v1/consolidations/recent`. Reads the
/// `cortex_consolidations` Meili index sorted `occurred_at:desc` with
/// the assembled `(repo, grain, since, until)` filter set.
pub async fn handle_consolidations_recent(
    State(_state): State<ApiState>,
    Query(q): Query<ConsolidationsRecentQuery>,
) -> Response {
    use axum::response::IntoResponse;

    if let Some(g) = q.grain.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !grain_is_valid(g) {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!(
                    "unknown grain `{g}`; allowed: {}",
                    GRAINS_ALLOWED.join(", ")
                ),
            );
        }
    }
    for ts in [q.since.as_deref(), q.until.as_deref()]
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

    let limit = clamp_limit(&q);
    let filter = build_filter(&q);
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        cortex_storage::names::INDEX_CONSOLIDATIONS
    );
    let mut body = json!({ "q": "", "limit": limit });
    if let Some(f) = &filter {
        body["filter"] = Value::String(f.clone());
    }
    body["sort"] = json!(["occurred_at:desc"]);

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

    Json(ConsolidationsRecentResponse {
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> ConsolidationsRecentQuery {
        ConsolidationsRecentQuery::default()
    }

    #[test]
    fn grain_is_valid_covers_producer_vocab() {
        for g in ["session", "topic", "decision_trace"] {
            assert!(grain_is_valid(g), "grain `{g}` must be allowed");
        }
        assert!(!grain_is_valid("Session"));
        assert!(!grain_is_valid(""));
        assert!(!grain_is_valid("decisiontrace"));
    }

    #[test]
    fn build_filter_returns_none_for_empty_query() {
        assert!(build_filter(&q()).is_none());
    }

    #[test]
    fn build_filter_assembles_repo_and_grain_with_and_join() {
        let mut p = q();
        p.repo = Some("cortex".into());
        p.grain = Some("topic".into());
        let f = build_filter(&p).expect("filter");
        assert!(f.contains("repo = \"cortex\""));
        assert!(f.contains("grain = \"topic\""));
        assert_eq!(f.matches(" AND ").count(), 1);
    }

    #[test]
    fn build_filter_renders_since_until_as_epoch_ms_on_occurred_at() {
        let since_str = "2026-05-01T00:00:00Z";
        let until_str = "2026-05-26T23:59:59Z";
        let since_ms = chrono::DateTime::parse_from_rfc3339(since_str)
            .unwrap()
            .timestamp_millis();
        let until_ms = chrono::DateTime::parse_from_rfc3339(until_str)
            .unwrap()
            .timestamp_millis();
        let mut p = q();
        p.since = Some(since_str.into());
        p.until = Some(until_str.into());
        let f = build_filter(&p).expect("filter");
        assert!(f.contains(&format!("occurred_at >= {since_ms}")));
        assert!(f.contains(&format!("occurred_at <= {until_ms}")));
    }

    #[test]
    fn build_filter_escapes_double_quotes_in_repo() {
        let mut p = q();
        p.repo = Some("evil\"repo".into());
        let f = build_filter(&p).expect("filter");
        assert!(f.contains("repo = \"evil\"\"repo\""));
    }

    #[test]
    fn clamp_limit_floors_at_one_caps_at_thirty() {
        let mut p = q();
        p.limit = Some(0);
        assert_eq!(clamp_limit(&p), 1);
        p.limit = Some(99_999);
        assert_eq!(clamp_limit(&p), LIMIT_MAX);
        p.limit = None;
        assert_eq!(clamp_limit(&p), LIMIT_DEFAULT);
    }
}
