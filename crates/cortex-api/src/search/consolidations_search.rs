//! Phase19 §2.4 — `POST /v1/consolidations/search` handler.
//!
//! Text-driven search restricted to the `cortex_consolidations`
//! Meili index. Returns the native consolidation envelope shape so
//! callers see `title` / `summary_markdown` / `topics` / `ext.consolidation.*`
//! without going through a `cortex_query` fusion projection.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec ("hybrid BM25 + vector + RRF scoped to
//! `cortex_consolidations` only") assumed direct reach into a
//! `query_text → embedding → vector lane` pipeline. The other phase19
//! handlers all follow the "Meili-direct" pattern with no embedder
//! dependency; the `/v1/search/vector` endpoint today rejects
//! `query_text` with `not_implemented` because server-side embedding
//! lands behind the orchestrator's `QueryService`, not the search
//! proxy. Routing the vector lane through `QueryService` would
//! require either:
//!
//! 1. Adding a `kinds` filter to [`crate::types::Scope`] so
//!    `cortex_query` can be scoped to `kind=consolidation`, OR
//! 2. Threading a `VectorLane` handle into [`ApiState`] alongside
//!    `service` so this handler can call it directly.
//!
//! Both are larger structural changes than this phase's scope. The
//! handler instead does a BM25-only Meili search against the
//! consolidations index and surfaces `match_strategy = "bm25"` on
//! the response so callers can tell which lane actually ran. The
//! field-set / wire shape is forward-compatible: a follow-up that
//! adds the vector lane will only need to flip the strategy label
//! and stitch `rrf_fuse` over the two ranked vectors — no caller
//! migration needed.
//!
//! Filter coverage uses the canonical filterable attrs from
//! `cortex_consolidations.settings.v1.json` (`repo`, `grain`,
//! `model`, `occurred_at`). `intent_hint` is accepted + echoed for
//! future RRF lane-weight tuning but does not affect BM25 ranking
//! today.

use axum::{extract::State, http::StatusCode, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default `k` when omitted.
const K_DEFAULT: u32 = 10;
/// Max `k` per the task contract.
const K_MAX: u32 = 20;
/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";

/// Intent hint vocab. Mirrors [`crate::types::Intent`] labels so a
/// future RRF wiring can map each value to a lane-weight preset.
const INTENT_HINTS_ALLOWED: &[&str] = &[
    "pre_change_context",
    "decision_lookup",
    "similar_problems",
    "law_check",
    "free_search",
    "explain",
];

/// Grain vocab the producer enum stamps on the envelope.
const GRAINS_ALLOWED: &[&str] = &["session", "topic", "decision_trace"];

/// Request body for `POST /v1/consolidations/search`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsolidationsSearchRequest {
    /// Free-text query. Required.
    pub query: String,
    /// Hits cap. Defaults to [`K_DEFAULT`]; clamped to [`K_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<u32>,
    /// Optional intent label — echoed on the response for caller
    /// telemetry; reserved for future RRF lane-weight tuning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_hint: Option<String>,
    /// Optional repo filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Optional grain filter (`session` / `topic` / `decision_trace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
}

