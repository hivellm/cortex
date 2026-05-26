//! phase13g §2 — `cortex_similar_sessions` consolidation vector
//! search.
//!
//! Vector search restricted to the `cortex-<repo>-consolidations`
//! Vectorizer collection. Returns top-K [`ConsolidationHit`] rows
//! with a confidence floor (default 0.6) so the agent gets
//! deterministic grounding on past sessions that already tackled
//! the same intent.
//!
//! ADR-014 pure-reader contract: the handler returns a typed
//! [`SimilarSessionsResponse`] verbatim from a
//! [`ConsolidationSearchSource`] implementation. Live wiring builds
//! a [`VectorLaneConsolidationSource`] on top of the existing
//! [`crate::lanes::VectorLane`] trait so the same JWT + retry
//! machinery the orchestrator uses also drives this endpoint.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::lanes::{LaneHit, VectorLane, VectorRequest};
use crate::types::Scope;

/// Default value of `k` when the caller omits it.
pub const DEFAULT_K: u32 = 5;

/// Server-side hard cap on `k` — matches the spec's "max 10" floor
/// so a runaway `k` cannot blow up the MCP envelope.
pub const MAX_K: u32 = 10;

/// Default confidence floor — drops sub-0.6 cosine-similarity hits
/// before the response is built (mirrors the topic-card threshold).
pub const DEFAULT_CONFIDENCE_FLOOR: f64 = 0.6;

/// Wire request body for `POST /v1/search/similar-sessions`.
#[derive(Debug, Clone, Deserialize)]
pub struct SimilarSessionsRequest {
    /// Free-text query. Required.
    pub query: String,
    /// Repo slug. Required for the live path — the source needs to
    /// resolve `cortex-<repo>-consolidations`. The cross-repo
    /// "all collections" variant is a follow-up; today the handler
    /// rejects a missing repo with `400 bad_input`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Top-K. Clamped server-side to `[1, MAX_K]`. Defaults to
    /// [`DEFAULT_K`] when omitted.
    #[serde(default)]
    pub k: Option<u32>,
    /// Confidence floor. Defaults to [`DEFAULT_CONFIDENCE_FLOOR`]
    /// when omitted.
    #[serde(default)]
    pub confidence_floor: Option<f64>,
}

/// One consolidation hit in the response. Fields mirror the
/// `Consolidation` envelope shape the consolidator writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationHit {
    /// Stable consolidator key (`cons-ses-…` / `cons-top-…`).
    pub consolidation_id: String,
    /// Originating session id (empty when the consolidation is a
    /// cross-session rollup).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    /// Display title. Empty when the envelope omitted the field.
    pub title: String,
    /// Full summary markdown the consolidator emitted.
    pub summary_markdown: String,
    /// Count of source envelopes folded into this consolidation.
    #[serde(default)]
    pub source_event_count: u64,
    /// RFC-3339 occurred_at stamp from the envelope. Empty when
    /// the lane carried `ts = 0`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub occurred_at: String,
    /// Source score the lane returned. Higher = more similar.
    pub score: f64,
}

/// Wire response body for `POST /v1/search/similar-sessions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarSessionsResponse {
    /// Hits sorted by `score` descending, length ≤ effective `k`.
    pub hits: Vec<ConsolidationHit>,
    /// Equal to `hits.len()`; carried so consumers cannot drift
    /// from the server's projection.
    pub total: u32,
    /// Echo of the filter that produced the response. Helps the
    /// GUI render "we searched repo X, dropped sub-Y, capped K=Z"
    /// without re-deriving from the request body.
    pub filter: SimilarSessionsFilter,
}

/// Filter echo carried back on every [`SimilarSessionsResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarSessionsFilter {
    /// Repo slug that scoped the search.
    pub repo: String,
    /// Effective `k` after server-side clamping.
    pub k: u32,
    /// Effective confidence floor (defaulted server-side when the
    /// request omitted it).
    pub confidence_floor: f64,
}

/// Source the handler reads from. Live wiring uses
/// [`VectorLaneConsolidationSource`]; tests substitute a fixture
/// so the projection logic is exercised without a Vectorizer.
#[async_trait]
pub trait ConsolidationSearchSource: Send + Sync {
    /// Return up to `k` hits whose score is `>= confidence_floor`,
    /// sorted by score descending. Empty Vec when no consolidation
    /// matches the cutoff.
    async fn search(
        &self,
        query: &str,
        repo: &str,
        k: u32,
        confidence_floor: f64,
    ) -> Result<Vec<ConsolidationHit>, SimilarSessionsError>;
}

