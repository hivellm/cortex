//! Phase21 §8.2 — `/v1/dashboard/acl-stats` endpoint.
//!
//! Returns aggregated ACL decision metrics: total evaluated/granted/denied
//! counters, a 5-minute-bucket denial-rate time series, a per-repo
//! classification distribution, and the top-10 denied principals.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{collect_lane_hits, DashboardState};

/// One time-bucket in the denial-rate time-series.
#[derive(Debug, Clone, Serialize)]
pub struct DenialRateBucket {
    /// Unix epoch seconds for the start of this 5-minute bucket.
    pub ts: i64,
    /// Number of denied decisions in this bucket.
    pub denied: u64,
    /// Total evaluated decisions in this bucket.
    pub evaluated: u64,
}

/// Classification distribution for one `(repo, level)` pair.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationDistributionRow {
    /// Repository name (ASCII-lowercased).
    pub repo: String,
    /// Sensitivity level: `0` = public, `1` = internal,
    /// `2` = confidential, `3` = restricted.
    pub level: u8,
    /// Number of hits at this `(repo, level)`.
    pub count: u64,
}

/// Top-denied-principal row.
#[derive(Debug, Clone, Serialize)]
pub struct TopDeniedPrincipal {
    /// The principal identifier that was denied.
    pub principal_id: String,
    /// Cumulative denial count since service boot.
    pub denial_count: u64,
}

/// Response body for `GET /v1/dashboard/acl-stats`.
#[derive(Debug, Clone, Serialize)]
pub struct AclStatsBody {
    /// Total evaluated / granted / denied since service boot.
    pub total_evaluated: u64,
    /// Total granted since service boot.
    pub total_granted: u64,
    /// Total denied since service boot.
    pub total_denied: u64,
    /// Denial rate bucketed into 5-minute slots over the last 60 minutes.
    /// Empty when AC is disabled or no decisions have been recorded yet.
    pub denial_rate_over_time: Vec<DenialRateBucket>,
    /// Per-repo classification distribution derived from the keyword lane.
    pub classification_distribution: Vec<ClassificationDistributionRow>,
    /// Top-10 denied principals, descending by cumulative denial count.
    pub top_denied_principals: Vec<TopDeniedPrincipal>,
}

/// `GET /v1/dashboard/acl-stats` handler.
pub async fn acl_stats_handler(State(state): State<DashboardState>) -> Response {
    let (total_evaluated, total_granted, total_denied, denial_time, top_denied) =
        match &state.acl_metrics {
            Some(m) => {
                use std::sync::atomic::Ordering;
                let eval = m.evaluated_total.load(Ordering::Relaxed);
                let granted = m.granted_total.load(Ordering::Relaxed);
                let denied = m.denied_total.load(Ordering::Relaxed);
                let time = m.denial_rate_over_time(60);
                let top = m.top_denied_principals(10);
                (eval, granted, denied, time, top)
            }
            None => (0, 0, 0, vec![], vec![]),
        };

    // Compute classification distribution from the keyword lane.
    let hits = collect_lane_hits(&state.lane);
    let mut dist: std::collections::BTreeMap<(String, u8), u64> =
        std::collections::BTreeMap::new();
    for hit in &hits {
        let repo = hit.repo.as_deref().unwrap_or("unknown").to_string();
        let level = hit
            .extras
            .get("class_level")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8)
            .unwrap_or(0);
        *dist.entry((repo, level)).or_insert(0) += 1;
    }
    let classification_distribution = dist
        .into_iter()
        .map(|((repo, level), count)| ClassificationDistributionRow { repo, level, count })
        .collect();

    let body = AclStatsBody {
        total_evaluated,
        total_granted,
        total_denied,
        denial_rate_over_time: denial_time
            .into_iter()
            .map(|(ts, denied, evaluated)| DenialRateBucket { ts, denied, evaluated })
            .collect(),
        classification_distribution,
        top_denied_principals: top_denied
            .into_iter()
            .map(|(principal_id, denial_count)| TopDeniedPrincipal {
                principal_id,
                denial_count,
            })
            .collect(),
    };

    (StatusCode::OK, Json(body)).into_response()
}
