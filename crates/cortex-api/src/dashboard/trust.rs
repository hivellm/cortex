use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::DashboardState;

/// Trust matrix payload — model × repo cells with a `[0, 1]` score.
/// Spec 14 owns the actual computation (rolling violation rate per
/// (model, repo) pair). Until that ships the lane has no per-(model,
/// repo) trust signal, so we return empty arrays / map and the GUI
/// surfaces an empty state.
#[derive(Debug, Clone, Serialize)]
pub struct TrustMatrix {
    /// Model names (rows of the matrix).
    pub models: Vec<String>,
    /// Repo names (columns of the matrix).
    pub repos: Vec<String>,
    /// `scores[model][repo]` → trust score in `[0, 1]`.
    pub scores: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    /// Provenance hint for the renderer's empty-state copy. Today
    /// always `"stub_until_spec14"`; flips to `"derived"` once spec
    /// 14 starts computing rolling violation rates per
    /// `(model, repo)`.
    pub source: &'static str,
}

pub(super) async fn trust(State(_state): State<DashboardState>) -> Response {
    let body = TrustMatrix {
        models: Vec::new(),
        repos: Vec::new(),
        scores: std::collections::BTreeMap::new(),
        source: "stub_until_spec14",
    };
    (StatusCode::OK, Json(body)).into_response()
}
