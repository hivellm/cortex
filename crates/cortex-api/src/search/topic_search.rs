//! Phase19 §1.5 — `POST /v1/topic-cards/search` handler.
//!
//! Lists every TopicCard whose `topics` array contains a value
//! matching the requested prefix (e.g. `tool:`, `kind:Bash`,
//! `repo:cortex`). Reads the `cortex_topic_cards` Meili index
//! directly — no fusion, no per-kind cross-mixing.
//!
//! Topic vocabulary is controlled by the classifier worker and
//! the consolidator. The TopicCard index has `topics` listed as
//! filterable, so we use a Meili `IN` filter when the caller
//! passes a literal `topic_id` and a `STARTS WITH` fallback when
//! it's a prefix. The `q` free-text search complements the
//! topic filter against the card's title / body / synthesis.

use axum::{extract::State, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Hits cap when caller does not specify `limit`.
const LIMIT_DEFAULT: u32 = 15;
/// Maximum hits the handler returns in a single call.
const LIMIT_MAX: u32 = 30;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Request body for `POST /v1/topic-cards/search`.
#[derive(Debug, Clone, Deserialize)]
pub struct TopicSearchRequest {
    /// Topic prefix or literal topic value (`tool:claude-code`,
    /// `repo:cortex`, `kind:Bash`). Required.
    pub topic_prefix: String,
    /// Optional repo filter on the card itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Free-text Meili query (matches title / body / synthesis_markdown).
    #[serde(default)]
    pub q: String,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/topic-cards/search`.
#[derive(Debug, Clone, Serialize)]
pub struct TopicSearchResponse {
    /// Echo of the requested prefix so the caller can correlate
    /// across multiple calls.
    pub topic_prefix: String,
    /// Raw Meili documents (native TopicCard shape).
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
fn clamp_limit(req: &TopicSearchRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Build the Meili filter expression. Uses `IN` for literal topics
/// (no trailing colon or wildcard) and `topic_id =` when the
/// caller passed a precise topic identifier.
pub(crate) fn build_filter(req: &TopicSearchRequest) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    let prefix = req.topic_prefix.trim();
    if !prefix.is_empty() {
        let esc = meili_escape(prefix);
        // Meili lacks STARTS WITH on arrays; the indexer stamps
        // the canonical topic strings (`tool:claude-code` etc) so
        // an `IN` match covers literal lookups. Prefix-only queries
        // (`tool:`) match the `topics` array contains.
        if prefix.ends_with(':') {
            // Prefix-style ask — there is no STARTS_WITH on Meili
            // arrays, so we surface this as the most reasonable
            // fallback: match against the bare prefix tag the
            // classifier sometimes stamps alone (e.g. `tool:` →
            // `tool:claude-code`). The caller drives the precision
            // via `q`.
            clauses.push(format!("topics = \"{}\"", esc.trim_end_matches(':')));
        } else {
            clauses.push(format!("topics = \"{esc}\""));
        }
    }
    if let Some(repo) = req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("repo = \"{}\"", meili_escape(repo)));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Handler — `POST /v1/topic-cards/search`. Reads from the
/// `cortex_topic_cards` Meili index with the assembled filter.
pub async fn handle_topic_search(
    State(_state): State<ApiState>,
    Json(req): Json<TopicSearchRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let prefix = req.topic_prefix.trim();
    if prefix.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "`topic_prefix` is required",
        );
    }

    let limit = clamp_limit(&req);
    let filter = build_filter(&req);
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        cortex_storage::names::INDEX_TOPIC_CARDS
    );
    let mut body = json!({ "q": req.q, "limit": limit });
    if let Some(f) = &filter {
        body["filter"] = Value::String(f.clone());
    }
    body["sort"] = json!(["updated_at:desc"]);

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

    Json(TopicSearchResponse {
        topic_prefix: req.topic_prefix,
        hits,
        processing_time_ms,
        estimated_total_hits,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(prefix: &str) -> TopicSearchRequest {
        TopicSearchRequest {
            topic_prefix: prefix.into(),
            repo: None,
            q: String::new(),
            limit: None,
        }
    }

    #[test]
    fn build_filter_matches_literal_topic() {
        let f = build_filter(&req("tool:claude-code")).expect("filter");
        assert!(f.contains("topics = \"tool:claude-code\""));
    }

    #[test]
    fn build_filter_strips_trailing_colon_on_prefix_ask() {
        let f = build_filter(&req("tool:")).expect("filter");
        assert!(f.contains("topics = \"tool\""));
    }

    #[test]
    fn build_filter_adds_repo_clause_when_set() {
        let mut r = req("kind:Bash");
        r.repo = Some("cortex".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("topics = \"kind:Bash\""));
        assert!(f.contains("repo = \"cortex\""));
        assert!(f.contains(" AND "));
    }

    #[test]
    fn build_filter_returns_none_when_only_whitespace() {
        assert!(build_filter(&req("   ")).is_none());
    }

    #[test]
    fn build_filter_escapes_double_quotes() {
        let f = build_filter(&req("evil\"topic")).expect("filter");
        assert!(f.contains("topics = \"evil\"\"topic\""));
    }

    #[test]
    fn clamp_limit_floors_at_one_and_caps_at_thirty() {
        let mut r = req("tool:x");
        r.limit = Some(0);
        assert_eq!(clamp_limit(&r), 1);
        r.limit = Some(10_000);
        assert_eq!(clamp_limit(&r), LIMIT_MAX);
        r.limit = None;
        assert_eq!(clamp_limit(&r), LIMIT_DEFAULT);
    }
}
