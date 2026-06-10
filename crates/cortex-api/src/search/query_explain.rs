//! Phase19 §3.5 — `POST /v1/query/explain` handler.
//!
//! Diagnostic surface — runs the same `cortex_query` pipeline the
//! caller would hit via `POST /v1/query` and projects the response
//! into an explain envelope so operators can see WHAT the
//! orchestrator did, not just the final bundle.
//!
//! ## Scope deviation from the original spec
//!
//! The task spec listed `per_lane_hits[]` as a per-lane raw hit
//! list. The current `QueryResponse` carries only `debug.lanes`
//! (wall-clock timings) + per-lane error / note bags
//! (`crates/cortex-api/src/types.rs` — `LaneTimings` / `DebugInfo`);
//! the orchestrator does NOT retain the pre-fusion ranked lists
//! after `rrf_fuse` runs. Surfacing the raw lists would require a
//! richer `DebugInfo` shape + an orchestrator change so the lane
//! futures stash their ranked output. Larger than this phase.
//!
//! The handler instead projects:
//!
//! - `per_lane_hits[]`: one entry per lane the orchestrator ran,
//!   with `ms` (wall-clock) + `error` (when fan-out failed) +
//!   `note` (when the lane recorded a structured note like
//!   `collection_missing`). Raw ranked hits are intentionally
//!   `null` until the orchestrator wires them through.
//! - `fusion_math`: the active RRF constants (`rrf_k`, `alpha`),
//!   the per-intent default `recency_decay_lambda`, and `drops`
//!   reconciled from `debug.errors` / `debug.notes`. Caller-
//!   supplied `scope.recency_decay` overrides the intent default.
//! - `final_envelope`: the entire `QueryResponse` returned by the
//!   orchestrator. Callers get the same shape they would get from
//!   `/v1/query` without re-running.
//!
//! Response surfaces `match_strategy = "envelope_only"` so callers
//! know they got the projection-only shape. Reserved value
//! `"with_lane_hits"` lands when the orchestrator exposes the
//! pre-fusion ranked lists.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{ApiState, CALLER_HEADER};
use crate::service::ServiceOutcome;
use crate::types::{DebugNote, IncludeField, Intent, QueryRequest, QueryResponse, Scope};

/// Request body for `POST /v1/query/explain`. Subset of
/// [`QueryRequest`] — `intent` defaults to `FreeSearch`, `scope`
/// defaults to empty.
#[derive(Debug, Clone, Deserialize)]
pub struct QueryExplainRequest {
    /// Free-text query — required.
    pub query: String,
    /// Intent label. Defaults to `free_search` when omitted.
    #[serde(default)]
    pub intent: Option<Intent>,
    /// Scope filter (mirrors `/v1/query`).
    #[serde(default)]
    pub scope: Option<Scope>,
}

/// One lane the orchestrator fanned out to.
#[derive(Debug, Clone, Serialize)]
pub struct LaneExplain {
    /// Lane discriminator (`vector` / `keyword` / `graph`).
    pub lane: &'static str,
    /// Wall-clock latency in ms (echoed from `debug.lanes.<lane>_ms`).
    /// `null` when the orchestrator did not run this lane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// Error message when the lane fan-out failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Structured note when the lane recorded one (e.g.
    /// `collection_missing`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<DebugNote>,
    /// Reserved — pre-fusion ranked hit list is not surfaced today
    /// (see module-level scope deviation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranked_hits: Option<Vec<Value>>,
}

/// Fusion-math summary attached to the explain envelope.
#[derive(Debug, Clone, Serialize)]
pub struct FusionMath {
    /// RRF k constant. Pulled from
    /// [`crate::search::fusion::RRF_K`].
    pub rrf_k: f64,
    /// RRF alpha (positional vs native-score blend). Pulled from
    /// [`crate::search::fusion::DEFAULT_RRF_ALPHA`].
    pub alpha: f32,
    /// Recency-decay λ honoured for this request. When the caller
    /// passes `scope.recency_decay`, that wins; otherwise the
    /// orchestrator's per-intent default applies.
    pub recency_decay_lambda: f32,
    /// Drops reconciled from `debug.errors` (lane fan-out
    /// failures) + `debug.notes` (collection-missing, etc.).
    /// Empty Vec when the orchestrator returned a clean run.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drops: Vec<FusionDrop>,
}

