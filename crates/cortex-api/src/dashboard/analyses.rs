use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{clip, collect_lane_hits, normalize_repo, ts_to_relative, DashboardState};

/// One analysis row — sourced from `kind=analysis` envelopes via two
/// upstream paths:
///
/// - **phase4e bootstrap-imported audits** (`docs/analysis/**/*.md`):
///   carry `{ title, status, body, source_path }`. `repo` is the owning
///   project; `source_path` deep-links the on-disk markdown. Spec-15
///   fields (panel, judge, rounds, duration) are absent and surface as
///   empty defaults.
/// - **spec-15 deep-analysis envelopes** (when the workflow ships):
///   carry the full debate metadata. Both shapes flow through this row.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisRow {
    /// Analysis id (event id for imports; `analysis_id` for spec-15).
    pub id: String,
    /// Free-text title — H1 of the imported markdown, or `question`
    /// for spec-15.
    pub title: String,
    /// `draft` | `running` | `concluded` | `cancelled`. Imports default
    /// to whatever the markdown's `Status:` line says (or `draft`).
    pub status: String,
    /// Panelist identifiers (model / agent names). Empty for imports.
    pub panel: Vec<String>,
    /// Identifier of the judging model / human. Empty for imports.
    pub judge: String,
    /// Number of debate rounds. Zero for imports.
    pub rounds: u32,
    /// Total wall-clock duration in seconds. Zero for imports.
    pub duration_s: u32,
    /// Final verdict body (Markdown).
    pub verdict: String,
    /// Decision id this analysis was promoted to (when applicable).
    pub decision_id: Option<String>,
    /// Relative time label.
    pub occurred_at: String,
    /// Owning repo — read off the envelope's `context_repo` so the GUI
    /// can group / filter analyses per project. `None` when the analysis
    /// is not anchored to a repo (rare; spec-15 cross-repo debates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Repo-rooted source path for imported audits — lets the GUI deep-
    /// link to the underlying markdown. Absent for spec-15 envelopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// Query params for `/v1/dashboard/analyses`. Mirrors
/// [`DecisionsQuery`] so the GUI's Analysis library tab can filter by
/// project (single `?repo=` or multi `?repos=`). Empty → all repos.
#[derive(Debug, Default, Deserialize)]
pub struct AnalysesQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Multi-repo filter — `?repos=Cortex&repos=Nexus`.
    #[serde(default)]
    pub repos: Vec<String>,
}

/// Detail body for `/v1/dashboard/analyses/{id}` — the list row plus
/// the un-clipped Markdown body. Mirrors [`DecisionDetail`] so the GUI's
/// drawer can render a full audit instead of the 800-char list excerpt.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisDetail {
    /// Spread of [`AnalysisRow`]. The `verdict` here is the same clipped
    /// excerpt the list endpoint serves; `body_markdown` carries the
    /// full document.
    #[serde(flatten)]
    pub row: AnalysisRow,
    /// Full envelope body (markdown). Sourced from
    /// `extras.body_markdown` when the meili_loader populated it,
    /// falling back to the raw lane text.
    pub body_markdown: String,
}

/// Build a single analysis row from a lane hit. Returns the row and the
/// un-clipped body so callers can either drop it (list endpoint) or
/// surface it (detail endpoint) without re-parsing the envelope twice.
fn build_analysis_row(h: &crate::lanes::LaneHit) -> (AnalysisRow, String) {
    let title = h
        .extras
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| clip(h.text.lines().next().unwrap_or(""), 120));
    let status = h
        .extras
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| {
            let token: String = s
                .chars()
                .skip_while(|c| !c.is_ascii_alphabetic())
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            token.to_ascii_lowercase()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "draft".to_string());
    let source_path = h
        .extras
        .get("source_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| h.path.clone());
    let body_markdown = h
        .extras
        .get("body_markdown")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| h.text.clone());
    let row = AnalysisRow {
        id: h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id)
            .to_string(),
        title,
        status,
        panel: Vec::new(),
        judge: String::new(),
        rounds: 0,
        duration_s: 0,
        verdict: clip(&body_markdown, 800),
        decision_id: None,
        occurred_at: ts_to_relative(h.ts),
        repo: h.repo.clone(),
        source_path,
    };
    (row, body_markdown)
}

pub(super) async fn analyses(
    State(state): State<DashboardState>,
    Query(params): Query<AnalysesQuery>,
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
    let rows: Vec<AnalysisRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("analysis"))
        .filter(|h| {
            if allow.is_empty() {
                return true;
            }
            h.repo
                .as_deref()
                .map(|r| allow.contains(&normalize_repo(r)))
                .unwrap_or(false)
        })
        .map(|h| build_analysis_row(&h).0)
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}

pub(super) async fn analysis_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let detail = hits
        .iter()
        .filter(|h| h.symbol.as_deref() == Some("analysis"))
        .find(|h| h.doc_id.strip_prefix("archive|").unwrap_or(&h.doc_id) == id)
        .map(|h| {
            let (row, body_markdown) = build_analysis_row(h);
            AnalysisDetail { row, body_markdown }
        });
    match detail {
        Some(d) => (StatusCode::OK, Json(d)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "reason": "analysis_not_found" })),
        )
            .into_response(),
    }
}
