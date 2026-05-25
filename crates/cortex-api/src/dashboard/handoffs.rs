use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{
    clip, collect_lane_hits, normalize_repo, symbol_to_kind, ts_to_relative, DashboardState,
};

/// One hand-off snapshot — pulled from `kind=memory` envelopes whose
/// path lives under `.rulebook/handoff/`. The walker now promotes
/// those automatically across every repo.
#[derive(Debug, Clone, Serialize)]
pub struct HandoffRow {
    /// Repo this hand-off belongs to.
    pub repo: Option<String>,
    /// Repo-relative path of the hand-off file.
    pub path: Option<String>,
    /// Filename component for display (e.g. `_pending.md` /
    /// `2026-04-27.md`).
    pub filename: String,
    /// Excerpt of the body clipped at ~600 chars.
    pub excerpt: String,
    /// Relative wall-clock label (`"3 hours ago"` / `"yesterday"`)
    /// — empty when no timestamp was preserved.
    pub updated: String,
    /// Raw ms epoch for client-side sorting.
    pub updated_ms: i64,
}

/// Query params for `/v1/dashboard/handoffs`.
#[derive(Debug, Default, Deserialize)]
pub struct HandoffsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
}

pub(super) async fn handoffs(
    State(state): State<DashboardState>,
    Query(params): Query<HandoffsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);

    let mut rows: Vec<HandoffRow> = hits
        .into_iter()
        .filter(|h| {
            symbol_to_kind(h.symbol.as_deref()) == "memory"
                && h.path
                    .as_deref()
                    .map(|p| p.contains(".rulebook/handoff/") || p.contains(".rulebook\\handoff\\"))
                    .unwrap_or(false)
        })
        .filter(|h| {
            // Optional repo filter.
            match params.repo.as_deref().filter(|s| !s.is_empty()) {
                Some(r) => h.repo.as_deref().map(normalize_repo) == Some(normalize_repo(r)),
                None => true,
            }
        })
        .map(|h| {
            let filename = h
                .path
                .as_deref()
                .and_then(|p| p.rsplit(['/', '\\']).next())
                .unwrap_or("(unnamed)")
                .to_string();
            HandoffRow {
                repo: h.repo.clone(),
                path: h.path.clone(),
                filename,
                excerpt: clip(&h.text, 600),
                updated: ts_to_relative(h.ts),
                updated_ms: h.ts,
            }
        })
        .collect();

    // Most-recent first — the user is usually looking for the latest
    // hand-off when resuming a session.
    rows.sort_by_key(|r| std::cmp::Reverse(r.updated_ms));
    (StatusCode::OK, Json(rows)).into_response()
}
