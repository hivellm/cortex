//! Phase19 §2.1 — `GET /v1/consolidations/{id}` handler.
//!
//! Fetch one consolidation by `event_id` (envelope primary key)
//! OR by `consolidation_id` (stable producer-assigned id).
//! Returns the full `ConsolidationPayload` so the caller does not
//! need to dig through `cortex_query` fusion rows to assemble
//! `summary`, `takeaways`, `source_event_ids`, `temporal_span`,
//! `repos`, `tags`, `outcome_distribution`, etc.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Optional `?repo=<slug>` query param so id-only lookups can
/// route to the per-repo `cortex-<slug>-consolidations` index.
/// Live Meili has no global `cortex_consolidations`; without the
/// hint the handler falls back to the global (which today returns
/// 404).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidationGetQuery {
    /// Optional repo scope for the lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Response body for `GET /v1/consolidations/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationGetResponse {
    /// Resolved Meili document — the native consolidation
    /// envelope shape including `ext.consolidation.*`.
    pub document: Value,
    /// `true` when the lookup matched on `consolidation_id`
    /// (re-emitted envelopes share the same consolidation_id);
    /// `false` when it matched on the envelope `event_id`.
    pub matched_consolidation_id: bool,
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

/// Validate the id shape so a malformed input returns a clean
/// 400 instead of forwarding garbage to Meili. ULIDs are 26
/// alphanumeric chars; consolidation producers may also stamp
/// shorter slug-style ids — we accept anything alphanumeric +
/// `_` + `-` up to 128 chars.
pub(crate) fn id_is_valid(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Handler — `GET /v1/consolidations/{id}`.
pub async fn handle_consolidation_get(
    State(_state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<ConsolidationGetQuery>,
) -> Response {
    use axum::response::IntoResponse;

    let id = id.trim().to_string();
    if !id_is_valid(&id) {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "id must be alphanumeric (ULID or slug shape, ≤128 chars)",
        );
    }

    let base = meili_base_url();
    let base = base.trim_end_matches('/');
    let index = super::resolve_family_index(
        q.repo.as_deref(),
        "consolidations",
        cortex_storage::names::INDEX_CONSOLIDATIONS,
    );

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
    let api_key = meili_api_key();

    // Step 1: Meili document GET on the primary key (= envelope event_id).
    // `event_id` is not filterable in per-repo indexes, but it IS the doc
    // primary key, so the cheapest lookup is /indexes/{idx}/documents/{id}.
    let doc_url = format!("{}/indexes/{}/documents/{}", base, index, id);
    let mut doc_req = client.get(&doc_url);
    if let Some(ref key) = api_key {
        doc_req = doc_req.bearer_auth(key);
    }
    match doc_req.send().await {
        Ok(r) if r.status().is_success() => {
            if let Ok(doc) = r.json::<Value>().await {
                return Json(ConsolidationGetResponse {
                    document: doc,
                    matched_consolidation_id: false,
                })
                .into_response();
            }
        }
        Ok(_) => {} // 404 → fall through to consolidation_id filter
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "api_unreachable",
                format!("meili unreachable: {e}"),
            );
        }
    }

    // Step 2: search by `ext.consolidation.consolidation_id` (filterable).
    let url = format!("{}/indexes/{}/search", base, index);
    let filter = format!(
        "ext.consolidation.consolidation_id = \"{}\"",
        meili_escape(&id),
    );
    let body = json!({
        "q": "",
        "limit": 1,
        "filter": filter,
    });
    let mut request = client.post(&url).json(&body);
    if let Some(ref key) = api_key {
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
            .unwrap_or("meili rejected the lookup")
            .to_string();
        return json_err(StatusCode::BAD_GATEWAY, "api_http_error", detail);
    }
    let hit = parsed
        .get("hits")
        .and_then(Value::as_array)
        .and_then(|v| v.first().cloned());
    let Some(doc) = hit else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("no consolidation matches id `{id}`"),
        );
    };
    Json(ConsolidationGetResponse {
        document: doc,
        matched_consolidation_id: true,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_valid_accepts_ulid_shape() {
        assert!(id_is_valid("01HZSESS0000000000000000ZZ"));
        assert!(id_is_valid("consolidation_id_with_underscores"));
        assert!(id_is_valid("slug-style-id"));
        assert!(id_is_valid("01234567890123456789012345"));
    }

    #[test]
    fn id_is_valid_rejects_empty_or_overlong_or_special() {
        assert!(!id_is_valid(""));
        assert!(!id_is_valid("with/slash"));
        assert!(!id_is_valid("with space"));
        assert!(!id_is_valid("with.dot"));
        let too_long = "x".repeat(129);
        assert!(!id_is_valid(&too_long));
    }

    #[test]
    fn meili_escape_doubles_quotes() {
        assert_eq!(meili_escape("plain"), "plain");
        assert_eq!(meili_escape("with\"quote"), "with\"\"quote");
        assert_eq!(meili_escape("\"\""), "\"\"\"\"");
    }
}
