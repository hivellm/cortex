//! Phase19 §3.1 — `POST /v1/laws/violations` handler.
//!
//! List LawViolation envelopes scoped by repo with optional
//! per-session / per-law / time-window narrowing. Reads the per-repo
//! `cortex-<slug>-governance` Meili index (the canonical home of
//! LawViolation envelopes per
//! `crates/cortex-workers/src/fulltext/routing.rs` —
//! `Kind::LawViolation => "governance"`).
//!
//! ## Scope deviation from the original spec
//!
//! The task spec listed `repo?` (optional). The handler instead
//! REQUIRES `repo` because LawViolation envelopes only live on the
//! per-repo `governance` index — the global `cortex_laws` schema at
//! `crates/cortex-storage/schemas/meili/cortex_laws.settings.v1.json`
//! does not declare `repo` / `session_id` / `ts` as filterable
//! (`severity` + `applies_to` only), so a cross-repo scan cannot
//! honour the per-call filters this tool advertises. Callers needing
//! cross-repo aggregation should fan out one call per repo (the
//! index list is bounded by the workspace).
//!
//! Filter coverage matches the per-repo settings at
//! `crates/cortex-workers/settings/settings.v1.json`: `repo` /
//! `session_id` / `law_id` / `kind` / `ts` are all filterable. The
//! handler always pins `kind = "law_violation"` so the response
//! never leaks law-definition docs (which share the family route).

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

/// Request body for `POST /v1/laws/violations`.
#[derive(Debug, Clone, Deserialize)]
pub struct LawViolationsRequest {
    /// Repo slug — REQUIRED (see module-level scope deviation).
    pub repo: String,
    /// Optional session filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional law id filter (`LAW-CORTEX-001`, `LAW-007`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub law_id: Option<String>,
    /// Lower bound on `ts` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Upper bound on `ts` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/laws/violations`.
#[derive(Debug, Clone, Serialize)]
pub struct LawViolationsResponse {
    /// Echo of the index the handler queried so callers can confirm
    /// which per-repo family the rows came from.
    pub index: String,
    /// Native Meili documents (LawViolation envelope shape preserved).
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
fn clamp_limit(req: &LawViolationsRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

/// Resolve the per-repo governance index uid from a repo slug.
/// Matches the per-repo family naming convention used by
/// `cortex-workers` (`cortex-<lower-slug>-governance`).
pub(crate) fn governance_index(repo: &str) -> String {
    format!("cortex-{}-governance", repo.to_ascii_lowercase())
}

/// Sanity-check the repo slug — reject empty / whitespace / shapes
/// that would break the per-repo index uid.
pub(crate) fn repo_is_valid(repo: &str) -> bool {
    let trimmed = repo.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return false;
    }
    // Meili index uids accept `[A-Za-z0-9_-]`; we lowercase before
    // use, so allow letters + digits + `-` + `_` here.
    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Build the Meili filter expression. `kind = "law_violation"` is
/// always present so law-definition docs that share the family
/// route never leak.
pub(crate) fn build_filter(req: &LawViolationsRequest) -> Option<String> {
    let mut clauses: Vec<String> = vec!["kind = \"law_violation\"".to_string()];
    if let Some(sid) = req
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!("session_id = \"{}\"", meili_escape(sid)));
    }
    if let Some(lid) = req
        .law_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!("law_id = \"{}\"", meili_escape(lid)));
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
    Some(clauses.join(" AND "))
}

/// Handler — `POST /v1/laws/violations`.
pub async fn handle_law_violations(
    State(_state): State<ApiState>,
    Json(req): Json<LawViolationsRequest>,
) -> Response {
    use axum::response::IntoResponse;

    if !repo_is_valid(&req.repo) {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "`repo` is required and must be alphanumeric (`-` / `_` allowed, ≤128 chars)",
        );
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

    let index = governance_index(&req.repo);
    let limit = clamp_limit(&req);
    let filter = match build_filter(&req) {
        Some(f) => f,
        None => "kind = \"law_violation\"".to_string(),
    };

    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        index
    );
    let body = json!({
        "q": "",
        "limit": limit,
        "filter": filter,
        "sort": ["ts:desc"],
    });

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
        // Missing Meili index → empty 200, not 502 (issue #4 Bug1).
        if super::is_meili_index_missing(status.as_u16(), &parsed) {
            return Json(LawViolationsResponse {
                index,
                hits: Vec::new(),
                processing_time_ms: 0,
                estimated_total_hits: 0,
            })
            .into_response();
        }
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

    Json(LawViolationsResponse {
        index,
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(repo: &str) -> LawViolationsRequest {
        LawViolationsRequest {
            repo: repo.into(),
            session_id: None,
            law_id: None,
            since: None,
            until: None,
            limit: None,
        }
    }

    #[test]
    fn governance_index_lowercases_slug() {
        assert_eq!(governance_index("Cortex"), "cortex-cortex-governance");
        assert_eq!(governance_index("nexus"), "cortex-nexus-governance");
    }

    #[test]
    fn repo_is_valid_accepts_alphanum_dash_underscore_rejects_special() {
        assert!(repo_is_valid("cortex"));
        assert!(repo_is_valid("Cortex_x-2"));
        assert!(!repo_is_valid(""));
        assert!(!repo_is_valid("  "));
        assert!(!repo_is_valid("bad/slash"));
        assert!(!repo_is_valid("bad space"));
        assert!(!repo_is_valid(&"x".repeat(129)));
    }

    #[test]
    fn build_filter_always_pins_kind_law_violation() {
        let f = build_filter(&req("cortex")).expect("filter");
        assert!(f.contains("kind = \"law_violation\""));
    }

    #[test]
    fn build_filter_joins_session_id_and_law_id_with_and() {
        let mut r = req("cortex");
        r.session_id = Some("01HSESS01".into());
        r.law_id = Some("LAW-007".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("session_id = \"01HSESS01\""));
        assert!(f.contains("law_id = \"LAW-007\""));
        assert!(f.contains("kind = \"law_violation\""));
        assert_eq!(f.matches(" AND ").count(), 2);
    }

    #[test]
    fn build_filter_renders_since_until_as_epoch_ms_on_ts() {
        let since_str = "2026-05-01T00:00:00Z";
        let until_str = "2026-05-26T00:00:00Z";
        let since_ms = chrono::DateTime::parse_from_rfc3339(since_str)
            .unwrap()
            .timestamp_millis();
        let until_ms = chrono::DateTime::parse_from_rfc3339(until_str)
            .unwrap()
            .timestamp_millis();
        let mut r = req("cortex");
        r.since = Some(since_str.into());
        r.until = Some(until_str.into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains(&format!("ts >= {since_ms}")));
        assert!(f.contains(&format!("ts <= {until_ms}")));
    }

    #[test]
    fn build_filter_escapes_double_quotes_in_string_literals() {
        let mut r = req("cortex");
        r.law_id = Some("evil\"law".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("law_id = \"evil\"\"law\""));
    }

    #[test]
    fn clamp_limit_floors_at_one_caps_at_fifty() {
        let mut r = req("cortex");
        r.limit = Some(0);
        assert_eq!(clamp_limit(&r), 1);
        r.limit = Some(99_999);
        assert_eq!(clamp_limit(&r), LIMIT_MAX);
        r.limit = None;
        assert_eq!(clamp_limit(&r), LIMIT_DEFAULT);
    }
}
