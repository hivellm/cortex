//! Phase19 §2.3 — `POST /v1/consolidations/by-entity` handler.
//!
//! Lists consolidation envelopes that reference an entity (file path,
//! function name, decision id, repo, or model). Reads the
//! `cortex_consolidations` Meili index.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec ("reads `extras.entities` + `extras.relations` via
//! Meili filter") assumed the consolidation envelope's classifier
//! entities were stamped on the Meili document as filterable
//! attributes. They are NOT: the classifier-extracted entities ship to
//! Nexus (graph mapper) and the `cortex_consolidations` index settings
//! at `crates/cortex-storage/schemas/meili/cortex_consolidations.settings.v1.json`
//! list only `grain` / `depth` / `scope_kind` / `scope_value` / `repo`
//! / `topics` / `occurred_at` / `model` as filterable. Reusing the
//! classifier entity list would require a writer-side projection AND
//! a schema bump AND a full reindex of historical consolidations,
//! which is out of scope for this phase.
//!
//! Routing per `entity.kind` against fields the schema already
//! exposes:
//!
//! | kind          | strategy                                       |
//! |---------------|------------------------------------------------|
//! | `repo`        | `filter = repo = "value"`                      |
//! | `model`       | `filter = model = "value"`                     |
//! | `decision_id` | `q = "value"` over title / summary_markdown / topics |
//! | `file`        | `q = "value"` (path indexed inside summary)   |
//! | `function`    | `q = "value"` (name indexed inside summary)   |
//!
//! Sort is hardcoded `occurred_at:desc` so callers always see newest-
//! first, matching `cortex_consolidations_recent`.

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

/// Controlled vocab for `entity.kind`. Routing per kind is documented
/// at the module level.
const KINDS_ALLOWED: &[&str] = &["file", "function", "decision_id", "repo", "model"];

/// Entity descriptor inside the request body.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityRef {
    /// One of `file` / `function` / `decision_id` / `repo` / `model`.
    pub kind: String,
    /// Identifier — path, function name, ADR id, repo name, model id.
    pub value: String,
}

/// Request body for `POST /v1/consolidations/by-entity`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsolidationsByEntityRequest {
    /// Entity to search by.
    pub entity: EntityRef,
    /// Hits cap. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response body for `POST /v1/consolidations/by-entity`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationsByEntityResponse {
    /// Native Meili consolidation documents.
    pub hits: Vec<Value>,
    /// `processingTimeMs` echoed from Meili.
    pub processing_time_ms: u64,
    /// `estimatedTotalHits` echoed from Meili.
    pub estimated_total_hits: u64,
    /// Strategy the handler used (`filter` for repo/model, `q` for the
    /// keyword-search fallback kinds). Surfaced so callers can tell
    /// how authoritative the match is.
    pub match_strategy: &'static str,
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
fn clamp_limit(req: &ConsolidationsByEntityRequest) -> u32 {
    req.limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn meili_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Validate the entity kind against the controlled vocab.
pub(crate) fn kind_is_valid(kind: &str) -> bool {
    KINDS_ALLOWED.contains(&kind)
}

/// What the handler asks Meili for given an entity. Either a filter
/// clause (authoritative match on a top-level Meili field) or a
/// free-text query (keyword fallback for kinds without a dedicated
/// projection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchPlan {
    /// Authoritative filter clause; `q` is empty.
    Filter(String),
    /// Keyword fallback; `filter` is `None`.
    KeywordQuery(String),
}

/// Build the Meili request shape from the entity. Caller still
/// validates `kind_is_valid` before invoking this.
pub(crate) fn plan_search(entity: &EntityRef) -> SearchPlan {
    let value = entity.value.trim();
    match entity.kind.as_str() {
        "repo" => SearchPlan::Filter(format!("repo = \"{}\"", meili_escape(value))),
        "model" => SearchPlan::Filter(format!("model = \"{}\"", meili_escape(value))),
        // `decision_id` / `file` / `function`: no top-level Meili
        // field on the consolidations index → fall back to Meili
        // keyword search across the searchable attrs (title,
        // summary_markdown, topics, repo). The raw value is passed
        // through verbatim so users can pin to ADR ids
        // (`DEC-0042`), file paths (`crates/cortex-api/src/lib.rs`),
        // or function names.
        _ => SearchPlan::KeywordQuery(value.to_string()),
    }
}

