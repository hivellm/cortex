use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{
    collect_lane_hits, normalize_repo, symbol_to_kind, DashboardState, KindCount, RepoCount,
};

/// Counters + per-kind breakdown derived from the seeded lane. Shape
/// matches what the prototype's overview surfaces consume.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewBody {
    /// Total events visible to the dashboard (sum across canonical
    /// indexes — `cortex-code` is the seeded one today).
    pub events_total: u64,
    /// Distinct repos seen in the lane.
    pub repos_indexed: u64,
    /// Per-kind event counts (`turn`, `tool_call`, `agent_call`).
    pub kind_breakdown: Vec<KindCount>,
    /// Top repos by event count, descending. Capped at 8.
    pub recent_repos: Vec<RepoCount>,
    /// Time-bucketed series the GUI's stats grid renders. Only
    /// quantities derivable from the captured lane are emitted —
    /// see [`SeriesBlock`].
    pub series: SeriesBlock,
    /// Honest "we don't have this signal yet" flag for the
    /// `series.classifier_cost_usd_today` ribbon. Always `true`
    /// today; flips to `false` once the spec-05 classifier worker
    /// stamps per-event cost on the lane and the API can sum them.
    /// The wire shape stays stable across the cut-over so clients
    /// don't need a v2 endpoint.
    pub classifier_cost_unavailable_until_spec05: bool,
}

/// Time-bucketed series block. Each array carries a fixed number of
/// buckets, oldest-first. `events_per_min` and `violations_7d_daily`
/// have no nulls — empty buckets are zero so the front-end Sparkline
/// renders a gap-free line. `pre_thinking_p95_ms` carries `null`
/// for buckets where no envelope landed with a duration stamp so
/// the renderer draws a gap rather than a dishonest zero.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesBlock {
    /// Total events per minute over the last 20 minutes.
    pub events_per_min: Vec<u64>,
    /// P95 duration (ms) per minute over the last 20 minutes. The
    /// signal source is `tool_call` + `agent_call` envelopes that
    /// carry `duration_ms`; turns alone don't populate the series
    /// today (spec-12 derivation pipeline owns turn-level latency
    /// and will widen the source set when it ships).
    pub pre_thinking_p95_ms: Vec<Option<u64>>,
    /// Daily count of `kind=law_violation` envelopes over the last
    /// 7 days, oldest-first.
    pub violations_7d_daily: Vec<u64>,
    /// Hourly classifier cost (USD) over the rolling 24 hours,
    /// oldest-first. All zero today — see
    /// `classifier_cost_unavailable_until_spec05` on the parent
    /// body for context.
    pub classifier_cost_usd_today: Vec<f64>,
}

