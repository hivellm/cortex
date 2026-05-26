//! Phase19 §1.4 — `GET /v1/sessions/{session_id}/files-touched` and
//! `POST /v1/search/files-touched` handlers.
//!
//! Aggregate every file path that appeared in any ToolCall's
//! `touched` array (or in the input fallback when `touched` was
//! empty) for one session — or for a `(repo, since, until)`
//! window. Returns per-path counters: `read_count` / `write_count`
//! / `other_count` / `last_touched_ts`.
//!
//! Reads from the on-disk Parquet archive via
//! `cortex_storage::archive::scan_envelopes_by_session`
//! (per-session) or `walk_envelopes` (window mode). The scan is
//! O(archive_size), so the handler clamps the response to
//! `limit <= 100` paths per call — callers narrow with `repo` /
//! `since` / `until` if the corpus is big.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::{
    extract::{Path as PathExtract, Query, State},
    http::StatusCode,
    response::Response,
    Json,
};
use cortex_core::events::{Envelope, Kind, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default cap when caller does not specify `limit`.
const LIMIT_DEFAULT: u32 = 50;
/// Hard ceiling on paths returned per call.
const LIMIT_MAX: u32 = 100;

/// Query params for the per-session route.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FilesTouchedQuery {
    /// Cap on rows. Defaults to [`LIMIT_DEFAULT`]; clamped to
    /// [`LIMIT_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Body for the cross-session POST route.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FilesTouchedWindow {
    /// Optional repo filter. When absent the scan covers every repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// RFC3339 lower bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// RFC3339 upper bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Cap on emitted paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Per-path counters.
#[derive(Debug, Clone, Serialize, Default)]
pub struct FileTouchRow {
    /// Forward-slash path (whatever the adapter stamped).
    pub path: String,
    /// `kind == "read"` (or `"Read"`) hits — case-folded.
    pub read_count: u32,
    /// `kind == "write"` / `"edit"` / `"create"` hits.
    pub write_count: u32,
    /// Anything else (delete, search, etc).
    pub other_count: u32,
    /// RFC3339 of the most recent touch.
    pub last_touched_ts: String,
}

/// Response body.
#[derive(Debug, Clone, Serialize)]
pub struct FilesTouchedResponse {
    /// Number of paths returned (≤ `limit`).
    pub count: u32,
    /// `true` when the underlying scan had more paths than `limit`.
    pub truncated: bool,
    /// Per-path rows, sorted by total touches desc.
    pub paths: Vec<FileTouchRow>,
}

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

#[allow(clippy::manual_clamp)]
fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(LIMIT_DEFAULT).min(LIMIT_MAX).max(1)
}

fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn archive_root(state: &ApiState) -> PathBuf {
    if let Some(root) = state.cfg.ingestion.archive_root.as_deref() {
        if !root.trim().is_empty() {
            return PathBuf::from(root);
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cortex").join("archive")
}

/// Classify a `TouchedArtifact.kind` into read / write / other.
/// The adapter writes free-form strings; this normalises so the
/// counters stay stable across adapter versions.
pub(crate) fn classify_op(kind: &str) -> OpClass {
    match kind.to_ascii_lowercase().as_str() {
        "read" | "view" | "open" | "stat" => OpClass::Read,
        "write" | "edit" | "create" | "update" | "modify" | "patch" | "append" => OpClass::Write,
        _ => OpClass::Other,
    }
}

/// Op classification surfaced to the aggregator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpClass {
    /// Read-flavoured op.
    Read,
    /// Write-flavoured op.
    Write,
    /// Anything else.
    Other,
}

/// Extract `(path, kind)` pairs from one ToolCall payload. Falls
/// back to `input.path` / `input.file_path` for adapters that do
/// not stamp `touched` (older versions / minimal shims).
pub(crate) fn extract_touched(tc: &ToolCall) -> Vec<(String, String)> {
    if !tc.touched.is_empty() {
        return tc
            .touched
            .iter()
            .map(|t| (t.path.clone(), t.kind.clone()))
            .collect();
    }
    let mut out = Vec::new();
    for key in ["path", "file_path", "filename", "file"] {
        if let Some(s) = tc.input.get(key).and_then(Value::as_str) {
            // Best-effort kind inference from the tool name.
            let inferred = match tc.tool_name.as_str() {
                "Read" | "View" => "read",
                "Write" | "Edit" | "Create" | "Patch" => "write",
                _ => "other",
            };
            out.push((s.to_string(), inferred.to_string()));
            break;
        }
    }
    out
}

/// Aggregate `(path, kind, ts)` triples into a sorted vector of
/// `FileTouchRow`. `paths_for` collects raw triples per envelope;
/// `fold_paths` does the BTreeMap fold + sort.
pub(crate) fn fold_paths(triples: &[(String, String, String)]) -> Vec<FileTouchRow> {
    let mut by_path: BTreeMap<String, FileTouchRow> = BTreeMap::new();
    for (path, kind, ts) in triples {
        let entry = by_path.entry(path.clone()).or_default();
        if entry.path.is_empty() {
            entry.path = path.clone();
        }
        match classify_op(kind) {
            OpClass::Read => entry.read_count += 1,
            OpClass::Write => entry.write_count += 1,
            OpClass::Other => entry.other_count += 1,
        }
        if ts.as_str() > entry.last_touched_ts.as_str() {
            entry.last_touched_ts = ts.clone();
        }
    }
    let mut rows: Vec<FileTouchRow> = by_path.into_values().collect();
    rows.sort_by(|a, b| {
        let a_total = a.read_count + a.write_count + a.other_count;
        let b_total = b.read_count + b.write_count + b.other_count;
        b_total
            .cmp(&a_total)
            .then_with(|| b.last_touched_ts.cmp(&a.last_touched_ts))
    });
    rows
}

/// Per-session handler — `GET /v1/sessions/{session_id}/files-touched`.
pub async fn handle_session_files_touched(
    State(state): State<ApiState>,
    PathExtract(session_id): PathExtract<String>,
    Query(params): Query<FilesTouchedQuery>,
) -> Response {
    use axum::response::IntoResponse;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "session_id path segment is required",
        );
    }
    let root = archive_root(&state);
    let limit = clamp_limit(params.limit);
    let session_for_blocking = session_id.clone();
    let scan = tokio::task::spawn_blocking(move || {
        cortex_storage::archive::scan_envelopes_by_session(&root, &session_for_blocking)
    })
    .await;
    let envs = match scan {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("archive scan: {e}"),
            );
        }
        Err(join_err) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("blocking task panic: {join_err}"),
            );
        }
    };
    let triples = paths_for(&envs);
    let mut rows = fold_paths(&triples);
    let total = rows.len() as u32;
    let truncated = total > limit;
    rows.truncate(limit as usize);
    Json(FilesTouchedResponse {
        count: rows.len() as u32,
        truncated,
        paths: rows,
    })
    .into_response()
}

/// Cross-session window handler — `POST /v1/search/files-touched`.
pub async fn handle_window_files_touched(
    State(state): State<ApiState>,
    Json(req): Json<FilesTouchedWindow>,
) -> Response {
    use axum::response::IntoResponse;
    let since_ms = req
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_rfc3339_to_ms);
    if let Some(None) = since_ms {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!("`{}` is not a valid RFC3339 timestamp", req.since.unwrap()),
        );
    }
    let until_ms = req
        .until
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_rfc3339_to_ms);
    if let Some(None) = until_ms {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            format!("`{}` is not a valid RFC3339 timestamp", req.until.unwrap()),
        );
    }
    let since_ms = since_ms.flatten();
    let until_ms = until_ms.flatten();

    let root = archive_root(&state);
    let repo = req
        .repo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let limit = clamp_limit(req.limit);

    let scan = tokio::task::spawn_blocking(move || {
        cortex_storage::archive::walk_envelopes(&root, |env| {
            if env.kind != Kind::ToolCall {
                return false;
            }
            if let Some(r) = &repo {
                if env.context.repo.as_deref() != Some(r.as_str()) {
                    return false;
                }
            }
            let ts_ms = parse_rfc3339_to_ms(&env.occurred_at);
            if let (Some(t), Some(lower)) = (ts_ms, since_ms) {
                if t < lower {
                    return false;
                }
            }
            if let (Some(t), Some(upper)) = (ts_ms, until_ms) {
                if t > upper {
                    return false;
                }
            }
            true
        })
    })
    .await;
    let envs = match scan {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("archive scan: {e}"),
            );
        }
        Err(join_err) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                format!("blocking task panic: {join_err}"),
            );
        }
    };
    let triples = paths_for(&envs);
    let mut rows = fold_paths(&triples);
    let total = rows.len() as u32;
    let truncated = total > limit;
    rows.truncate(limit as usize);
    Json(FilesTouchedResponse {
        count: rows.len() as u32,
        truncated,
        paths: rows,
    })
    .into_response()
}

