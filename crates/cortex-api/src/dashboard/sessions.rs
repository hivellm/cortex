use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{clip, collect_lane_hits, session_id_of, symbol_to_kind, DashboardState, KindCount};

/// One session row — aggregated from the lane hits' `session_id`
/// extras stamp. The dashboard sidebar lists these so the user can
/// click to filter Timeline / Memory views.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// Canonical 26-char ULID.
    pub session_id: String,
    /// Total events captured under this session.
    pub event_count: u64,
    /// Per-kind breakdown.
    pub kind_breakdown: Vec<KindCount>,
    /// `ts` of the earliest event we have (ms epoch). 0 if missing.
    pub started_at_ms: i64,
    /// `ts` of the latest event we have (ms epoch).
    pub last_event_ms: i64,
    /// Duration in milliseconds between earliest and latest event.
    pub duration_ms: i64,
    /// Repos touched by this session.
    pub repos: Vec<String>,
    /// Title surface — first turn's user_message clipped at 80 chars.
    /// Empty when no turn was captured under this session.
    pub title: String,
}

pub(super) async fn sessions(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Group by session_id.
    let mut groups: std::collections::BTreeMap<String, Vec<crate::lanes::LaneHit>> =
        std::collections::BTreeMap::new();
    for h in hits {
        if let Some(sid) = session_id_of(&h) {
            groups.entry(sid.to_string()).or_default().push(h);
        }
    }

    let mut rows: Vec<SessionRow> = groups
        .into_iter()
        .map(|(session_id, mut bucket)| {
            // Sort oldest → newest so [0] is the earliest event.
            bucket.sort_by_key(|h| h.ts);
            let started_at_ms = bucket.first().map(|h| h.ts).unwrap_or(0);
            let last_event_ms = bucket.last().map(|h| h.ts).unwrap_or(0);
            let duration_ms = (last_event_ms - started_at_ms).max(0);

            let mut by_kind: std::collections::BTreeMap<&'static str, u64> =
                std::collections::BTreeMap::new();
            let mut repos: std::collections::BTreeSet<String> = Default::default();
            let mut title = String::new();
            for h in &bucket {
                let kind = symbol_to_kind(h.symbol.as_deref());
                *by_kind.entry(kind).or_insert(0) += 1;
                if let Some(r) = h.repo.as_deref() {
                    repos.insert(r.to_string());
                }
                if title.is_empty() && kind == "turn" {
                    title = clip(h.text.lines().next().unwrap_or(""), 80);
                }
            }

            SessionRow {
                session_id,
                event_count: bucket.len() as u64,
                kind_breakdown: by_kind
                    .into_iter()
                    .map(|(k, count)| KindCount {
                        kind: k.to_string(),
                        count,
                    })
                    .collect(),
                started_at_ms,
                last_event_ms,
                duration_ms,
                repos: repos.into_iter().collect(),
                title,
            }
        })
        .collect();

    // Sort by most-recent-activity, descending.
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_event_ms));
    (StatusCode::OK, Json(rows)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn make_state(hits: Vec<crate::lanes::LaneHit>) -> super::super::DashboardState {
        let lane = crate::lanes::MemoryKeywordLane::new();
        lane.seed("cortex-code", hits);
        super::super::DashboardState {
            lane: Arc::new(lane),
            nexus: None,
            analyzer: Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: Arc::new(crate::tasks_loader::MultiTaskLoader::new(vec![
                crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from(
                    "__tests_no_rulebook__",
                )),
            ])),
            metadata: None,
            loader_metrics: Arc::new(crate::LoaderMetrics::new()),
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
        }
    }

    fn turn_hit_in(session: &str, text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String(session.to_string()),
        );
        crate::lanes::LaneHit {
            doc_id: format!("archive|{}", text),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[tokio::test]
    async fn sessions_groups_by_session_id_and_sorts_by_recency() {
        let state = make_state(vec![
            turn_hit_in("01SESSIONA0000000000000001", "first ever", "Cortex", 100),
            turn_hit_in(
                "01SESSIONA0000000000000001",
                "still session A",
                "Cortex",
                200,
            ),
            turn_hit_in(
                "01SESSIONB0000000000000002",
                "session B latest",
                "Vectorizer",
                500,
            ),
        ]);
        let resp = sessions(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["session_id"], "01SESSIONB0000000000000002");
        assert_eq!(rows[0]["event_count"], 1);
        assert_eq!(rows[0]["title"], "session B latest");
        assert_eq!(rows[1]["session_id"], "01SESSIONA0000000000000001");
        assert_eq!(rows[1]["event_count"], 2);
        assert_eq!(rows[1]["duration_ms"], 100);
        assert_eq!(rows[1]["title"], "first ever");
    }
}