/// One drop entry — lane name + reason. Stable shape so dashboards
/// can group by `reason`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FusionDrop {
    /// Lane that produced the drop (`vector` / `keyword` /
    /// `graph`).
    pub lane: String,
    /// Stable kind (`error` / `collection_missing` / other note
    /// kinds the orchestrator emits).
    pub reason: String,
    /// Human-readable message.
    pub message: String,
}

/// Response body for `POST /v1/query/explain`.
#[derive(Debug, Clone, Serialize)]
pub struct QueryExplainResponse {
    /// Echo of the requested intent.
    pub intent: String,
    /// Per-lane fan-out summary.
    pub per_lane_hits: Vec<LaneExplain>,
    /// Fusion-math summary.
    pub fusion_math: FusionMath,
    /// Final envelope the orchestrator returned (full shape).
    pub final_envelope: QueryResponse,
    /// Always `"envelope_only"` today. Reserved value
    /// `"with_lane_hits"` lands when the orchestrator exposes the
    /// pre-fusion ranked lists.
    pub match_strategy: &'static str,
}

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

/// Reconcile `debug.errors` + `debug.notes` into a single drop list.
pub(crate) fn collect_drops(envelope: &QueryResponse) -> Vec<FusionDrop> {
    let mut out: Vec<FusionDrop> = Vec::new();
    for (lane, message) in &envelope.debug.errors {
        out.push(FusionDrop {
            lane: lane.clone(),
            reason: "error".to_string(),
            message: message.clone(),
        });
    }
    for note in &envelope.debug.notes {
        out.push(FusionDrop {
            lane: note.lane.clone(),
            reason: note.kind.clone(),
            message: note.message.clone(),
        });
    }
    out
}

/// Build the per-lane explain list. Includes every lane the
/// orchestrator may have run — `ms: None` when the lane was not
/// invoked. The match against `errors` / `notes` is keyed on the
/// lane label string the orchestrator uses (lowercase).
pub(crate) fn build_per_lane(envelope: &QueryResponse) -> Vec<LaneExplain> {
    let lanes = [
        ("vector", envelope.debug.lanes.vector_ms),
        ("keyword", envelope.debug.lanes.keyword_ms),
        ("graph", envelope.debug.lanes.graph_ms),
    ];
    lanes
        .into_iter()
        .map(|(lane, ms)| {
            let error = envelope.debug.errors.get(lane).cloned();
            let note = envelope
                .debug
                .notes
                .iter()
                .find(|n| n.lane == lane)
                .cloned();
            LaneExplain {
                lane,
                ms,
                error,
                note,
                ranked_hits: None,
            }
        })
        .collect()
}

/// Resolve the recency-decay λ the orchestrator honoured for this
/// request. Caller-supplied `scope.recency_decay` wins; otherwise
/// the per-intent default applies (mirrors the orchestrator's
/// resolution in
/// [`crate::search::fusion::FusionConfig::default_recency_lambda_for_intent`]).
pub(crate) fn resolve_recency_lambda(req: &QueryRequest) -> f32 {
    if let Some(lambda) = req.scope.recency_decay {
        return lambda;
    }
    crate::search::fusion::FusionConfig::default_recency_lambda_for_intent(req.intent.label())
}