/// Collect `(path, kind, ts)` triples from every ToolCall in `envs`.
/// Public-in-crate so unit tests can drive it without touching the
/// archive filesystem.
pub(crate) fn paths_for(envs: &[Envelope]) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for env in envs {
        if env.kind != Kind::ToolCall {
            continue;
        }
        let Ok(tc) = serde_json::from_value::<ToolCall>(env.payload.clone()) else {
            continue;
        };
        for (path, kind) in extract_touched(&tc) {
            out.push((path, kind, env.occurred_at.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::TouchedArtifact;

    #[test]
    fn classify_op_normalises_adapter_variants() {
        for r in ["read", "Read", "VIEW", "open", "stat"] {
            assert_eq!(classify_op(r), OpClass::Read);
        }
        for w in ["write", "Edit", "create", "PATCH", "modify", "append"] {
            assert_eq!(classify_op(w), OpClass::Write);
        }
        for o in ["delete", "search", "grep", ""] {
            assert_eq!(classify_op(o), OpClass::Other);
        }
    }

    #[test]
    fn extract_touched_prefers_touched_array_over_input_fallback() {
        let tc = ToolCall {
            tool_name: "Bash".into(),
            input: json!({"path": "/should-be-ignored"}),
            output: None,
            duration_ms: None,
            touched: vec![TouchedArtifact {
                kind: "read".into(),
                path: "/touched/a.rs".into(),
            }],
            outcome: "ok".into(),
        };
        let out = extract_touched(&tc);
        assert_eq!(out, vec![("/touched/a.rs".into(), "read".into())]);
    }

    #[test]
    fn extract_touched_falls_back_to_input_when_touched_empty() {
        let tc = ToolCall {
            tool_name: "Read".into(),
            input: json!({"path": "/from-input.rs"}),
            output: None,
            duration_ms: None,
            touched: vec![],
            outcome: "ok".into(),
        };
        let out = extract_touched(&tc);
        assert_eq!(out, vec![("/from-input.rs".into(), "read".into())]);

        let tc_edit = ToolCall {
            tool_name: "Edit".into(),
            input: json!({"file_path": "/edited.rs"}),
            output: None,
            duration_ms: None,
            touched: vec![],
            outcome: "ok".into(),
        };
        assert_eq!(
            extract_touched(&tc_edit),
            vec![("/edited.rs".into(), "write".into())]
        );
    }

    #[test]
    fn extract_touched_returns_empty_when_no_path_visible() {
        let tc = ToolCall {
            tool_name: "Bash".into(),
            input: json!({"command": "ls -la"}),
            output: None,
            duration_ms: None,
            touched: vec![],
            outcome: "ok".into(),
        };
        assert!(extract_touched(&tc).is_empty());
    }

    #[test]
    fn fold_paths_aggregates_per_path_and_sorts_by_total_desc() {
        let triples = vec![
            (
                "/a.rs".to_string(),
                "read".to_string(),
                "2026-05-26T07:00:00Z".to_string(),
            ),
            (
                "/a.rs".to_string(),
                "read".to_string(),
                "2026-05-26T07:05:00Z".to_string(),
            ),
            (
                "/a.rs".to_string(),
                "write".to_string(),
                "2026-05-26T07:10:00Z".to_string(),
            ),
            (
                "/b.rs".to_string(),
                "read".to_string(),
                "2026-05-26T07:01:00Z".to_string(),
            ),
            (
                "/c.rs".to_string(),
                "delete".to_string(),
                "2026-05-26T07:02:00Z".to_string(),
            ),
        ];
        let rows = fold_paths(&triples);
        assert_eq!(rows.len(), 3);
        // /a.rs has 3 touches → first.
        assert_eq!(rows[0].path, "/a.rs");
        assert_eq!(rows[0].read_count, 2);
        assert_eq!(rows[0].write_count, 1);
        assert_eq!(rows[0].other_count, 0);
        assert_eq!(rows[0].last_touched_ts, "2026-05-26T07:10:00Z");
        // /b.rs vs /c.rs both have 1; tie-break by ts desc → /c.rs first.
        assert_eq!(rows[1].path, "/c.rs");
        assert_eq!(rows[1].other_count, 1);
        assert_eq!(rows[2].path, "/b.rs");
        assert_eq!(rows[2].read_count, 1);
    }

    #[test]
    fn clamp_limit_floors_at_one_and_caps_at_hundred() {
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(10_000)), LIMIT_MAX);
        assert_eq!(clamp_limit(None), LIMIT_DEFAULT);
        assert_eq!(clamp_limit(Some(25)), 25);
    }
}
