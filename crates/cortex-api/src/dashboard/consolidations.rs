//! `/v1/dashboard/consolidations` — surfaces phase11j Consolidation
//! envelopes (session / topic / decision_trace digests produced by
//! `cortex-consolidator` and `cortex-ops tool-call-digest`).
//!
//! Reads the lane's `cortex-meili-consolidations` index that
//! [`crate::meili_loader`] seeds at boot + every Meili refresh. Any
//! envelope whose [`crate::dashboard::symbol_to_kind`] resolves to
//! `"consolidation"` shows up here.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{clip, collect_lane_hits, normalize_repo, ts_to_relative, DashboardState};

/// One row in the `/v1/dashboard/consolidations` list.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationRow {
    /// Envelope ULID — opaque identifier, also the route id for
    /// `/v1/dashboard/consolidations/{id}`.
    pub id: String,
    /// Stable consolidator key (`cons-ses-…` / `cons-top-…` / etc.)
    /// when present; some bootstrap envelopes omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consolidation_id: Option<String>,
    /// Display title — first non-empty of `title` / first body line.
    pub title: String,
    /// `session` | `topic` | `decision_trace`. Empty when the
    /// envelope predates the schema bump.
    pub grain: String,
    /// `shallow` | `deep`. Empty when the envelope predates the bump.
    pub depth: String,
    /// Model id that produced the summary (e.g. `claude-haiku-4-5`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Number of source envelopes folded into this consolidation.
    /// `0` when the envelope omits the field.
    #[serde(default)]
    pub source_event_count: u64,
    /// Repo this consolidation is anchored to. `None` for cross-repo
    /// rollups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Free-text topics list — controlled vocab from the classifier.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Body excerpt clipped at 800 chars; full markdown ships under
    /// `/v1/dashboard/consolidations/{id}` `body_markdown`.
    pub excerpt: String,
    /// Relative time label.
    pub occurred_at: String,
}

/// Detail body for `/v1/dashboard/consolidations/{id}` — the row plus
/// the unclipped markdown.
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationDetail {
    /// Spread of [`ConsolidationRow`].
    #[serde(flatten)]
    pub row: ConsolidationRow,
    /// Full envelope body (markdown).
    pub body_markdown: String,
}

/// Query params — mirrors [`super::DecisionsQuery`] / [`super::AnalysesQuery`]
/// so the GUI's filter dropdowns work the same way.
#[derive(Debug, Default, Deserialize)]
pub struct ConsolidationsQuery {
    /// Single-repo filter — `?repo=Cortex`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Multi-repo filter — `?repos=Cortex&repos=Nexus`.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Single-grain filter — `?grain=session`.
    #[serde(default)]
    pub grain: Option<String>,
}

fn build_row(h: &crate::lanes::LaneHit) -> (ConsolidationRow, String) {
    let title = h
        .extras
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| clip(h.text.lines().next().unwrap_or(""), 120));
    let body_markdown = h
        .extras
        .get("body_markdown")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| h.text.clone());
    let grain = h
        .extras
        .get("grain")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let depth = h
        .extras
        .get("depth")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model = h
        .extras
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let consolidation_id = h
        .extras
        .get("consolidation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let source_event_count = h
        .extras
        .get("source_event_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let topics: Vec<String> = h
        .extras
        .get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let row = ConsolidationRow {
        id: h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id)
            .to_string(),
        consolidation_id,
        title,
        grain,
        depth,
        model,
        source_event_count,
        repo: h.repo.clone(),
        topics,
        excerpt: clip(&body_markdown, 800),
        occurred_at: ts_to_relative(h.ts),
    };
    (row, body_markdown)
}

pub(super) async fn consolidations(
    State(state): State<DashboardState>,
    Query(params): Query<ConsolidationsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let mut allow: std::collections::HashSet<String> = params
        .repos
        .into_iter()
        .map(|r| normalize_repo(&r))
        .collect();
    if let Some(r) = params.repo.filter(|s| !s.is_empty()) {
        allow.insert(normalize_repo(&r));
    }
    let grain_filter = params.grain.filter(|s| !s.is_empty());

    let mut rows: Vec<ConsolidationRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("consolidation"))
        .filter(|h| {
            if allow.is_empty() {
                return true;
            }
            match h.repo.as_deref() {
                Some(r) => allow.contains(&normalize_repo(r)),
                None => false,
            }
        })
        .map(|h| build_row(&h).0)
        .filter(|r| match grain_filter.as_deref() {
            Some(g) => r.grain == g,
            None => true,
        })
        .collect();
    // Newest first by ts proxy — ts is 0 for many envelopes so fall
    // back to id sort to keep the order stable across reloads.
    rows.sort_by(|a, b| b.id.cmp(&a.id));

    Json(rows).into_response()
}

pub(super) async fn consolidation_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let want = id.trim();
    for h in &hits {
        if h.symbol.as_deref() != Some("consolidation") {
            continue;
        }
        let row_id = h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id);
        if row_id == want {
            let (row, body_markdown) = build_row(h);
            return Json(ConsolidationDetail { row, body_markdown }).into_response();
        }
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "consolidation not found", "id": id })),
    )
        .into_response()
}
