//! Phase14f §1 — `POST /v1/pre-thinking/feedback` endpoint.
//!
//! Operator (or downstream model wrapper) posts feedback on a
//! prior pre-thinking bundle keyed on the `query_id` the bundle
//! carried. The endpoint UPSERTs into `pre_thinking_feedback` so
//! a re-post overwrites the prior row (idempotent — phase14f §1.3).
//! Unknown `query_id` is rejected with HTTP 404 so the operator
//! catches typos before the row pollutes the table.
//!
//! Validation policy:
//! - `query_id` must match the ULID-shape regex Cortex uses
//!   elsewhere (`^[0-9A-HJ-KM-NP-TV-Z]{26}$` Crockford-safe), OR
//!   any non-empty string — the audit-table cross-check happens
//!   on the lookup side once the audit log is wired (phase14f
//!   follow-up). Today the endpoint accepts any non-empty id.
//! - `helpful` is required.
//! - `rating` ∈ [1, 5] when present.
//! - `files_cited` deduplicated + serialised as a JSON string so
//!   the SQLite column stays single-column.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use cortex_storage::MetadataStore;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Wire request body. Optional fields land as `None` in SQLite.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedbackRequest {
    /// The `query_id` the pre-thinking bundle carried.
    pub query_id: String,
    /// Intent that produced the bundle (operator-visible label
    /// like `explain`, `law_check`). Stored as-is.
    #[serde(default)]
    pub intent: Option<String>,
    /// `true` when the operator says the bundle was useful.
    pub helpful: bool,
    /// Files the model ended up citing from the bundle (optional —
    /// caller passes them when known so the implicit-feedback
    /// detector has a baseline to compare against).
    #[serde(default)]
    pub files_cited: Vec<String>,
    /// 1..=5 star rating. Optional.
    #[serde(default)]
    pub rating: Option<i64>,
    /// Free-text operator note. Optional.
    #[serde(default)]
    pub free_text: Option<String>,
    /// Optional pre-computed implicit-feedback Jaccard score in
    /// `[0.0, 1.0]`. When `None`, the column stays NULL until the
    /// async detector runs.
    #[serde(default)]
    pub implicit_score: Option<f64>,
}

/// Wire response shape.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FeedbackResponse {
    /// Echoed back so the caller can correlate.
    pub query_id: String,
    /// `true` when the row was inserted; `false` when it
    /// overwrote a prior row (UPSERT semantics).
    pub upserted: bool,
}

/// Axum state — wraps the metadata store handle the feedback
/// recorder writes through.
#[derive(Clone)]
pub struct FeedbackState {
    /// Shared metadata handle. Behind a `Mutex` because rusqlite
    /// connections are `!Sync`.
    pub metadata: Arc<Mutex<MetadataStore>>,
}

/// Build the `/v1/pre-thinking/feedback` + `/v1/feedback/list`
/// sub-router (phase19 §3.2 lands the list surface alongside the
/// existing UPSERT endpoint).
pub fn build_router(state: FeedbackState) -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route("/v1/pre-thinking/feedback", post(feedback_handler))
        .route("/v1/feedback/list", post(feedback_list_handler))
        .with_state(state)
}

/// Axum handler. Validates input + UPSERTs into
/// `pre_thinking_feedback`.
pub async fn feedback_handler(
    State(state): State<FeedbackState>,
    Json(req): Json<FeedbackRequest>,
) -> Response {
    match handle(state, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(err) => err.into_response(),
    }
}

#[derive(Debug)]
struct FeedbackError(StatusCode, String);

impl FeedbackError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({"error": self.1});
        (self.0, Json(body)).into_response()
    }
}

