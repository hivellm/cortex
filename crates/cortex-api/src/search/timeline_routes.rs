//! Phase18 §4.4 — HTTP surfaces for the timeline + branch + entity
//! retrieval routes.
//!
//! Every handler in this file talks to Nexus through the shared
//! `ApiState::nexus` handle. Reads use inlined-literal Cypher to
//! sidestep the Nexus 2.2.0 parameter-binding bug; the writers
//! reuse the same per-call `sanitize_literal` filter as the
//! `cortex-ops branch` CLI (phase18 §4.2).
//!
//! Routes shipped:
//!
//! - `GET  /v1/timeline/{project}`
//! - `GET  /v1/branch/{project}`
//! - `POST /v1/branch/{project}`
//! - `GET  /v1/branch/{project}/{branch}`
//! - `POST /v1/branch/{project}/{branch}/merge`
//! - `POST /v1/branch/{project}/{branch}/abandon`
//! - `GET  /v1/entity/{id}/history`
//! - `GET  /v1/entity/{id}/supersession`

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::http::ApiState;
use cortex_workers::graph::branch::{validate_branch_name, BRANCH_LABEL};
use cortex_workers::temporal::branch_filter::compose_id;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

fn err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

fn sanitize_literal(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '\n' | '\r'))
        .collect()
}

fn parse_unix(raw: Option<&str>) -> Result<Option<i64>, String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| Some(dt.timestamp()))
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Nexus returns each Cypher row as a positional array keyed by the
/// separate `columns` list. The HTTP surface (and the GUI / spec 33)
/// contract is keyed objects (`{ id, kind, title, ... }`), so zip the
/// columns with each row's cells into a JSON object. Rows that are
/// already objects (or scalars) pass through unchanged.
fn key_rows(columns: &[String], rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| match row.as_array() {
            Some(cells) => {
                let mut obj = serde_json::Map::with_capacity(columns.len());
                for (i, col) in columns.iter().enumerate() {
                    obj.insert(
                        col.clone(),
                        cells.get(i).cloned().unwrap_or(serde_json::Value::Null),
                    );
                }
                serde_json::Value::Object(obj)
            }
            None => row.clone(),
        })
        .collect()
}

// ============================================================================
// GET /v1/timeline/{project}
// ============================================================================

#[derive(Debug, Clone, Default, Deserialize)]
/// Query params for `GET /v1/timeline/{project}`.
pub struct TimelineQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Optional branch filter (`<project>:<branch>`).
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Optional `TimelineKind` discriminator filter.
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Lower bound on `valid_from_unix` (RFC-3339 / `YYYY-MM-DD`).
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Upper bound on `valid_from_unix` (RFC-3339 / `YYYY-MM-DD`).
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Cap on `recorded_at_unix` so the response reflects what was believed at that valid-time instant (ADR-018).
    pub as_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Max rows returned. Clamped to `[1, MAX_LIMIT]`.
    pub limit: Option<u32>,
}