/// Source-level error shape. Mapped onto HTTP status codes by the
/// handler.
#[derive(Debug, thiserror::Error)]
pub enum SimilarSessionsError {
    /// Backend (Vectorizer) was unreachable or returned a transport
    /// error.
    #[error("backend transport: {0}")]
    Transport(String),
    /// Backend reported the collection does not exist. Maps onto
    /// `404 collection_missing` so callers can spot empty repos.
    #[error("collection missing: {0}")]
    CollectionMissing(String),
    /// Catch-all for non-transport upstream errors.
    #[error("backend: {0}")]
    Other(String),
}

/// Live source backed by any [`VectorLane`] implementation.
pub struct VectorLaneConsolidationSource<L: VectorLane> {
    lane: Arc<L>,
}

impl<L: VectorLane> VectorLaneConsolidationSource<L> {
    /// Build a live source wrapping the supplied vector lane.
    pub fn new(lane: Arc<L>) -> Self {
        Self { lane }
    }
}

#[async_trait]
impl<L: VectorLane + 'static> ConsolidationSearchSource for VectorLaneConsolidationSource<L> {
    async fn search(
        &self,
        query: &str,
        repo: &str,
        k: u32,
        confidence_floor: f64,
    ) -> Result<Vec<ConsolidationHit>, SimilarSessionsError> {
        let collection = consolidation_collection(repo);
        let req = VectorRequest {
            collection,
            query: query.to_string(),
            k: k as usize,
            scope: Scope::default(),
        };
        let hits = self
            .lane
            .search(&req)
            .await
            .map_err(|e| SimilarSessionsError::Transport(e.to_string()))?;
        let projected: Vec<ConsolidationHit> = hits
            .into_iter()
            .filter(|h| h.score >= confidence_floor)
            .map(project_hit)
            .collect();
        Ok(projected)
    }
}

/// Collection name the consolidator writes to per repo. Mirrors
/// `cortex_storage::names` so the live lookup hits the same index
/// the writer populates.
fn consolidation_collection(repo: &str) -> String {
    format!("cortex-{}-consolidations", repo.to_ascii_lowercase())
}

/// Project a [`LaneHit`] into the [`ConsolidationHit`] shape the
/// handler returns. Pure (same input ⇒ same output).
pub fn project_hit(hit: LaneHit) -> ConsolidationHit {
    let consolidation_id = hit
        .extras
        .get("consolidation_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| hit.doc_id.clone());
    let session_id = hit
        .extras
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let title = hit
        .extras
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let summary_markdown = hit
        .extras
        .get("body_markdown")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| hit.text.clone());
    let source_event_count = hit
        .extras
        .get("source_event_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let occurred_at = hit
        .extras
        .get("occurred_at")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    ConsolidationHit {
        consolidation_id,
        session_id,
        title,
        summary_markdown,
        source_event_count,
        occurred_at,
        score: hit.score,
    }
}

/// Source that honestly reports "no consolidations available". Used
/// at boot when the daemon has not wired a live Vectorizer lane
/// into the similar-sessions route yet — operators see an empty
/// payload rather than a 5xx, and the GUI distinguishes "wired but
/// empty" from "not wired" via the `repo` echo on the filter.
pub struct UnwiredConsolidationSource;

#[async_trait]
impl ConsolidationSearchSource for UnwiredConsolidationSource {
    async fn search(
        &self,
        _query: &str,
        _repo: &str,
        _k: u32,
        _confidence_floor: f64,
    ) -> Result<Vec<ConsolidationHit>, SimilarSessionsError> {
        Ok(Vec::new())
    }
}

/// Axum state holding the live source the handler dispatches to.
#[derive(Clone)]
pub struct SimilarSessionsState {
    /// Behind an `Arc<dyn _>` so the same handler can serve both
    /// live + fixture sources without monomorphising the route
    /// signature.
    pub source: Arc<dyn ConsolidationSearchSource>,
}

impl Default for SimilarSessionsState {
    fn default() -> Self {
        Self {
            source: Arc::new(UnwiredConsolidationSource),
        }
    }
}

/// Build the `/v1/search/similar-sessions` sub-router with the
/// supplied source. Callers that have not yet wired a live
/// Vectorizer lane use [`SimilarSessionsState::default`] to get
/// the honest-empty source.
pub fn build_router(state: SimilarSessionsState) -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route(
            "/v1/search/similar-sessions",
            post(similar_sessions_handler),
        )
        .with_state(state)
}

