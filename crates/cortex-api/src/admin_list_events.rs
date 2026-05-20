//! phase11w — admin `/v1/admin/list-events` endpoint.
//!
//! The cron-driven `cortex-ops tool-call-digest --apply` walks the
//! lane snapshot to find old tool_call envelopes that need
//! summarising + purging. The Meilisearch indexes the keyword lane
//! is built from do not carry tool_call rows directly (the live
//! Meili boot only fans out turns / decisions / memories /
//! analyses / violations); the canonical source is the parquet
//! archive that the lane is seeded from at boot. Re-walking parquet
//! from the cron binary would mean re-implementing the
//! `archive_loader` here, so we expose the lane's event view
//! through a small admin endpoint instead.
//!
//! The endpoint is a read-only projection: every row carries the
//! `event_id` (parsed from the archive-stamped `doc_id =
//! "archive|<event_id>"` shape), the canonical `kind`, the
//! `occurred_at` RFC-3339 timestamp the lane stamped, the `repo`
//! slug, and the `tool` name (extracted from the `symbol =
//! "tool_call:<Name>"` convention). Filters restrict to `kind`, an
//! upper-bound `before` timestamp, and a row cap.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::lanes::MemoryKeywordLane;

/// Query params for `/v1/admin/list-events`.
#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    /// Required: canonical kind (`tool_call`, `turn`, `memory`, …).
    pub kind: String,
    /// Optional RFC-3339 upper bound on `occurred_at`. When set,
    /// every row in the response satisfies `occurred_at < before`.
    #[serde(default)]
    pub before: Option<String>,
    /// Maximum rows to return. Defaults to 1 000, capped at 50 000
    /// so the cron can paginate large backlogs without blowing
    /// memory in either direction.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Optional repo filter — restricts to one or more repos. Each
    /// `repo=<name>` query param is appended; canonicalised via
    /// `cortex_storage::names::slug_for_repo` before comparison.
    #[serde(default)]
    pub repo: Vec<String>,
}

/// One row in the `/v1/admin/list-events` response.
#[derive(Debug, Clone, Serialize)]
pub struct ListedEvent {
    /// Globally unique event id.
    pub event_id: String,
    /// Canonical kind.
    pub kind: String,
    /// RFC-3339 timestamp the lane stamped.
    pub occurred_at: String,
    /// Repo slug (canonicalised).
    pub repo: Option<String>,
    /// Tool name when the row is a `tool_call` whose
    /// `symbol = "tool_call:<Name>"` shape resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// `payload.summarized_by` when set — lets the digest's
    /// idempotence guard short-circuit already-digested events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summarized_by: Option<String>,
}

/// Handler. Pulls the keyword lane out of the dashboard state so
/// the route mounts on the same router as the rest of the
/// dashboard endpoints.
pub async fn handle_list_events(
    State(state): State<crate::dashboard::DashboardState>,
    Query(params): Query<ListEventsQuery>,
) -> Response {
    let lane = state.lane.clone();
    handle_list_events_with_lane(lane, params).await
}

/// Lane-typed handler. Test-friendly entrypoint that does not need
/// a full `DashboardState`.
pub async fn handle_list_events_with_lane(
    lane: Arc<MemoryKeywordLane>,
    params: ListEventsQuery,
) -> Response {
    let limit = params.limit.unwrap_or(1_000).clamp(1, 50_000);
    let kind_filter = params.kind.trim().to_string();
    if kind_filter.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "kind is required"})),
        )
            .into_response();
    }
    let before_filter_ms = params
        .before
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc).timestamp_millis());
    let repo_allow: std::collections::HashSet<String> = params
        .repo
        .iter()
        .map(|r| cortex_storage::names::slug_for_repo(r))
        .collect();

    let snapshot: Vec<crate::lanes::LaneHit> = match lane.hits.lock() {
        Ok(g) => g.values().flatten().cloned().collect(),
        Err(p) => p.into_inner().values().flatten().cloned().collect(),
    };

    let mut out: Vec<ListedEvent> = Vec::new();
    for h in snapshot {
        // Project the lane symbol onto the canonical kind list.
        let row_kind = symbol_to_kind(h.symbol.as_deref());
        if row_kind != kind_filter {
            continue;
        }
        let event_id = match h.doc_id.strip_prefix("archive|") {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        if let Some(cutoff_ms) = before_filter_ms {
            if h.ts >= cutoff_ms {
                continue;
            }
        }
        let repo_slug = h
            .repo
            .as_deref()
            .map(cortex_storage::names::slug_for_repo);
        if !repo_allow.is_empty() {
            match repo_slug.as_deref() {
                Some(r) if repo_allow.contains(r) => {}
                _ => continue,
            }
        }
        let tool = h
            .symbol
            .as_deref()
            .and_then(|s| s.strip_prefix("tool_call:"))
            .map(|s| s.to_string());
        let summarized_by = h
            .extras
            .get("summarized_by")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let occurred_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(h.ts)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        out.push(ListedEvent {
            event_id,
            kind: row_kind.to_string(),
            occurred_at,
            repo: repo_slug,
            tool,
            summarized_by,
        });
        if out.len() >= limit {
            break;
        }
    }
    // Newest first so callers paginating with `before=<ts>` walk
    // backwards through history.
    out.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    (StatusCode::OK, Json(out)).into_response()
}

