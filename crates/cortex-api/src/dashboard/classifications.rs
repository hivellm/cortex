use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::{
    clip, collect_lane_hits, normalize_repo, symbol_to_kind, ts_to_relative, DashboardState,
    KindCount, RepoCount,
};

/// One classified-event row surfaced by the Classifications view.
/// Mirrors what the cortex-fulltext-worker stamped on the Meili doc
/// (which the meili_loader projects onto every LaneHit's extras),
/// shaped for the GUI table.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationRow {
    /// `event_id` (best-effort — falls back to the `doc_id` chunks).
    pub event_id: String,
    /// `turn` / `tool_call` / `decision` / `memory` / etc.
    pub kind: String,
    /// Repo the event was captured from.
    pub repo: Option<String>,
    /// Repo-relative path when available (artifact / handoff / spec).
    pub path: Option<String>,
    /// Topics the classifier stamped (controlled-vocab tags).
    pub topics: Vec<String>,
    /// `info` / `notable` / `critical`.
    pub severity: Option<String>,
    /// `none` / `low` / `high` (or whatever the classifier surfaced).
    pub pii_risk: Option<String>,
    /// Short summary clipped at 240 chars — same content the
    /// Sonnet classifier produces, surfaced inline so the operator
    /// can see whether the summaries are useful at scale.
    pub summary: String,
    /// Wall-clock ms epoch.
    pub ts: i64,
    /// Relative time label.
    pub at: String,
}

/// Aggregate counts the GUI renders as histograms / topic clouds
/// alongside the recent rows.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationStats {
    /// Total classified events surfaced (post-filter).
    pub total: u64,
    /// Top topics across the surfaced rows, descending by count.
    pub top_topics: Vec<TopicCount>,
    /// Per-severity counts.
    pub by_severity: Vec<KindCount>,
    /// Per-pii-risk counts.
    pub by_pii_risk: Vec<KindCount>,
    /// Per-repo counts.
    pub by_repo: Vec<RepoCount>,
}

/// One row of the topic cloud.
#[derive(Debug, Clone, Serialize)]
pub struct TopicCount {
    /// The topic tag.
    pub topic: String,
    /// How many surfaced rows carried it.
    pub count: u64,
}

/// Top-level body for `/v1/dashboard/classifications`. Splits the
/// stats from the rows so the GUI can lay them out in distinct
/// regions without re-aggregating client-side.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationsBody {
    /// Aggregate counts over the surfaced rows.
    pub stats: ClassificationStats,
    /// Recent rows, newest-first, capped by `limit`.
    pub rows: Vec<ClassificationRow>,
}

/// Query params for `/v1/dashboard/classifications`. All optional;
/// empty filters surface every classified event in the lane.
#[derive(Debug, Default, Deserialize)]
pub struct ClassificationsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Single-topic filter — `?topic=performance`.
    #[serde(default)]
    pub topic: Option<String>,
    /// Single-severity filter — `?severity=critical`.
    #[serde(default)]
    pub severity: Option<String>,
    /// Single-kind filter — `?kind=turn`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Cap on rows returned. Stats always cover the full filtered
    /// set regardless of this limit.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub(super) async fn classifications(
    State(state): State<DashboardState>,
    Query(params): Query<ClassificationsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Apply optional filters first so stats reflect what the user
    // is looking at, not the whole corpus.
    let filtered: Vec<&crate::lanes::LaneHit> = hits
        .iter()
        .filter(|h| {
            if let Some(r) = params.repo.as_deref().filter(|s| !s.is_empty()) {
                if h.repo.as_deref().map(normalize_repo) != Some(normalize_repo(r)) {
                    return false;
                }
            }
            if let Some(k) = params.kind.as_deref().filter(|s| !s.is_empty()) {
                if symbol_to_kind(h.symbol.as_deref()) != k {
                    return false;
                }
            }
            if let Some(sev) = params.severity.as_deref().filter(|s| !s.is_empty()) {
                if h.severity.as_deref() != Some(sev) {
                    return false;
                }
            }
            if let Some(t) = params.topic.as_deref().filter(|s| !s.is_empty()) {
                let topics = h
                    .extras
                    .get("topics")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().any(|x| x.as_str() == Some(t)))
                    .unwrap_or(false);
                if !topics {
                    return false;
                }
            }
            true
        })
        .collect();

    // Aggregates over the filtered set.
    let mut topic_counts: std::collections::HashMap<String, u64> = Default::default();
    let mut sev_counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut pii_counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut repo_counts: std::collections::BTreeMap<String, u64> = Default::default();
    for h in &filtered {
        if let Some(arr) = h.extras.get("topics").and_then(|v| v.as_array()) {
            for t in arr.iter().filter_map(|v| v.as_str()) {
                *topic_counts.entry(t.to_string()).or_insert(0) += 1;
            }
        }
        if let Some(s) = h.severity.as_deref() {
            *sev_counts.entry(s.to_string()).or_insert(0) += 1;
        }
        if let Some(p) = h.extras.get("pii_risk").and_then(|v| v.as_str()) {
            *pii_counts.entry(p.to_string()).or_insert(0) += 1;
        }
        if let Some(r) = h.repo.as_deref() {
            *repo_counts.entry(normalize_repo(r)).or_insert(0) += 1;
        }
    }

    let mut top_topics: Vec<TopicCount> = topic_counts
        .into_iter()
        .map(|(topic, count)| TopicCount { topic, count })
        .collect();
    top_topics.sort_by_key(|t| std::cmp::Reverse(t.count));
    top_topics.truncate(40);

    let by_severity: Vec<KindCount> = sev_counts
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let by_pii_risk: Vec<KindCount> = pii_counts
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let mut by_repo: Vec<RepoCount> = repo_counts
        .into_iter()
        .map(|(repo, count)| RepoCount { repo, count })
        .collect();
    by_repo.sort_by_key(|r| std::cmp::Reverse(r.count));

    let total = filtered.len() as u64;

    // Recent rows, newest-first, capped at `limit` (default 100, max 500).
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let mut sorted: Vec<&crate::lanes::LaneHit> = filtered.into_iter().collect();
    sorted.sort_by_key(|h| std::cmp::Reverse(h.ts));
    let rows: Vec<ClassificationRow> = sorted
        .into_iter()
        .take(limit)
        .map(|h| {
            let topics = h
                .extras
                .get("topics")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let summary = h
                .extras
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| clip(s, 240))
                .unwrap_or_else(|| clip(&h.text, 240));
            let pii_risk = h
                .extras
                .get("pii_risk")
                .and_then(|v| v.as_str())
                .map(String::from);
            let event_id = h
                .doc_id
                .rsplit_once('|')
                .map(|(_, id)| id.to_string())
                .unwrap_or_else(|| h.doc_id.clone());
            ClassificationRow {
                event_id,
                kind: symbol_to_kind(h.symbol.as_deref()).to_string(),
                repo: h.repo.clone(),
                path: h.path.clone(),
                topics,
                severity: h.severity.clone(),
                pii_risk,
                summary,
                ts: h.ts,
                at: ts_to_relative(h.ts),
            }
        })
        .collect();

    let body = ClassificationsBody {
        stats: ClassificationStats {
            total,
            top_topics,
            by_severity,
            by_pii_risk,
            by_repo,
        },
        rows,
    };
    (StatusCode::OK, Json(body)).into_response()
}