/// Axum handler for `POST /v1/search/similar-sessions`.
pub async fn similar_sessions_handler(
    State(state): State<SimilarSessionsState>,
    Json(req): Json<SimilarSessionsRequest>,
) -> Response {
    let query = req.query.trim();
    if query.is_empty() {
        return bad_input("`query` is required");
    }
    let repo = match req.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => r.to_string(),
        None => return bad_input("`repo` is required"),
    };
    let k = clamp_k(req.k);
    let floor = req.confidence_floor.unwrap_or(DEFAULT_CONFIDENCE_FLOOR);
    let mut hits = match state.source.search(query, &repo, k, floor).await {
        Ok(h) => h,
        Err(SimilarSessionsError::CollectionMissing(name)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "reason": "collection_missing",
                    "message": format!("no consolidation collection found for repo `{repo}`"),
                    "details": { "collection": name },
                })),
            )
                .into_response();
        }
        Err(SimilarSessionsError::Transport(msg)) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "reason": "vectorizer_unreachable",
                    "message": msg,
                })),
            )
                .into_response();
        }
        Err(SimilarSessionsError::Other(msg)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "reason": "backend_error",
                    "message": msg,
                })),
            )
                .into_response();
        }
    };
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if hits.len() > k as usize {
        hits.truncate(k as usize);
    }
    let total = hits.len() as u32;
    let response = SimilarSessionsResponse {
        hits,
        total,
        filter: SimilarSessionsFilter {
            repo,
            k,
            confidence_floor: floor,
        },
    };
    Json(response).into_response()
}

/// Clamp `k` into `[1, MAX_K]`. `None` returns [`DEFAULT_K`].
pub fn clamp_k(k: Option<u32>) -> u32 {
    k.unwrap_or(DEFAULT_K).clamp(1, MAX_K)
}

