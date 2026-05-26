//! Phase19 §3.3 — `POST /v1/decisions/search` handler.
//!
//! List Decision envelopes from the global `cortex_decisions` Meili
//! index filtered by status / tag / window. Returns the native
//! envelope shape so callers see `decision_id` / `decision_title` /
//! `decision_status` / `decision_supersedes` without going through
//! `cortex_query` fusion.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec listed `{status?, supersedes?, superseded_by?,
//! tag?, limit<=50}`. The `cortex_decisions` schema at
//! `crates/cortex-storage/schemas/meili/cortex_decisions.settings.v1.json`
//! declares only `status` / `topics` / `repo` / `occurred_at` as
//! filterable; `decision_supersedes` is stamped on the document by
//! the writer but NOT declared filterable on this global index, and
//! there is no `superseded_by` field at all (the reverse relation
//! is graph-only — chase it via `cortex_decision_chain`). Adding
//! either filter would require a schema bump + reindex.
//!
//! The handler exposes the subset that the schema can honour:
//! `status`, `tag` (mapped to `topics`), `repo`, and an RFC3339
//! `since`/`until` window. `supersedes` and `superseded_by` are
//! rejected with `bad_input` when supplied so callers notice the
//! gap immediately rather than getting a misleading empty-rows
//! response. Free-text `q` is also accepted so callers can match
//! ADR titles / bodies. Sort hardcoded `occurred_at:desc` to
//! match the schema's ranking rule.

use axum::{extract::State, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default hits cap when caller omits `limit`.
const LIMIT_DEFAULT: u32 = 20;
/// Maximum hits the handler returns in a single call.
const LIMIT_MAX: u32 = 50;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Canonical status vocab the ADR lifecycle uses. Mirrors the
/// `proposed` → `accepted` → `superseded`/`deprecated` chain
/// documented in `AGENTS.md` and `crates/cortex-workers`.
const STATUSES_ALLOWED: &[&str] = &["proposed", "accepted", "superseded", "deprecated"];

/// Request body for `POST /v1/decisions/search`.
#[derive(Debug, Clone, Deserialize)]
pub struct DecisionSearchRequest {
    /// Free-text query against title / body / topics / repo.
    #[serde(default)]
    pub q: String,
    /// Optional status filter (`proposed` / `accepted` /
    /// `superseded` / `deprecated`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Optional tag filter — mapped to the `topics` filterable
    /// array (the schema declares `topics` filterable; ADR tags
    /// flow through the classifier into that vocab).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional repo filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Reserved — surfaced as `bad_input` when non-empty (see
    /// scope deviation in the module docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// Reserved — surfaced as `bad_input` when non-empty (see
    /// scope deviation in the module docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// RFC3339 lower bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// RFC3339 upper bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/decisions/search`.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionSearchResponse {
    /// Native Meili Decision documents.
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

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[allow(clippy::manual_clamp)]
fn clamp_limit(req: &DecisionSearchRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

/// Validate the status discriminator against the ADR lifecycle vocab.
pub(crate) fn status_is_valid(status: &str) -> bool {
    STATUSES_ALLOWED.contains(&status)
}

/// Build the Meili filter expression. Returns `None` when no
/// constraints apply.
pub(crate) fn build_filter(req: &DecisionSearchRequest) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(s) = req
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!("status = \"{}\"", meili_escape(s)));
    }
    if let Some(t) = req.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("topics = \"{}\"", meili_escape(t)));
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
        clauses.push(format!("occurred_at >= {ts_ms}"));
    }
    if let Some(until) = req
        .until
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ts_ms = parse_rfc3339_to_ms(until)?;
        clauses.push(format!("occurred_at <= {ts_ms}"));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Handler — `POST /v1/decisions/search`.
pub async fn handle_decision_search(
    State(_state): State<ApiState>,
    Json(req): Json<DecisionSearchRequest>,
) -> Response {
    use axum::response::IntoResponse;

    if let Some(v) = req
        .supersedes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!(
                "`supersedes` filter unsupported (not declared filterable on `cortex_decisions`); pivot via `cortex_decision_chain` for `{v}`"
            ),
        );
    }
    if let Some(v) = req
        .superseded_by
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!(
                "`superseded_by` filter unsupported (no such field on the schema); pivot via `cortex_decision_chain` for `{v}`"
            ),
        );
    }
    if let Some(s) = req
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !status_is_valid(s) {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!(
                    "unknown status `{s}`; allowed: {}",
                    STATUSES_ALLOWED.join(", ")
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
        cortex_storage::names::INDEX_DECISIONS
    );
    let mut body = json!({ "q": req.q, "limit": limit });
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

    Json(DecisionSearchResponse {
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> DecisionSearchRequest {
        DecisionSearchRequest {
            q: String::new(),
            status: None,
            tag: None,
            repo: None,
            supersedes: None,
            superseded_by: None,
            since: None,
            until: None,
            limit: None,
        }
    }

    #[test]
    fn status_is_valid_covers_adr_lifecycle() {
        for s in ["proposed", "accepted", "superseded", "deprecated"] {
            assert!(status_is_valid(s));
        }
        assert!(!status_is_valid("Accepted"));
        assert!(!status_is_valid("approved"));
        assert!(!status_is_valid(""));
    }

    #[test]
    fn build_filter_returns_none_for_empty_request() {
        assert!(build_filter(&req()).is_none());
    }

    #[test]
    fn build_filter_joins_status_tag_repo_with_and() {
        let mut r = req();
        r.status = Some("accepted".into());
        r.tag = Some("retention".into());
        r.repo = Some("cortex".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("status = \"accepted\""));
        assert!(f.contains("topics = \"retention\""));
        assert!(f.contains("repo = \"cortex\""));
        assert_eq!(f.matches(" AND ").count(), 2);
    }

    #[test]
    fn build_filter_renders_since_until_as_epoch_ms_on_occurred_at() {
        let since_str = "2026-05-01T00:00:00Z";
        let until_str = "2026-05-26T00:00:00Z";
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
        assert!(f.contains(&format!("occurred_at >= {since_ms}")));
        assert!(f.contains(&format!("occurred_at <= {until_ms}")));
    }

    #[test]
    fn build_filter_escapes_double_quotes_in_string_literals() {
        let mut r = req();
        r.tag = Some("evil\"tag".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("topics = \"evil\"\"tag\""));
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
