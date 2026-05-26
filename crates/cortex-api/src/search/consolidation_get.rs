//! Phase19 §2.1 — `GET /v1/consolidations/{id}` handler.
//!
//! Fetch one consolidation by `event_id` (envelope primary key)
//! OR by `consolidation_id` (stable producer-assigned id).
//! Returns the full `ConsolidationPayload` so the caller does not
//! need to dig through `cortex_query` fusion rows to assemble
//! `summary`, `takeaways`, `source_event_ids`, `temporal_span`,
//! `repos`, `tags`, `outcome_distribution`, etc.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

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

    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        cortex_storage::names::INDEX_CONSOLIDATIONS
    );
    // Try the consolidation_id filter first (stable producer id);
    // fall back to event_id on miss so re-emitted envelopes still
    // resolve. The handler does one Meili call with an OR filter
    // so the round-trip cost is bounded.
    let filter = format!(
        "event_id = \"{}\" OR ext.consolidation.consolidation_id = \"{}\"",
        meili_escape(&id),
        meili_escape(&id),
    );
    let body = json!({
        "q": "",
        "limit": 1,
        "filter": filter,
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
    let matched_consolidation_id = doc
        .get("ext")
        .and_then(|e| e.get("consolidation"))
        .and_then(|c| c.get("consolidation_id"))
        .and_then(Value::as_str)
        .map(|cid| cid == id)
        .unwrap_or(false);
    Json(ConsolidationGetResponse {
        document: doc,
        matched_consolidation_id,
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
