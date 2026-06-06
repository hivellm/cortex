use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Datelike, Timelike};
use serde::Serialize;

use super::{collect_lane_hits, DashboardState};

/// Per-tool usage row — aggregated from `kind=tool_call` events
/// captured in the archive lane. Today populated by every PostToolUse
/// the spec-18 plugin emits.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStat {
    /// Tool name (`Edit`, `Read`, `Bash`, …).
    pub tool: String,
    /// Call count in the seeded window.
    pub calls: u64,
    /// Average duration in ms — placed at 0 until duration_ms is
    /// preserved through the lane (spec-12 derivation pipeline).
    pub avg_ms: u64,
    /// Error rate (0..1).
    pub err_rate: f64,
    /// Share of total calls (0..1).
    pub share: f64,
}

/// Top-level body of `/v1/dashboard/tools/stats`. Wraps the per-tool
/// rows the GUI table consumes plus the day×hour heatmap matrix the
/// design's Tool analytics view renders.
#[derive(Debug, Clone, Serialize)]
pub struct ToolsStatsBody {
    /// Per-tool aggregates, descending by call count.
    pub tools: Vec<ToolStat>,
    /// Tool-call density per (weekday, hour-of-day) over the last
    /// 7 days, UTC. `cells[d][h]` is the count for weekday `d` at
    /// hour `h`. Days follow ISO numbering (Monday = 0).
    pub heatmap: HeatmapBlock,
}

/// 7×24 heatmap of tool-call counts.
#[derive(Debug, Clone, Serialize)]
pub struct HeatmapBlock {
    /// Always `"UTC"` — buckets read off `chrono::DateTime<Utc>`.
    pub tz: &'static str,
    /// Day labels in display order, matching the row dimension of
    /// `cells`.
    pub days: [&'static str; 7],
    /// `[7][24]` tool-call counts. Outer index is weekday (0 = Mon),
    /// inner index is hour of day. Buckets with no calls are zero.
    pub cells: Vec<Vec<u64>>,
}

pub(super) async fn tools_stats(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let mut by_tool: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for h in &hits {
        if let Some(s) = h.symbol.as_deref() {
            if let Some(tool) = s.strip_prefix("tool_call:") {
                *by_tool.entry(tool.to_string()).or_insert(0) += 1;
            }
        }
    }
    let total: u64 = by_tool.values().sum();
    let total_f = if total == 0 { 1.0 } else { total as f64 };
    let mut rows: Vec<ToolStat> = by_tool
        .into_iter()
        .map(|(tool, calls)| ToolStat {
            tool,
            calls,
            avg_ms: 0,
            err_rate: 0.0,
            share: calls as f64 / total_f,
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.calls));

    let heatmap = build_tool_heatmap(&hits);
    let body = ToolsStatsBody {
        tools: rows,
        heatmap,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Bucket every `tool_call:*` envelope captured in the last 7 days
/// into a 7×24 grid of `(weekday, hour)` UTC counts. Hits with no
/// timestamp are dropped — they cannot be placed honestly.
fn build_tool_heatmap(hits: &[crate::lanes::LaneHit]) -> HeatmapBlock {
    let now = chrono::Utc::now();
    let cutoff_ms = now.timestamp_millis() - 7 * 86_400_000;
    let mut cells = vec![vec![0u64; 24]; 7];
    for h in hits {
        let symbol = match h.symbol.as_deref() {
            Some(s) if s.starts_with("tool_call:") => s,
            _ => continue,
        };
        let _ = symbol; // explicit kept-handle — keeps the filter readable
        if h.ts <= 0 || h.ts < cutoff_ms {
            continue;
        }
        let dt = match chrono::DateTime::<chrono::Utc>::from_timestamp_millis(h.ts) {
            Some(dt) => dt,
            None => continue,
        };
        let weekday = dt.weekday().num_days_from_monday() as usize;
        let hour = dt.hour() as usize;
        if weekday < 7 && hour < 24 {
            cells[weekday][hour] += 1;
        }
    }
    HeatmapBlock {
        tz: "UTC",
        days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        cells,
    }
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
            temporal_metrics: Arc::new(crate::TemporalMetrics::new()),
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
        }
    }

    fn tool_call_hit(tool: &str, body: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
            doc_id: format!("archive|{tool}-{ts}"),
            text: format!("[{tool}] {body}"),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some(format!("tool_call:{tool}")),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[tokio::test]
    async fn tools_stats_emits_seven_by_twentyfour_heatmap() {
        let state = make_state(vec![
            tool_call_hit("Edit", "x", "Cortex", 100),
            tool_call_hit("Read", "y", "Cortex", 200),
        ]);
        let resp = tools_stats(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["heatmap"]["tz"], "UTC");
        let days = parsed["heatmap"]["days"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[0], "Mon");
        let cells = parsed["heatmap"]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 7);
        for row in cells {
            assert_eq!(row.as_array().unwrap().len(), 24);
        }
    }
}