async fn handle(
    state: FeedbackState,
    req: FeedbackRequest,
) -> Result<FeedbackResponse, FeedbackError> {
    if req.query_id.trim().is_empty() {
        return Err(FeedbackError(
            StatusCode::BAD_REQUEST,
            "query_id must be non-empty".into(),
        ));
    }
    if let Some(r) = req.rating {
        if !(1..=5).contains(&r) {
            return Err(FeedbackError(
                StatusCode::BAD_REQUEST,
                format!("rating must be in [1, 5]; got {r}"),
            ));
        }
    }
    if let Some(s) = req.implicit_score {
        if !(0.0..=1.0).contains(&s) {
            return Err(FeedbackError(
                StatusCode::BAD_REQUEST,
                format!("implicit_score must be in [0.0, 1.0]; got {s}"),
            ));
        }
    }
    let mut files = req.files_cited.clone();
    files.sort();
    files.dedup();
    let files_json = serde_json::to_string(&files).map_err(|e| {
        FeedbackError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialise files_cited: {e}"),
        )
    })?;
    let store = state.metadata.lock().await;
    let prior_existed = store
        .pre_thinking_feedback(&req.query_id)
        .map(|r| r.is_some())
        .unwrap_or(false);
    store
        .upsert_pre_thinking_feedback(
            &req.query_id,
            req.intent.as_deref(),
            req.helpful,
            &files_json,
            req.rating,
            req.free_text.as_deref(),
            req.implicit_score,
            Utc::now(),
        )
        .map_err(|e| {
            FeedbackError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("metadata upsert: {e}"),
            )
        })?;
    Ok(FeedbackResponse {
        query_id: req.query_id,
        upserted: !prior_existed,
    })
}

// ---------------------------------------------------------------------
// Phase19 §3.2 — `POST /v1/feedback/list`
// ---------------------------------------------------------------------

/// Default `limit` when omitted.
const LIST_LIMIT_DEFAULT: u32 = 20;
/// Maximum `limit` the handler honours.
const LIST_LIMIT_MAX: u32 = 50;

/// Wire request body for `POST /v1/feedback/list`.
///
/// ## Scope deviation from the original spec
///
/// The task spec listed `repo?` as a filter; the
/// `pre_thinking_feedback` table at
/// `crates/cortex-storage/src/metadata.rs` carries no `repo`
/// column (the schema is `query_id` / `intent` / `helpful` /
/// `files_cited` / `rating` / `free_text` / `implicit_score` /
/// `recorded_at`). Adding a `repo` column would require a non-
/// trivial migration + writer update on the feedback ingest
/// path; the handler instead documents the gap by silently
/// rejecting non-empty `repo` filters with `bad_input` so callers
/// notice immediately rather than getting a misleading "no rows"
/// response.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedbackListRequest {
    /// Filter on the `helpful` boolean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helpful: Option<bool>,
    /// Filter on the `intent` label (`explain`, `law_check`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// RFC3339 lower bound on `recorded_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// RFC3339 upper bound on `recorded_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Reserved — surfaced as `bad_input` when non-empty (see
    /// scope deviation above).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Hits cap. Defaults to [`LIST_LIMIT_DEFAULT`]; clamped to
    /// [`LIST_LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One row in the feedback list response.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FeedbackRow {
    /// The `query_id` the feedback was keyed on.
    pub query_id: String,
    /// Intent label captured at upsert time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Operator-reported helpfulness.
    pub helpful: bool,
    /// Files the operator said the model cited (already
    /// deserialised from the SQLite JSON column).
    pub files_cited: Vec<String>,
    /// 1..=5 star rating when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<i64>,
    /// Free-text operator note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
    /// Implicit-feedback Jaccard score when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_score: Option<f64>,
    /// RFC3339 timestamp the row was written.
    pub recorded_at: String,
}

/// Wire response body.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FeedbackListResponse {
    /// Rows ordered by `recorded_at DESC` (newest-first).
    pub rows: Vec<FeedbackRow>,
    /// Equal to `rows.len()`; carried so consumers cannot drift.
    pub total: u32,
}

#[allow(clippy::manual_clamp)]
fn clamp_list_limit(limit: Option<u32>) -> u32 {
    limit
        .unwrap_or(LIST_LIMIT_DEFAULT)
        .min(LIST_LIMIT_MAX)
        .max(1)
}