/// Phase18 §4.4 — `GET /v1/timeline/{project}` handler.
pub async fn handle_timeline(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(q): Query<TimelineQuery>,
) -> Response {
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let as_of_unix = match parse_unix(q.as_of.as_deref()) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_input", format!("as_of: {e}")),
    };
    let from_unix = match parse_unix(q.from.as_deref()) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_input", format!("from: {e}")),
    };
    let to_unix = match parse_unix(q.to.as_deref()) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_input", format!("to: {e}")),
    };
    let branch_name = q.branch.as_deref().unwrap_or("main");
    let branch_id = compose_id(&project, branch_name);
    let limit = clamp_limit(q.limit);

    let mut clauses = vec![format!(
        "t.project_id = '{}'",
        sanitize_literal(&project)
    )];
    clauses.push(format!("t.branch_id = '{}'", sanitize_literal(&branch_id)));
    if let Some(k) = q.kind.as_deref().filter(|s| !s.is_empty()) {
        clauses.push(format!("t.kind = '{}'", sanitize_literal(k)));
    }
    if let Some(f) = from_unix {
        clauses.push(format!("t.valid_from_unix >= {f}"));
    }
    if let Some(t) = to_unix {
        clauses.push(format!("t.valid_from_unix <= {t}"));
    }
    if let Some(a) = as_of_unix {
        clauses.push(format!("t.recorded_at_unix <= {a}"));
    }
    let cypher = format!(
        "MATCH (t:TimelineEvent) WHERE {where_} \
         RETURN t.id AS id, t.kind AS kind, t.project_id AS project_id, \
                t.branch_id AS branch_id, t.entity_id AS entity_id, \
                t.valid_from_unix AS valid_from_unix, \
                t.recorded_at_unix AS recorded_at_unix, \
                t.title AS title, t.summary AS summary \
         ORDER BY t.valid_from_unix DESC LIMIT {limit}",
        where_ = clauses.join(" AND "),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => (
            StatusCode::OK,
            Json(json!({
                "project": project,
                "branch": branch_id,
                "filters": {
                    "as_of_unix": as_of_unix,
                    "kind": q.kind,
                    "from_unix": from_unix,
                    "to_unix": to_unix,
                    "limit": limit,
                },
                "events": key_rows(&out.columns, &out.rows),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// GET /v1/branch/{project}
// ============================================================================

/// Phase18 §4.4 — `GET /v1/branch/{project}` handler.
pub async fn handle_branch_list(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Response {
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let cypher = format!(
        "MATCH (b:{label}) WHERE b.project_id = '{proj}' \
         RETURN b.id AS id, b.name AS name, b.parent_branch_id AS parent_branch_id, \
                b.status AS status, b.merge_strategy AS merge_strategy, \
                b.created_at AS created_at, b.created_by AS created_by \
         ORDER BY b.created_at ASC",
        label = BRANCH_LABEL,
        proj = sanitize_literal(&project),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => (
            StatusCode::OK,
            Json(json!({
                "project": project,
                "branches": key_rows(&out.columns, &out.rows),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// POST /v1/branch/{project}  — create
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
/// Request body for `POST /v1/branch/{project}`.
pub struct BranchCreateBody {
    /// Branch name to create (validated by ADR-019 regex).
    pub name: String,
    /// Lower bound on `valid_from_unix` (RFC-3339 / `YYYY-MM-DD`).
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Optional fork anchor in valid-time (RFC-3339 / `YYYY-MM-DD`).
    pub valid_time: Option<String>,
}

/// Phase18 §4.4 — `POST /v1/branch/{project}` handler.
pub async fn handle_branch_create(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<BranchCreateBody>,
) -> Response {
    if let Err(e) = validate_branch_name(&body.name) {
        return err(StatusCode::BAD_REQUEST, "bad_input", format!("name: {e}"));
    }
    if body.from != "main" {
        if let Err(e) = validate_branch_name(&body.from) {
            return err(StatusCode::BAD_REQUEST, "bad_input", format!("from: {e}"));
        }
    }
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let child_id = compose_id(&project, &body.name);
    let parent_id = compose_id(&project, &body.from);
    let now_rfc3339 = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let fork_lit = match body.valid_time.as_deref().map(normalize_rfc3339).transpose() {
        Ok(v) => v
            .flatten()
            .map(|s| format!("'{}'", sanitize_literal(&s)))
            .unwrap_or_else(|| "NULL".to_string()),
        Err(e) => {
            return err(
                StatusCode::BAD_REQUEST,
                "bad_input",
                format!("valid_time: {e}"),
            )
        }
    };
    let cypher = format!(
        "MERGE (parent:{label} {{ id: '{parent}' }}) \
         MERGE (b:{label} {{ id: '{child}' }}) \
         ON CREATE SET b.project_id = '{proj}', b.name = '{name}', \
                       b.parent_branch_id = '{parent}', \
                       b.fork_valid_time = {fork_lit}, \
                       b.status = 'active', \
                       b.created_at = '{now}', \
                       b.created_by = 'cortex-api' \
         MERGE (b)-[:FORKED_FROM]->(parent) \
         RETURN b",
        label = BRANCH_LABEL,
        parent = sanitize_literal(&parent_id),
        child = sanitize_literal(&child_id),
        proj = sanitize_literal(&project),
        name = sanitize_literal(&body.name),
        now = sanitize_literal(&now_rfc3339),
        fork_lit = fork_lit,
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => (
            StatusCode::OK,
            Json(json!({
                "branch_id": child_id,
                "parent_branch_id": parent_id,
                "status": "active",
                "result": key_rows(&out.columns, &out.rows).into_iter().next(),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// GET /v1/branch/{project}/{branch}
// ============================================================================

/// Phase18 §4.4 — `GET /v1/branch/{project}/{branch}` handler.
pub async fn handle_branch_show(
    State(state): State<ApiState>,
    Path((project, branch)): Path<(String, String)>,
) -> Response {
    if branch != "main" {
        if let Err(e) = validate_branch_name(&branch) {
            return err(StatusCode::BAD_REQUEST, "bad_input", format!("branch: {e}"));
        }
    }
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let id = compose_id(&project, &branch);
    let cypher = format!(
        "MATCH (b:{label} {{ id: '{id}' }}) RETURN b",
        label = BRANCH_LABEL,
        id = sanitize_literal(&id),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => {
            if out.rows.is_empty() {
                return err(
                    StatusCode::NOT_FOUND,
                    "branch_not_found",
                    format!("{id:?} not found"),
                );
            }
            (
                StatusCode::OK,
                Json(json!({ "branch_id": id, "branch": key_rows(&out.columns, &out.rows).into_iter().next() })),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// POST /v1/branch/{project}/{branch}/merge
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
/// Request body for `POST /v1/branch/{project}/{branch}/merge`.
pub struct BranchMergeBody {
    /// Merge strategy: `accept` / `partial` / `discard` (ADR-021 §1.4).
    pub strategy: String,
}

/// Phase18 §4.4 — `POST /v1/branch/{project}/{branch}/merge` handler.
pub async fn handle_branch_merge(
    State(state): State<ApiState>,
    Path((project, branch)): Path<(String, String)>,
    Json(body): Json<BranchMergeBody>,
) -> Response {
    if let Err(e) = validate_branch_name(&branch) {
        return err(StatusCode::BAD_REQUEST, "bad_input", format!("branch: {e}"));
    }
    if !matches!(body.strategy.as_str(), "accept" | "partial" | "discard") {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!("strategy must be accept|partial|discard, got {:?}", body.strategy),
        );
    }
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let child_id = compose_id(&project, &branch);
    let lookup_cypher = format!(
        "MATCH (b:{label} {{ id: '{id}' }}) RETURN b.parent_branch_id AS parent_branch_id",
        label = BRANCH_LABEL,
        id = sanitize_literal(&child_id),
    );
    let parent_id = match nexus.execute_cypher(&lookup_cypher, None).await {
        Ok(out) => out
            .rows
            .first()
            .and_then(|r| r.get("parent_branch_id").and_then(|v| v.as_str()).map(String::from)),
        Err(e) => return err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    };
    let Some(parent_id) = parent_id else {
        return err(
            StatusCode::NOT_FOUND,
            "branch_not_found",
            format!("{child_id:?} either does not exist or carries no parent_branch_id"),
        );
    };
    let now_rfc3339 = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let merge_event_id = format!(
        "MRG-{}",
        now_rfc3339
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
    );
    let cypher = format!(
        "MATCH (b:{label} {{ id: '{child}' }}), (parent:{label} {{ id: '{parent}' }}) \
         SET b.status = 'merged', \
             b.merge_strategy = '{strategy}', \
             b.merge_point_event_id = '{merge_event}' \
         MERGE (b)-[r:MERGED_INTO]->(parent) \
         SET r.strategy = '{strategy}', \
             r.merge_point_event_id = '{merge_event}' \
         RETURN b",
        label = BRANCH_LABEL,
        child = sanitize_literal(&child_id),
        parent = sanitize_literal(&parent_id),
        strategy = body.strategy,
        merge_event = sanitize_literal(&merge_event_id),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => (
            StatusCode::OK,
            Json(json!({
                "branch_id": child_id,
                "parent_branch_id": parent_id,
                "strategy": body.strategy,
                "merge_point_event_id": merge_event_id,
                "merged_at": now_rfc3339,
                "result": key_rows(&out.columns, &out.rows).into_iter().next(),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// POST /v1/branch/{project}/{branch}/abandon
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
/// Request body for `POST /v1/branch/{project}/{branch}/abandon`.
pub struct BranchAbandonBody {
    /// Free-text abandonment reason (required by ADR-022).
    pub reason: String,
}

/// Phase18 §4.4 — `POST /v1/branch/{project}/{branch}/abandon` handler.
pub async fn handle_branch_abandon(
    State(state): State<ApiState>,
    Path((project, branch)): Path<(String, String)>,
    Json(body): Json<BranchAbandonBody>,
) -> Response {
    if let Err(e) = validate_branch_name(&branch) {
        return err(StatusCode::BAD_REQUEST, "bad_input", format!("branch: {e}"));
    }
    if body.reason.trim().is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "reason must be non-empty (ADR-022)",
        );
    }
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let id = compose_id(&project, &branch);
    let cypher = format!(
        "MATCH (b:{label} {{ id: '{id}' }}) \
         SET b.status = 'abandoned', b.abandonment_reason = '{reason}' \
         RETURN b",
        label = BRANCH_LABEL,
        id = sanitize_literal(&id),
        reason = sanitize_literal(&body.reason),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => {
            if out.rows.is_empty() {
                return err(
                    StatusCode::NOT_FOUND,
                    "branch_not_found",
                    format!("{id:?} not found"),
                );
            }
            (
                StatusCode::OK,
                Json(json!({
                    "branch_id": id,
                    "status": "abandoned",
                    "abandonment_reason": body.reason,
                    "result": key_rows(&out.columns, &out.rows).into_iter().next(),
                })),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// GET /v1/entity/{id}/history
// ============================================================================

#[derive(Debug, Clone, Default, Deserialize)]
/// Query params for `GET /v1/entity/{id}/history`.
pub struct EntityHistoryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Cap on `recorded_at_unix` so the response reflects what was believed at that valid-time instant (ADR-018).
    pub as_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Max rows returned. Clamped to `[1, MAX_LIMIT]`.
    pub limit: Option<u32>,
}

/// Phase18 §4.4 — `GET /v1/entity/{id}/history` handler.
pub async fn handle_entity_history(
    State(state): State<ApiState>,
    Path(entity_id): Path<String>,
    Query(q): Query<EntityHistoryQuery>,
) -> Response {
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let as_of_unix = match parse_unix(q.as_of.as_deref()) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_input", format!("as_of: {e}")),
    };
    let limit = clamp_limit(q.limit);
    let mut clauses = vec![format!(
        "t.entity_id = '{}'",
        sanitize_literal(&entity_id)
    )];
    if let Some(a) = as_of_unix {
        clauses.push(format!("t.recorded_at_unix <= {a}"));
    }
    let cypher = format!(
        "MATCH (t:TimelineEvent) WHERE {where_} \
         RETURN t.id AS id, t.kind AS kind, t.branch_id AS branch_id, \
                t.valid_from_unix AS valid_from_unix, \
                t.recorded_at_unix AS recorded_at_unix, \
                t.title AS title, t.summary AS summary \
         ORDER BY t.valid_from_unix DESC LIMIT {limit}",
        where_ = clauses.join(" AND "),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => (
            StatusCode::OK,
            Json(json!({
                "entity_id": entity_id,
                "as_of_unix": as_of_unix,
                "events": key_rows(&out.columns, &out.rows),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

// ============================================================================
// GET /v1/entity/{id}/supersession
// ============================================================================

/// Phase18 §4.4 — `GET /v1/entity/{id}/supersession` handler.
pub async fn handle_entity_supersession(
    State(state): State<ApiState>,
    Path(entity_id): Path<String>,
) -> Response {
    let Some(nexus) = state.nexus.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "nexus_unconfigured",
            "cortex-api was started without a Nexus client",
        );
    };
    let cypher = format!(
        "OPTIONAL MATCH p_path = (this {{ id: '{id}' }})-[:SUPERSEDES*0..10]->(predecessor) \
         WITH this, [n IN nodes(p_path) | {{ id: n.id, kind: head(labels(n)) }}] AS predecessors \
         OPTIONAL MATCH s_path = (successor)-[:SUPERSEDES*0..10]->(this) \
         RETURN this.id AS id, head(labels(this)) AS label, \
                predecessors, \
                [n IN nodes(s_path) | {{ id: n.id, kind: head(labels(n)) }}] AS successors",
        id = sanitize_literal(&entity_id),
    );
    match nexus.execute_cypher(&cypher, None).await {
        Ok(out) => {
            if out.rows.is_empty() {
                return err(
                    StatusCode::NOT_FOUND,
                    "entity_not_found",
                    format!("{entity_id:?} not found"),
                );
            }
            (
                StatusCode::OK,
                Json(json!({
                    "entity_id": entity_id,
                    "lineage": key_rows(&out.columns, &out.rows).into_iter().next(),
                })),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
    }
}

fn normalize_rfc3339(raw: &str) -> Result<Option<String>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| Some(dt.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string()))
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unix_handles_full_rfc3339_and_day_precision() {
        let full = parse_unix(Some("2026-04-01T00:00:00Z")).unwrap().unwrap();
        let day = parse_unix(Some("2026-04-01")).unwrap().unwrap();
        assert_eq!(full, day);
    }

    #[test]
    fn parse_unix_returns_none_on_empty() {
        assert!(parse_unix(None).unwrap().is_none());
        assert!(parse_unix(Some("")).unwrap().is_none());
        assert!(parse_unix(Some("  ")).unwrap().is_none());
    }

    #[test]
    fn parse_unix_rejects_garbage() {
        assert!(parse_unix(Some("xxx")).is_err());
    }

    #[test]
    fn sanitize_literal_drops_quote_backslash_newline() {
        assert_eq!(sanitize_literal("a'b\"c\\d\ne"), "abcde");
    }

    #[test]
    fn clamp_limit_clamps_to_range() {
        assert_eq!(clamp_limit(None), DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(1_000)), MAX_LIMIT);
    }

    #[test]
    fn normalize_rfc3339_round_trips_full_and_day() {
        assert_eq!(
            normalize_rfc3339("2026-04-01").unwrap().as_deref(),
            Some("2026-04-01T00:00:00Z")
        );
    }
}