/// Response body for `POST /v1/consolidations/search`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationsSearchResponse {
    /// Native Meili consolidation documents, sorted by Meili
    /// relevance (`words` → `typo` → `proximity` → `attribute` →
    /// `sort` → `exactness` → `occurred_at:desc` per the index
    /// settings ranking rules).
    pub hits: Vec<Value>,
    /// `processingTimeMs` echoed from Meili.
    pub processing_time_ms: u64,
    /// `estimatedTotalHits` echoed from Meili.
    pub estimated_total_hits: u64,
    /// Always `"bm25"` today. Reserved value `"hybrid_rrf"` will
    /// land when the vector lane is wired through.
    pub match_strategy: &'static str,
    /// Echo of the intent hint the caller passed (or `null`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_hint: Option<String>,
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
fn clamp_k(req: &ConsolidationsSearchRequest) -> u32 {
    req.k.unwrap_or(K_DEFAULT).min(K_MAX).max(1)
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Validate the intent label against [`crate::types::Intent`].
pub(crate) fn intent_hint_is_valid(intent: &str) -> bool {
    INTENT_HINTS_ALLOWED.contains(&intent)
}

/// Validate the grain discriminator against the producer enum.
pub(crate) fn grain_is_valid(grain: &str) -> bool {
    GRAINS_ALLOWED.contains(&grain)
}

/// Build the Meili filter expression from the request fields.
/// Returns `None` when no constraints apply.
pub(crate) fn build_filter(req: &ConsolidationsSearchRequest) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(repo) = req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        clauses.push(format!("repo = \"{}\"", meili_escape(repo)));
    }
    if let Some(grain) = req
        .grain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        clauses.push(format!(
            "ext.consolidation.grain = \"{}\"",
            meili_escape(grain)
        ));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// Handler — `POST /v1/consolidations/search`.
pub async fn handle_consolidations_search(
    State(_state): State<ApiState>,
    Json(req): Json<ConsolidationsSearchRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let query = req.query.trim();
    if query.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "bad_input", "`query` is required");
    }
    if let Some(g) = req
        .grain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
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
    if let Some(h) = req
        .intent_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !intent_hint_is_valid(h) {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!(
                    "unknown intent_hint `{h}`; allowed: {}",
                    INTENT_HINTS_ALLOWED.join(", ")
                ),
            );
        }
    }

    let k = clamp_k(&req);
    let filter = build_filter(&req);

    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        super::resolve_family_index(
            req.repo.as_deref(),
            "consolidations",
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
    );
    let mut body = json!({ "q": query, "limit": k });
    if let Some(f) = &filter {
        body["filter"] = Value::String(f.clone());
    }

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

    Json(ConsolidationsSearchResponse {
        hits,
        processing_time_ms,
        estimated_total_hits,
        match_strategy: "bm25",
        intent_hint: req.intent_hint,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(query: &str) -> ConsolidationsSearchRequest {
        ConsolidationsSearchRequest {
            query: query.into(),
            k: None,
            intent_hint: None,
            repo: None,
            grain: None,
        }
    }

    #[test]
    fn intent_hint_is_valid_covers_intent_labels() {
        for h in [
            "pre_change_context",
            "decision_lookup",
            "similar_problems",
            "law_check",
            "free_search",
            "explain",
        ] {
            assert!(intent_hint_is_valid(h), "label `{h}` must be allowed");
        }
        assert!(!intent_hint_is_valid("decisions"));
        assert!(!intent_hint_is_valid(""));
    }

    #[test]
    fn grain_is_valid_covers_producer_vocab() {
        for g in ["session", "topic", "decision_trace"] {
            assert!(grain_is_valid(g));
        }
        assert!(!grain_is_valid("Session"));
    }

    #[test]
    fn build_filter_returns_none_for_query_only() {
        assert!(build_filter(&req("retention")).is_none());
    }

    #[test]
    fn build_filter_joins_repo_and_grain_with_and() {
        let mut r = req("retention");
        r.repo = Some("cortex".into());
        r.grain = Some("session".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("repo = \"cortex\""));
        assert!(f.contains("ext.consolidation.grain = \"session\""));
        assert_eq!(f.matches(" AND ").count(), 1);
    }

    #[test]
    fn build_filter_escapes_double_quotes_in_repo() {
        let mut r = req("x");
        r.repo = Some("evil\"repo".into());
        let f = build_filter(&r).expect("filter");
        assert!(f.contains("repo = \"evil\"\"repo\""));
    }

    #[test]
    fn clamp_k_floors_at_one_caps_at_twenty() {
        let mut r = req("x");
        r.k = Some(0);
        assert_eq!(clamp_k(&r), 1);
        r.k = Some(99_999);
        assert_eq!(clamp_k(&r), K_MAX);
        r.k = None;
        assert_eq!(clamp_k(&r), K_DEFAULT);
    }
}
