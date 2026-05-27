//! Phase19 §1.1 — `POST /v1/search/events` handler.
//!
//! Granular envelope listing filtered by `kind` (Turn / ToolCall /
//! Consolidation / Decision / KnowledgeNote / LearningNote /
//! TopicCard / Law / Memory / Analysis). Reads directly from the
//! kind-routed Meili index so the response carries the native
//! envelope shape — no fusion projection.
//!
//! This route exists because the fused `cortex_query` cannot answer
//! "show me every ToolCall envelope in repo X this week" without
//! the host post-filtering through a 12 KB pre-thinking budget.
//! The MCP `cortex_events_by_kind` tool wraps this route 1:1.

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

/// Request body for `POST /v1/search/events`.
#[derive(Debug, Clone, Deserialize)]
pub struct EventsByKindRequest {
    /// Envelope kind discriminator. Mapped to the matching Meili
    /// index via [`kind_to_index`].
    pub kind: String,
    /// Optional repo filter (`source.repo` in the envelope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Optional session filter (`session_id` in the envelope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Lower bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Upper bound on `occurred_at` (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Free-text query (Meili `q`). Empty string returns every doc
    /// matching the filter set up to `limit`.
    #[serde(default)]
    pub q: String,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/search/events`.
#[derive(Debug, Clone, Serialize)]
pub struct EventsByKindResponse {
    /// Echo of the requested kind.
    pub kind: String,
    /// Resolved Meili index uid the handler queried.
    pub index: String,
    /// Raw Meili documents (native envelope shape preserved).
    pub hits: Vec<Value>,
    /// `processingTimeMs` echoed from Meili.
    pub processing_time_ms: u64,
    /// `estimatedTotalHits` echoed from Meili.
    pub estimated_total_hits: u64,
}

/// Map a `kind` discriminator string to the canonical Meili index
/// uid. Returns `None` for unknown kinds so the handler can fail
/// fast with a `bad_input` error.
pub fn kind_to_index(kind: &str) -> Option<&'static str> {
    use cortex_storage::names::*;
    match kind {
        "turn" | "Turn" => Some(INDEX_TURNS),
        "tool_call" | "ToolCall" => Some(INDEX_TOOL_CALLS),
        "consolidation" | "Consolidation" => Some(INDEX_CONSOLIDATIONS),
        "decision" | "Decision" => Some(INDEX_DECISIONS),
        "analysis" | "Analysis" => Some(INDEX_ANALYSES),
        "memory" | "Memory" => Some(INDEX_MEMORIES),
        "law" | "Law" | "violation" | "Violation" => Some(INDEX_LAWS),
        "knowledge" | "KnowledgeNote" | "Knowledge" => Some(INDEX_KNOWLEDGE),
        "learning" | "LearningNote" | "Learning" => Some(INDEX_LEARNINGS),
        "topic_card" | "TopicCard" => Some(INDEX_TOPIC_CARDS),
        _ => None,
    }
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
fn clamp_limit(req: &EventsByKindRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

/// Build the Meili `filter` expression from the request fields.
/// Returns `None` when no constraints apply (Meili rejects an empty
/// filter string).
pub(crate) fn build_filter(req: &EventsByKindRequest) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(repo) = req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Meili filter strings escape via doubled quotes inside the
        // double-quoted literal.
        let esc = repo.replace('"', "\"\"");
        clauses.push(format!("repo = \"{esc}\""));
    }
    if let Some(sid) = req
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let esc = sid.replace('"', "\"\"");
        clauses.push(format!("session_id = \"{esc}\""));
    }
    // `occurred_at` is indexed as a numeric (epoch ms) on every
    // envelope index per the canonical settings — Meili supports
    // range comparators directly.
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

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Handler — `POST /v1/search/events`. Maps `kind` to the matching
/// Meili index and forwards the request with the assembled filter.
pub async fn handle_events_by_kind(
    State(_state): State<ApiState>,
    Json(req): Json<EventsByKindRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let global_index = match kind_to_index(req.kind.trim()) {
        Some(i) => i,
        None => {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!("unknown kind `{}`", req.kind),
            );
        }
    };
    // Route per-repo when caller scopes by repo — live Meili only
    // populates the per-repo `cortex-<slug>-<family>` indexes for
    // most kinds (globals exist only for `cortex_decisions` +
    // `cortex_laws`). Resolve the per-repo family from the kind so
    // the read-side hits the same index the writer wrote to.
    let kind_lower = req.kind.trim().to_ascii_lowercase();
    let family = super::kind_to_family(&kind_lower);
    let index: String = match (
        req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        family,
    ) {
        (Some(r), Some(fam)) => format!("cortex-{}-{}", r.to_ascii_lowercase(), fam),
        _ => global_index.to_string(),
    };

    // If the caller passed a since/until that fails to parse, the
    // filter assembler returns None silently — surface a 400 here so
    // the operator gets a clear error instead of an unfiltered scan.
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
        index.as_str()
    );
    let mut body = json!({ "q": req.q, "limit": limit });
    if let Some(f) = &filter {
        body["filter"] = Value::String(f.clone());
    }
    // Newest first — every envelope index has `ts` in
    // its sortableAttributes set per the canonical settings.
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

    Json(EventsByKindResponse {
        kind: req.kind,
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

    #[test]
    fn kind_to_index_covers_every_canonical_kind() {
        for kind in [
            "turn",
            "tool_call",
            "consolidation",
            "decision",
            "analysis",
            "memory",
            "law",
            "knowledge",
            "learning",
            "topic_card",
        ] {
            assert!(
                kind_to_index(kind).is_some(),
                "lowercase kind `{kind}` must map to an index"
            );
        }
        for pascal in [
            "Turn",
            "ToolCall",
            "Consolidation",
            "Decision",
            "Analysis",
            "Memory",
            "Law",
            "Violation",
            "KnowledgeNote",
            "LearningNote",
            "TopicCard",
        ] {
            assert!(
                kind_to_index(pascal).is_some(),
                "PascalCase kind `{pascal}` must map to an index"
            );
        }
    }

    #[test]
    fn kind_to_index_rejects_unknown_discriminator() {
        assert!(kind_to_index("Unknown").is_none());
        assert!(kind_to_index("").is_none());
        assert!(kind_to_index("session").is_none());
    }

    #[test]
    fn build_filter_returns_none_when_no_constraints() {
        let req = EventsByKindRequest {
            kind: "turn".into(),
            repo: None,
            session_id: None,
            since: None,
            until: None,
            q: String::new(),
            limit: None,
        };
        assert!(build_filter(&req).is_none());
    }

    #[test]
    fn build_filter_assembles_repo_and_session_clauses() {
        let req = EventsByKindRequest {
            kind: "tool_call".into(),
            repo: Some("cortex".into()),
            session_id: Some("01HZ8VYWX9YR2NQH4PFW3K7C50".into()),
            since: None,
            until: None,
            q: String::new(),
            limit: None,
        };
        let f = build_filter(&req).expect("filter expected");
        assert!(f.contains("repo = \"cortex\""));
        assert!(f.contains("session_id = \"01HZ8VYWX9YR2NQH4PFW3K7C50\""));
        assert!(f.contains(" AND "));
    }

    #[test]
    fn build_filter_renders_rfc3339_as_epoch_ms_range() {
        let since_str = "2026-05-26T00:00:00Z";
        let until_str = "2026-05-26T23:59:59Z";
        let since_ms = chrono::DateTime::parse_from_rfc3339(since_str)
            .unwrap()
            .timestamp_millis();
        let until_ms = chrono::DateTime::parse_from_rfc3339(until_str)
            .unwrap()
            .timestamp_millis();
        let req = EventsByKindRequest {
            kind: "turn".into(),
            repo: None,
            session_id: None,
            since: Some(since_str.into()),
            until: Some(until_str.into()),
            q: String::new(),
            limit: None,
        };
        let f = build_filter(&req).expect("filter expected");
        assert!(f.contains(&format!("ts >= {since_ms}")));
        assert!(f.contains(&format!("ts <= {until_ms}")));
    }

    #[test]
    fn build_filter_escapes_embedded_double_quotes_in_repo() {
        let req = EventsByKindRequest {
            kind: "turn".into(),
            repo: Some("repo\"with\"quotes".into()),
            session_id: None,
            since: None,
            until: None,
            q: String::new(),
            limit: None,
        };
        let f = build_filter(&req).expect("filter expected");
        // Meili escapes via doubled quotes inside the literal.
        assert!(f.contains("repo = \"repo\"\"with\"\"quotes\""));
    }

    #[test]
    fn clamp_limit_floors_at_one_and_caps_at_fifty() {
        let mut req = EventsByKindRequest {
            kind: "turn".into(),
            repo: None,
            session_id: None,
            since: None,
            until: None,
            q: String::new(),
            limit: Some(0),
        };
        assert_eq!(clamp_limit(&req), 1);
        req.limit = Some(100_000);
        assert_eq!(clamp_limit(&req), LIMIT_MAX);
        req.limit = None;
        assert_eq!(clamp_limit(&req), LIMIT_DEFAULT);
    }

    #[test]
    fn parse_rfc3339_to_ms_round_trips_known_instant() {
        // Anchor on the chrono crate's own round-trip — derived
        // value avoids tripping on calendar-arithmetic surprises.
        let dt = chrono::DateTime::parse_from_rfc3339("2026-05-26T07:00:00Z").unwrap();
        let expected = dt.timestamp_millis();
        let ms = parse_rfc3339_to_ms("2026-05-26T07:00:00Z").unwrap();
        assert_eq!(ms, expected);
        assert!(parse_rfc3339_to_ms("not-a-timestamp").is_none());
    }
}
