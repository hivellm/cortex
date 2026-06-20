//! Phase19 §2.6 — `GET /v1/consolidations/diff` handler.
//!
//! Returns consolidations created at or after `since_ts` (epoch ms),
//! ordered by ascending timestamp. Drives the "what's new in
//! consolidations since I last polled?" pattern without a fusion
//! round-trip.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec referenced an `accumulated_at` field; the
//! `cortex_consolidations` Meili index settings at
//! `crates/cortex-storage/schemas/meili/cortex_consolidations.settings.v1.json`
//! list `occurred_at` as the canonical sortable + filterable
//! timestamp on every envelope index (`accumulated_at` is not on the
//! schema). The handler therefore sorts on `occurred_at:asc` and
//! filters `ts >= since_ts` — the only timestamp the
//! consolidations doc actually carries. Callers polling for "new
//! consolidations since last cursor" pass the highest `occurred_at`
//! they have seen so far.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default `limit` when omitted.
const LIMIT_DEFAULT: u32 = 50;
/// Maximum `limit` the handler honours.
const LIMIT_MAX: u32 = 200;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Query params for `GET /v1/consolidations/diff`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidationsDiffQuery {
    /// Epoch ms lower bound on `occurred_at`. Required.
    pub since_ts: i64,
    /// Optional repo filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `GET /v1/consolidations/diff`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationsDiffResponse {
    /// Echoed `since_ts` so the caller can confirm the cursor.
    pub since_ts: i64,
    /// Native Meili consolidation documents, sorted by
    /// `occurred_at` ascending so the caller advances the cursor on
    /// the last hit's `occurred_at`.
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

#[allow(clippy::manual_clamp)]
fn clamp_limit(q: &ConsolidationsDiffQuery) -> u32 {
    q.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

/// Build the Meili filter expression. `ts >= since_ts` is
/// always present; `repo = "..."` joins via AND when supplied.
pub(crate) fn build_filter(q: &ConsolidationsDiffQuery) -> String {
    let mut clauses: Vec<String> = vec![format!("ts >= {}", q.since_ts)];
    if let Some(repo) = q.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("repo = \"{}\"", meili_escape(repo)));
    }
    clauses.join(" AND ")
}

/// Handler — `GET /v1/consolidations/diff`.
pub async fn handle_consolidations_diff(
    State(_state): State<ApiState>,
    Query(q): Query<ConsolidationsDiffQuery>,
) -> Response {
    use axum::response::IntoResponse;

    if q.since_ts < 0 {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "since_ts must be a non-negative epoch-ms integer",
        );
    }

    let limit = clamp_limit(&q);
    let filter = build_filter(&q);
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        super::resolve_family_index(
            q.repo.as_deref(),
            "consolidations",
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
    );
    let body = json!({
        "q": "",
        "limit": limit,
        "filter": filter,
        "sort": ["ts:asc"],
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
            return Json(ConsolidationsDiffResponse {
                since_ts: q.since_ts,
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

    Json(ConsolidationsDiffResponse {
        since_ts: q.since_ts,
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(since_ts: i64) -> ConsolidationsDiffQuery {
        ConsolidationsDiffQuery {
            since_ts,
            repo: None,
            limit: None,
        }
    }

    #[test]
    fn build_filter_always_includes_since_ts() {
        let f = build_filter(&q(1_700_000_000_000));
        assert_eq!(f, "ts >= 1700000000000");
    }

    #[test]
    fn build_filter_joins_repo_with_and() {
        let mut p = q(1_700_000_000_000);
        p.repo = Some("cortex".into());
        let f = build_filter(&p);
        assert!(f.contains("ts >= 1700000000000"));
        assert!(f.contains("repo = \"cortex\""));
        assert_eq!(f.matches(" AND ").count(), 1);
    }

    #[test]
    fn build_filter_escapes_double_quotes_in_repo() {
        let mut p = q(0);
        p.repo = Some("evil\"repo".into());
        let f = build_filter(&p);
        assert!(f.contains("repo = \"evil\"\"repo\""));
    }

    #[test]
    fn build_filter_drops_empty_or_whitespace_repo() {
        let mut p = q(1);
        p.repo = Some("   ".into());
        let f = build_filter(&p);
        assert_eq!(f, "ts >= 1");
    }

    #[test]
    fn clamp_limit_floors_at_one_caps_at_two_hundred() {
        let mut p = q(0);
        p.limit = Some(0);
        assert_eq!(clamp_limit(&p), 1);
        p.limit = Some(99_999);
        assert_eq!(clamp_limit(&p), LIMIT_MAX);
        p.limit = None;
        assert_eq!(clamp_limit(&p), LIMIT_DEFAULT);
    }
}