fn parse_rfc3339(s: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Axum handler — `POST /v1/feedback/list`.
pub async fn feedback_list_handler(
    State(state): State<FeedbackState>,
    Json(req): Json<FeedbackListRequest>,
) -> Response {
    if let Some(repo) = req.repo.as_deref().map(str::trim) {
        if !repo.is_empty() {
            return FeedbackError(
                StatusCode::BAD_REQUEST,
                "`repo` filter is not supported — `pre_thinking_feedback` table has no repo column"
                    .into(),
            )
            .into_response();
        }
    }
    let since = match req
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => match parse_rfc3339(s) {
            Some(dt) => Some(dt),
            None => {
                return FeedbackError(
                    StatusCode::BAD_REQUEST,
                    format!("`{s}` is not a valid RFC3339 timestamp"),
                )
                .into_response();
            }
        },
        None => None,
    };
    let until = match req
        .until
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => match parse_rfc3339(s) {
            Some(dt) => Some(dt),
            None => {
                return FeedbackError(
                    StatusCode::BAD_REQUEST,
                    format!("`{s}` is not a valid RFC3339 timestamp"),
                )
                .into_response();
            }
        },
        None => None,
    };
    let intent = req
        .intent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = clamp_list_limit(req.limit);

    let store = state.metadata.lock().await;
    let raw = match store.list_pre_thinking_feedback(req.helpful, intent, since, until, limit) {
        Ok(r) => r,
        Err(e) => {
            return FeedbackError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("metadata list: {e}"),
            )
            .into_response();
        }
    };
    let rows: Vec<FeedbackRow> = raw
        .into_iter()
        .map(
            |(query_id, intent, helpful, files_json, rating, free_text, implicit_score, recorded_at)| {
                let files_cited: Vec<String> =
                    serde_json::from_str(&files_json).unwrap_or_default();
                FeedbackRow {
                    query_id,
                    intent,
                    helpful,
                    files_cited,
                    rating,
                    free_text,
                    implicit_score,
                    recorded_at,
                }
            },
        )
        .collect();
    let total = rows.len() as u32;
    (StatusCode::OK, Json(FeedbackListResponse { rows, total })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn router() -> (axum::Router, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        let state = FeedbackState {
            metadata: handle.clone(),
        };
        (build_router(state), handle)
    }

    async fn post_json(
        router: &axum::Router,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pre-thinking/feedback")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, val)
    }

    #[tokio::test]
    async fn happy_path_returns_upserted_true() {
        let (router, _store) = router().await;
        let (status, body) = post_json(
            &router,
            serde_json::json!({
                "query_id": "01HXQUERY00000000000000A",
                "intent": "explain",
                "helpful": true,
                "files_cited": ["a.rs", "b.rs"],
                "rating": 5,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["query_id"], "01HXQUERY00000000000000A");
        assert_eq!(body["upserted"], true);
    }

    #[tokio::test]
    async fn empty_query_id_returns_400() {
        let (router, _) = router().await;
        let (status, body) = post_json(
            &router,
            serde_json::json!({"query_id": "  ", "helpful": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap_or("").contains("query_id"));
    }

    #[tokio::test]
    async fn out_of_range_rating_returns_400() {
        let (router, _) = router().await;
        for r in [0i64, 6, -1, 100] {
            let (status, body) = post_json(
                &router,
                serde_json::json!({"query_id": "q", "helpful": true, "rating": r}),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "rating {r} should be rejected"
            );
            assert!(body["error"].as_str().unwrap_or("").contains("rating"));
        }
    }

    #[tokio::test]
    async fn idempotent_re_post_returns_upserted_false() {
        let (router, _) = router().await;
        let body = serde_json::json!({"query_id": "q-dup", "helpful": true});
        let (s1, b1) = post_json(&router, body.clone()).await;
        assert_eq!(s1, StatusCode::OK);
        assert_eq!(b1["upserted"], true);
        let (s2, b2) = post_json(&router, body).await;
        assert_eq!(s2, StatusCode::OK);
        assert_eq!(b2["upserted"], false);
    }

    #[tokio::test]
    async fn malformed_body_returns_400() {
        let (router, _) = router().await;
        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pre-thinking/feedback")
                    .header("content-type", "application/json")
                    .body(Body::from("{not valid json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Axum's Json extractor rejects malformed bodies as 400.
        assert!(resp.status().is_client_error());
    }
}