pub(super) async fn overview(State(state): State<DashboardState>) -> Response {
    let snapshot = collect_lane_hits(&state.lane);

    let mut by_kind: std::collections::BTreeMap<&'static str, u64> =
        std::collections::BTreeMap::new();
    let mut by_repo: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut events_total: u64 = 0;
    for hit in &snapshot {
        events_total += 1;
        let kind = symbol_to_kind(hit.symbol.as_deref());
        *by_kind.entry(kind).or_insert(0) += 1;
        if let Some(repo) = hit.repo.as_deref() {
            *by_repo.entry(normalize_repo(repo)).or_insert(0) += 1;
        }
    }

    let mut repos_sorted: Vec<RepoCount> = by_repo
        .into_iter()
        .map(|(repo, count)| RepoCount { repo, count })
        .collect();
    repos_sorted.sort_by(|a, b| b.count.cmp(&a.count));
    repos_sorted.truncate(8);

    let now = chrono::Utc::now();
    let now_ms = now.timestamp_millis();
    // Pull the rolling-24h cost ribbon from the metadata store when
    // it's wired. `is_stub = true` means "store unwired or empty
    // window" — the GUI keeps showing "—" instead of $0.00 in that
    // case. Once the classifier-worker has booked at least one
    // classification in the last 24h, the flag flips to `false` and
    // the ribbon renders real numbers.
    let (classifier_cost_series, classifier_cost_stub) = match state.metadata.as_ref() {
        Some(store) => {
            let guard = match store.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            crate::dashboard_series::read_classifier_cost_24h(&guard, now)
        }
        None => (crate::dashboard_series::classifier_cost_zeros(), true),
    };
    let series = SeriesBlock {
        events_per_min: crate::dashboard_series::bucket_per_minute(&snapshot, now_ms, 20),
        pre_thinking_p95_ms: crate::dashboard_series::bucket_p95_duration_per_minute(
            &snapshot, now_ms, 20,
        ),
        violations_7d_daily: crate::dashboard_series::bucket_violations_per_day(
            &snapshot, now_ms, 7,
        ),
        classifier_cost_usd_today: classifier_cost_series,
    };

    let body = OverviewBody {
        events_total,
        repos_indexed: repos_sorted.len() as u64,
        kind_breakdown: by_kind
            .into_iter()
            .map(|(kind, count)| KindCount {
                kind: kind.to_string(),
                count,
            })
            .collect(),
        recent_repos: repos_sorted,
        series,
        classifier_cost_unavailable_until_spec05: classifier_cost_stub,
    };
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use serde_json::Value;
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

    fn turn_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = std::collections::BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String("session-default".to_string()),
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
            extras: std::collections::BTreeMap::new(),
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[tokio::test]
    async fn overview_breaks_down_by_kind_and_repo() {
        let state = make_state(vec![
            turn_hit("hi", "Cortex", 100),
            turn_hit("again", "Cortex", 200),
            tool_call_hit("Edit", "stuff", "Vectorizer", 150),
        ]);
        let resp = overview(State(state)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["events_total"], 3);
        assert_eq!(parsed["repos_indexed"], 2);
        let kinds = parsed["kind_breakdown"].as_array().unwrap();
        let turn_count = kinds
            .iter()
            .find(|k| k["kind"] == "turn")
            .and_then(|k| k["count"].as_u64());
        assert_eq!(turn_count, Some(2));
        let tool_count = kinds
            .iter()
            .find(|k| k["kind"] == "tool_call")
            .and_then(|k| k["count"].as_u64());
        assert_eq!(tool_count, Some(1));
    }

    #[tokio::test]
    async fn overview_carries_full_series_block_and_classifier_stub_flag() {
        let now = chrono::Utc::now().timestamp_millis();
        let state = make_state(vec![
            turn_hit("a", "Cortex", now - 30_000),
            tool_call_hit("Edit", "x", "Cortex", now - 60_000),
        ]);
        let resp = overview(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        let series = &parsed["series"];
        assert_eq!(series["events_per_min"].as_array().unwrap().len(), 20);
        assert_eq!(series["pre_thinking_p95_ms"].as_array().unwrap().len(), 20);
        assert_eq!(series["violations_7d_daily"].as_array().unwrap().len(), 7);
        assert_eq!(
            series["classifier_cost_usd_today"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        assert_eq!(parsed["classifier_cost_unavailable_until_spec05"], true);
        assert!(series["pre_thinking_p95_ms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.is_null()));
        assert!(series["classifier_cost_usd_today"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_f64() == Some(0.0)));
    }

    #[tokio::test]
    async fn overview_p95_picks_real_value_when_lane_carries_duration_ms() {
        let now = chrono::Utc::now().timestamp_millis();
        let mut hits: Vec<crate::lanes::LaneHit> = Vec::new();
        for d in [10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            let mut extras: std::collections::BTreeMap<String, serde_json::Value> =
                std::collections::BTreeMap::new();
            extras.insert(
                "duration_ms".to_string(),
                serde_json::Value::Number(serde_json::Number::from(d)),
            );
            hits.push(crate::lanes::LaneHit {
                doc_id: format!("archive|d{d}"),
                text: format!("dur={d}"),
                repo: Some("Cortex".to_string()),
                path: None,
                symbol: Some("tool_call:Edit".to_string()),
                content_hash: None,
                score: 1.0,
                ts: now - 30_000,
                severity: None,
                extras,
                overlay: crate::lanes::Overlay::default(),
            });
        }
        let state = make_state(hits);
        let resp = overview(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        let p95 = parsed["series"]["pre_thinking_p95_ms"].as_array().unwrap();
        let last = p95.last().unwrap();
        assert!(
            last.as_u64().is_some(),
            "expected P95 to be populated for the latest bucket, got {last:?}"
        );
        let v = last.as_u64().unwrap();
        assert!(
            (90..=100).contains(&v),
            "P95 of 10..100 step 10 should be in [90, 100], got {v}"
        );
    }
}