/// Map the `LaneHit.symbol` field onto the canonical kind list. Mirrors
/// `dashboard::symbol_to_kind` (kept inline so this module does not
/// pull a parent's `pub(super)` helper).
fn symbol_to_kind(symbol: Option<&str>) -> &'static str {
    match symbol {
        Some(s) if s.starts_with("tool_call") => "tool_call",
        Some(s) if s.starts_with("agent_call") => "agent_call",
        Some("turn") => "turn",
        Some("decision") => "decision",
        Some("law_violation") => "law_violation",
        Some("analysis") => "analysis",
        Some(s) if s.starts_with("memory") => "memory",
        _ => "turn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::{LaneHit, MemoryKeywordLane};
    use std::collections::BTreeMap;

    fn hit(event_id: &str, symbol: &str, repo: &str, ts_ms: i64) -> LaneHit {
        LaneHit {
            doc_id: format!("archive|{event_id}"),
            text: format!("text-{event_id}"),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some(symbol.to_string()),
            content_hash: None,
            score: 1.0,
            ts: ts_ms,
            severity: None,
            extras: BTreeMap::new(),
            overlay: crate::lanes::Overlay::default(),
        }
    }

    fn lane_seeded(rows: Vec<LaneHit>) -> Arc<MemoryKeywordLane> {
        let lane = Arc::new(MemoryKeywordLane::new());
        lane.seed("cortex_archive", rows);
        lane
    }

    #[tokio::test]
    async fn list_events_returns_only_requested_kind() {
        let lane = lane_seeded(vec![
            hit("01TC0001", "tool_call:Bash", "cortex", 1_000),
            hit("01TC0002", "tool_call:Read", "cortex", 2_000),
            hit("01TURN001", "turn", "cortex", 3_000),
        ]);
        let resp = handle_list_events_with_lane(
            lane,
            ListEventsQuery {
                kind: "tool_call".to_string(),
                before: None,
                limit: None,
                repo: vec![],
            },
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(arr.iter().all(|r| r["kind"] == "tool_call"));
        assert!(arr.iter().any(|r| r["tool"] == "Bash"));
        assert!(arr.iter().any(|r| r["tool"] == "Read"));
    }

    #[tokio::test]
    async fn list_events_honours_before_cutoff() {
        let now = chrono::Utc::now();
        let cutoff_iso = now.to_rfc3339();
        let cutoff_ms = now.timestamp_millis();
        let lane = lane_seeded(vec![
            hit("01OLD", "tool_call:Bash", "cortex", cutoff_ms - 60_000),
            hit("01FRESH", "tool_call:Bash", "cortex", cutoff_ms + 60_000),
        ]);
        let resp = handle_list_events_with_lane(
            lane,
            ListEventsQuery {
                kind: "tool_call".to_string(),
                before: Some(cutoff_iso),
                limit: None,
                repo: vec![],
            },
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1, "only the row older than `before` survives");
        assert_eq!(arr[0]["event_id"], "01OLD");
    }

    #[tokio::test]
    async fn list_events_caps_at_limit() {
        let lane = lane_seeded(
            (0..20)
                .map(|i| hit(&format!("01TC{i:04}"), "tool_call:Bash", "cortex", i as i64))
                .collect(),
        );
        let resp = handle_list_events_with_lane(
            lane,
            ListEventsQuery {
                kind: "tool_call".to_string(),
                before: None,
                limit: Some(5),
                repo: vec![],
            },
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 5);
    }
}