fn bad_input(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "reason": "bad_input",
            "message": msg,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct StubSource(Vec<ConsolidationHit>);

    #[async_trait]
    impl ConsolidationSearchSource for StubSource {
        async fn search(
            &self,
            _query: &str,
            _repo: &str,
            _k: u32,
            confidence_floor: f64,
        ) -> Result<Vec<ConsolidationHit>, SimilarSessionsError> {
            // Fixture intentionally ignores `k` (the handler is the
            // canonical k-clamper) and returns every hit above the
            // floor unsorted. This lets each test exercise the
            // handler's sort + truncate pipeline against the same
            // upstream shape.
            let out: Vec<ConsolidationHit> = self
                .0
                .iter()
                .filter(|h| h.score >= confidence_floor)
                .cloned()
                .collect();
            Ok(out)
        }
    }

    fn make_hit(id: &str, score: f64) -> ConsolidationHit {
        ConsolidationHit {
            consolidation_id: id.into(),
            session_id: "01HSESS".into(),
            title: format!("Title {id}"),
            summary_markdown: format!("Summary for {id}"),
            source_event_count: 3,
            occurred_at: "2026-05-25T12:00:00Z".into(),
            score,
        }
    }

    #[test]
    fn clamp_k_applies_default_and_bounds() {
        assert_eq!(clamp_k(None), DEFAULT_K);
        assert_eq!(clamp_k(Some(0)), 1);
        assert_eq!(clamp_k(Some(20)), MAX_K);
        assert_eq!(clamp_k(Some(5)), 5);
    }

    #[test]
    fn consolidation_collection_lowercases_repo() {
        assert_eq!(
            consolidation_collection("Cortex"),
            "cortex-cortex-consolidations"
        );
    }

    #[test]
    fn project_hit_falls_back_to_doc_id_and_text_when_extras_absent() {
        let hit = LaneHit {
            doc_id: "doc-1".into(),
            text: "raw body".into(),
            score: 0.9,
            extras: BTreeMap::new(),
            ..LaneHit::default()
        };
        let projected = project_hit(hit);
        assert_eq!(projected.consolidation_id, "doc-1");
        assert_eq!(projected.summary_markdown, "raw body");
        assert_eq!(projected.title, "");
        assert_eq!(projected.session_id, "");
        assert_eq!(projected.source_event_count, 0);
    }

    #[test]
    fn project_hit_pulls_consolidation_id_from_extras() {
        let mut extras = BTreeMap::new();
        extras.insert(
            "consolidation_id".into(),
            serde_json::Value::String("cons-ses-001".into()),
        );
        extras.insert(
            "title".into(),
            serde_json::Value::String("session digest".into()),
        );
        extras.insert(
            "body_markdown".into(),
            serde_json::Value::String("full markdown".into()),
        );
        extras.insert(
            "source_event_count".into(),
            serde_json::Value::Number(7.into()),
        );
        extras.insert(
            "session_id".into(),
            serde_json::Value::String("01HSESS001".into()),
        );
        let hit = LaneHit {
            doc_id: "doc-x".into(),
            text: "fallback".into(),
            score: 0.85,
            extras,
            ..LaneHit::default()
        };
        let projected = project_hit(hit);
        assert_eq!(projected.consolidation_id, "cons-ses-001");
        assert_eq!(projected.title, "session digest");
        assert_eq!(projected.summary_markdown, "full markdown");
        assert_eq!(projected.source_event_count, 7);
        assert_eq!(projected.session_id, "01HSESS001");
    }

    #[tokio::test]
    async fn handler_returns_empty_hits_when_source_returns_empty() {
        let state = SimilarSessionsState {
            source: Arc::new(StubSource(Vec::new())),
        };
        let resp = similar_sessions_handler(
            State(state),
            Json(SimilarSessionsRequest {
                query: "anything".into(),
                repo: Some("cortex".into()),
                k: None,
                confidence_floor: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let value: SimilarSessionsResponse = serde_json::from_slice(&body).unwrap();
        assert!(value.hits.is_empty());
        assert_eq!(value.total, 0);
        assert_eq!(value.filter.repo, "cortex");
        assert_eq!(value.filter.k, DEFAULT_K);
        assert_eq!(value.filter.confidence_floor, DEFAULT_CONFIDENCE_FLOOR);
    }

    #[tokio::test]
    async fn handler_sorts_hits_by_score_descending_then_caps_at_k() {
        let state = SimilarSessionsState {
            source: Arc::new(StubSource(vec![
                make_hit("a", 0.7),
                make_hit("b", 0.95),
                make_hit("c", 0.8),
                make_hit("d", 0.65),
                make_hit("e", 0.85),
            ])),
        };
        let resp = similar_sessions_handler(
            State(state),
            Json(SimilarSessionsRequest {
                query: "rework".into(),
                repo: Some("cortex".into()),
                k: Some(3),
                confidence_floor: Some(0.6),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let value: SimilarSessionsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.total, 3);
        let ids: Vec<&str> = value
            .hits
            .iter()
            .map(|h| h.consolidation_id.as_str())
            .collect();
        // `b` (0.95) > `e` (0.85) > `c` (0.8); `a` (0.7) and `d` (0.65)
        // drop because k = 3 trims after sorting.
        assert_eq!(ids, vec!["b", "e", "c"]);
    }

    #[tokio::test]
    async fn handler_clamps_k_above_max_to_max_k() {
        let state = SimilarSessionsState {
            source: Arc::new(StubSource(
                (0..20)
                    .map(|i| make_hit(&format!("h{i}"), 1.0 - (i as f64) * 0.01))
                    .collect(),
            )),
        };
        let resp = similar_sessions_handler(
            State(state),
            Json(SimilarSessionsRequest {
                query: "x".into(),
                repo: Some("cortex".into()),
                k: Some(50),
                confidence_floor: Some(0.0),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let value: SimilarSessionsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.total, MAX_K);
        assert_eq!(value.filter.k, MAX_K);
    }

    #[tokio::test]
    async fn handler_rejects_missing_query() {
        let state = SimilarSessionsState {
            source: Arc::new(StubSource(Vec::new())),
        };
        let resp = similar_sessions_handler(
            State(state),
            Json(SimilarSessionsRequest {
                query: "   ".into(),
                repo: Some("cortex".into()),
                k: None,
                confidence_floor: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_rejects_missing_repo() {
        let state = SimilarSessionsState {
            source: Arc::new(StubSource(Vec::new())),
        };
        let resp = similar_sessions_handler(
            State(state),
            Json(SimilarSessionsRequest {
                query: "anything".into(),
                repo: None,
                k: None,
                confidence_floor: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