/// Handler — `POST /v1/consolidations/by-entity`. Routes the request
/// per [`plan_search`] and returns the native Meili documents.
pub async fn handle_consolidations_by_entity(
    State(_state): State<ApiState>,
    Json(req): Json<ConsolidationsByEntityRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let kind = req.entity.kind.trim();
    if !kind_is_valid(kind) {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!(
                "unknown entity kind `{kind}`; allowed: {}",
                KINDS_ALLOWED.join(", ")
            ),
        );
    }
    if req.entity.value.trim().is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "entity.value must not be empty",
        );
    }

    let limit = clamp_limit(&req);
    let plan = plan_search(&req.entity);
    let (q, filter, strategy): (&str, Option<String>, &'static str) = match &plan {
        SearchPlan::Filter(f) => ("", Some(f.clone()), "filter"),
        SearchPlan::KeywordQuery(query) => (query.as_str(), None, "q"),
    };

    // Route per-repo when the entity carries a repo discriminator
    // (live Meili only has the per-repo `cortex-<slug>-consolidations`
    // family — the global `cortex_consolidations` index does not
    // exist today).
    let repo_for_index: Option<&str> = if req.entity.kind == "repo" {
        Some(req.entity.value.trim()).filter(|s| !s.is_empty())
    } else {
        None
    };
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        super::resolve_family_index(
            repo_for_index,
            "consolidations",
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
    );
    let mut body = json!({ "q": q, "limit": limit });
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

    Json(ConsolidationsByEntityResponse {
        hits,
        processing_time_ms,
        estimated_total_hits,
        match_strategy: strategy,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(kind: &str, value: &str) -> EntityRef {
        EntityRef {
            kind: kind.into(),
            value: value.into(),
        }
    }

    #[test]
    fn kind_is_valid_covers_controlled_vocab() {
        for k in ["file", "function", "decision_id", "repo", "model"] {
            assert!(kind_is_valid(k), "kind `{k}` must be allowed");
        }
        assert!(!kind_is_valid(""));
        assert!(!kind_is_valid("Repo"));
        assert!(!kind_is_valid("path"));
    }

    #[test]
    fn plan_search_filters_authoritative_kinds() {
        match plan_search(&ent("repo", "cortex")) {
            SearchPlan::Filter(f) => assert_eq!(f, "repo = \"cortex\""),
            _ => panic!("repo must use filter"),
        }
        match plan_search(&ent("model", "claude-opus-4-7")) {
            SearchPlan::Filter(f) => assert_eq!(f, "model = \"claude-opus-4-7\""),
            _ => panic!("model must use filter"),
        }
    }

    #[test]
    fn plan_search_falls_back_to_keyword_for_decision_file_function() {
        for k in ["decision_id", "file", "function"] {
            match plan_search(&ent(k, "DEC-0042")) {
                SearchPlan::KeywordQuery(q) => assert_eq!(q, "DEC-0042"),
                _ => panic!("kind `{k}` must fall back to keyword search"),
            }
        }
    }

    #[test]
    fn plan_search_escapes_double_quotes_in_filter_value() {
        match plan_search(&ent("repo", "evil\"name")) {
            SearchPlan::Filter(f) => assert_eq!(f, "repo = \"evil\"\"name\""),
            _ => panic!("repo must use filter"),
        }
    }

    #[test]
    fn plan_search_trims_whitespace_in_value() {
        match plan_search(&ent("decision_id", "  DEC-0042  ")) {
            SearchPlan::KeywordQuery(q) => assert_eq!(q, "DEC-0042"),
            _ => panic!("kind decision_id must fall back to keyword"),
        }
    }

    #[test]
    fn clamp_limit_floors_at_one_caps_at_thirty() {
        let mut req = ConsolidationsByEntityRequest {
            entity: ent("repo", "cortex"),
            limit: Some(0),
        };
        assert_eq!(clamp_limit(&req), 1);
        req.limit = Some(99_999);
        assert_eq!(clamp_limit(&req), LIMIT_MAX);
        req.limit = None;
        assert_eq!(clamp_limit(&req), LIMIT_DEFAULT);
    }
}
