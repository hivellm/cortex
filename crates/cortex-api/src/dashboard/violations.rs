use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{clip, collect_lane_hits, ts_to_relative, DashboardState};

/// One law violation — shape mirrors `MOCK.violations`. Sourced from
/// `kind=law_violation` envelopes in the lane.
#[derive(Debug, Clone, Serialize)]
pub struct ViolationRow {
    /// Violation id.
    pub id: String,
    /// Law id the violation is tagged against.
    pub law_id: Option<String>,
    /// Relative time label.
    pub at: String,
    /// Repo where the violation was observed.
    pub repo: Option<String>,
    /// `blocked` | `annotated` | other.
    pub action: String,
    /// Evidence excerpt.
    pub evidence: String,
    /// Free-text remediation note.
    pub remediation: Option<String>,
}

pub(super) async fn violations(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows: Vec<ViolationRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("law_violation"))
        .map(|h| {
            let id = h
                .doc_id
                .strip_prefix("archive|")
                .unwrap_or(&h.doc_id)
                .to_string();
            // The meili_loader stamps `law_id` on extras when the
            // envelope body carries it. Fall back to None for
            // envelopes that arrived through the (currently empty)
            // archive path.
            let law_id = h
                .extras
                .get("law_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Critical-severity violations are surfaced as `blocked`;
            // everything else stays `annotated`. Honest mapping until
            // spec-14 emits a richer `action` enum on the wire.
            let action = if h.severity.as_deref() == Some("critical") {
                "blocked".to_string()
            } else {
                "annotated".to_string()
            };
            ViolationRow {
                id,
                law_id,
                at: ts_to_relative(h.ts),
                repo: h.repo.clone(),
                action,
                evidence: clip(&h.text, 240),
                remediation: None,
            }
        })
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}
