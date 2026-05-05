use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::tasks_loader::{ListQuery, SortField, SortOrder};

use super::DashboardState;

/// Query params for the list endpoint. `axum-extra::Query` handles the
/// repeated multi-value params (`status=...&status=...`) directly.
#[derive(Debug, Default, Deserialize)]
pub struct TasksListQuery {
    /// Multi-value status filter.
    #[serde(default)]
    pub status: Vec<String>,
    /// Multi-value phase filter (exact match against the canonical
    /// phase key, e.g. `phase2g`).
    #[serde(default)]
    pub phase: Vec<String>,
    /// Multi-value repo (project) filter — phase5b multi-project.
    /// Matches `TaskRow.repo` (lowercase project slug).
    #[serde(default)]
    pub repo: Vec<String>,
    /// Drop archived rows when set to `false`. Defaults to `true`.
    #[serde(default)]
    pub include_archived: Option<bool>,
    /// Page size (default 200, capped at 5000 by the loader).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Page offset (default 0).
    #[serde(default)]
    pub offset: Option<usize>,
    /// `phase` (default), `updated_at`, or `created_at`.
    #[serde(default)]
    pub sort: Option<String>,
    /// `asc` or `desc`. Defaults to ascending for `phase` and
    /// descending for the timestamp fields.
    #[serde(default)]
    pub order: Option<String>,
}

pub(super) fn list_query_from(params: TasksListQuery) -> ListQuery {
    let sort = match params.sort.as_deref() {
        Some("updated_at") => SortField::UpdatedAt,
        Some("created_at") => SortField::CreatedAt,
        _ => SortField::Phase,
    };
    let order = match params.order.as_deref() {
        Some("asc") => Some(SortOrder::Asc),
        Some("desc") => Some(SortOrder::Desc),
        _ => None,
    };
    ListQuery {
        status: params.status,
        phase: params.phase,
        repo: params.repo,
        include_archived: params.include_archived.unwrap_or(true),
        limit: params.limit.unwrap_or(200),
        offset: params.offset.unwrap_or(0),
        sort,
        order,
    }
}

/// `GET /v1/dashboard/tasks` — filtered list with phase + status
/// breakdowns. Returns `200` even when the workspace root is missing
/// (just yields an empty list + zero breakdowns) so the GUI's empty
/// state is the only thing the user sees on a misconfigured deploy.
pub(super) async fn tasks_list(
    State(state): State<DashboardState>,
    Query(params): Query<TasksListQuery>,
) -> Response {
    let query = list_query_from(params);
    let body = state.tasks.list(&query);
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /v1/dashboard/tasks/summary` — aggregate counters for the
/// sidebar pill + the Tasks-view stats grid.
pub(super) async fn tasks_summary(State(state): State<DashboardState>) -> Response {
    let body = state.tasks.summary();
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /v1/dashboard/tasks/{id}` — full proposal + sectioned
/// checklist + listing of `specs/`. Returns `404` when the id is not
/// found in either the active or archived tree.
pub(super) async fn tasks_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    match state.tasks.detail(&id) {
        Some(body) => (StatusCode::OK, Json(body)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "task_not_found", "id": id })),
        )
            .into_response(),
    }
}