/// Handler — `POST /v1/query/explain`.
pub async fn handle_query_explain(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<QueryExplainRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let query = req.query.trim();
    if query.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "bad_input", "`query` is required");
    }
    let intent = req.intent.unwrap_or(Intent::FreeSearch);
    let scope = req.scope.unwrap_or_default();
    let query_req = QueryRequest {
        intent,
        scope,
        query: query.to_string(),
        limit: 20,
        k: 50,
        include: vec![
            IncludeField::Snippets,
            IncludeField::Decisions,
            IncludeField::Violations,
        ],
        budget_ms: 500,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
    };
    let recency_lambda = resolve_recency_lambda(&query_req);

    let caller = headers
        .get(CALLER_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();
    let envelope = match state
        .service
        .handle_with_headers(&caller, query_req, &headers)
        .await
    {
        ServiceOutcome::Ok(resp) => *resp,
        ServiceOutcome::EmptyQuery => {
            return json_err(StatusCode::BAD_REQUEST, "empty_query", "query was empty");
        }
        ServiceOutcome::EmptyScope => {
            return json_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "scope_repo_required",
                "scope.repo is required for this caller",
            );
        }
        ServiceOutcome::Denied => {
            return json_err(StatusCode::FORBIDDEN, "scope_forbidden", "scope ACL denied");
        }
        ServiceOutcome::RateLimited(_) => {
            return json_err(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "caller exhausted quota",
            );
        }
    };

    let per_lane_hits = build_per_lane(&envelope);
    let drops = collect_drops(&envelope);
    let fusion_math = FusionMath {
        rrf_k: crate::search::fusion::RRF_K,
        alpha: crate::search::fusion::DEFAULT_RRF_ALPHA,
        recency_decay_lambda: recency_lambda,
        drops,
    };
    let intent_label = envelope.intent.clone();

    Json(QueryExplainResponse {
        intent: intent_label,
        per_lane_hits,
        fusion_math,
        final_envelope: envelope,
        match_strategy: "envelope_only",
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DebugInfo, DebugNote, LaneTimings};
    use std::collections::BTreeMap;

    fn envelope_with(
        errors: BTreeMap<String, String>,
        notes: Vec<DebugNote>,
        timings: LaneTimings,
    ) -> QueryResponse {
        QueryResponse {
            debug: DebugInfo {
                lanes: timings,
                errors,
                truncated: false,
                notes,
            },
            ..QueryResponse::default()
        }
    }

    #[test]
    fn build_per_lane_emits_three_entries_with_timings() {
        let env = envelope_with(
            BTreeMap::new(),
            Vec::new(),
            LaneTimings {
                vector_ms: Some(12),
                keyword_ms: Some(34),
                graph_ms: None,
            },
        );
        let lanes = build_per_lane(&env);
        assert_eq!(lanes.len(), 3);
        assert_eq!(lanes[0].lane, "vector");
        assert_eq!(lanes[0].ms, Some(12));
        assert_eq!(lanes[1].lane, "keyword");
        assert_eq!(lanes[1].ms, Some(34));
        assert_eq!(lanes[2].lane, "graph");
        assert!(lanes[2].ms.is_none());
    }

    #[test]
    fn build_per_lane_attaches_errors_and_notes_per_lane() {
        let mut errors = BTreeMap::new();
        errors.insert("vector".into(), "vectorizer unreachable".into());
        let notes = vec![DebugNote {
            lane: "keyword".into(),
            kind: "collection_missing".into(),
            collection: Some("cortex-foo-turns".into()),
            message: "no such index".into(),
        }];
        let env = envelope_with(errors, notes, LaneTimings::default());
        let lanes = build_per_lane(&env);
        assert_eq!(lanes[0].error.as_deref(), Some("vectorizer unreachable"));
        assert!(lanes[0].note.is_none());
        assert!(lanes[1].error.is_none());
        assert_eq!(lanes[1].note.as_ref().unwrap().kind, "collection_missing");
    }

    #[test]
    fn collect_drops_unions_errors_and_notes_with_stable_reason() {
        let mut errors = BTreeMap::new();
        errors.insert("vector".into(), "boom".into());
        let notes = vec![DebugNote {
            lane: "keyword".into(),
            kind: "collection_missing".into(),
            collection: None,
            message: "missing".into(),
        }];
        let env = envelope_with(errors, notes, LaneTimings::default());
        let drops = collect_drops(&env);
        assert_eq!(drops.len(), 2);
        assert!(drops
            .iter()
            .any(|d| d.lane == "vector" && d.reason == "error"));
        assert!(drops
            .iter()
            .any(|d| d.lane == "keyword" && d.reason == "collection_missing"));
    }

    fn req_for(intent: Intent) -> QueryRequest {
        QueryRequest {
            intent,
            scope: Scope::default(),
            query: "x".into(),
            limit: 20,
            k: 50,
            include: vec![IncludeField::Snippets],
            budget_ms: 500,
            budget_bytes: None,
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
        }
    }

    #[test]
    fn resolve_recency_lambda_prefers_scope_override() {
        let mut req = req_for(Intent::FreeSearch);
        req.scope.recency_decay = Some(0.42);
        assert_eq!(resolve_recency_lambda(&req), 0.42);
    }

    #[test]
    fn resolve_recency_lambda_falls_back_to_intent_default() {
        let req = req_for(Intent::FreeSearch);
        let expected = crate::search::fusion::FusionConfig::default_recency_lambda_for_intent(
            req.intent.label(),
        );
        assert_eq!(resolve_recency_lambda(&req), expected);
    }
}
