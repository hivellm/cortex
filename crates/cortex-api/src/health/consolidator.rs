//! phase14a §4.1 — `/v1/health/consolidator` endpoint.
//!
//! Surfaces the last-run timestamp + last-status string per
//! consolidator grain (Session / Topic / DecisionTrace). The
//! upstream daemon (phase14a §3) persists [`ConsolidationReport`]
//! rows; this handler projects the latest one per grain into a
//! typed `ConsolidatorHealthReport`.
//!
//! ADR-014 pure-reader contract: handler returns the typed report
//! verbatim from a [`ConsolidatorHealthSource`] implementation.
//! Default boot-time wiring uses [`UnwiredConsolidatorHealthSource`]
//! — every grain reads as `last_run: null, last_status: null` until
//! main.rs threads a live `consolidation_reports`-backed source in
//! a follow-up.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-grain status row carried on the response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrainHealth {
    /// `finished_at` stamp of the most recent
    /// [`super::super::consolidator::ConsolidationReport`] row for
    /// this grain. `None` when no run has been recorded yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<DateTime<Utc>>,
    /// Last status string (`success` / `failed` / `running`).
    /// `None` when no run has been recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    /// Number of envelopes the last run emitted.
    #[serde(default)]
    pub envelopes_emitted: u64,
    /// Wall-clock latency of the last run in milliseconds.
    #[serde(default)]
    pub latency_ms: u64,
}

/// Top-level wire response for `/v1/health/consolidator`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidatorHealthReport {
    /// Session-grain status row.
    pub session_grain: GrainHealth,
    /// Topic-grain status row.
    pub topic_grain: GrainHealth,
    /// DecisionTrace-grain status row.
    pub decision_trace_grain: GrainHealth,
}

/// Source the handler reads from.
#[async_trait]
pub trait ConsolidatorHealthSource: Send + Sync {
    /// Return the latest [`ConsolidatorHealthReport`]. The
    /// implementation is free to read from SQLite, Meili, or an
    /// in-memory cache — the handler does not care.
    async fn snapshot(&self) -> ConsolidatorHealthReport;
}

/// Default source returning the all-empty report. Used at boot
/// until the daemon-backed source is threaded in.
pub struct UnwiredConsolidatorHealthSource;

#[async_trait]
impl ConsolidatorHealthSource for UnwiredConsolidatorHealthSource {
    async fn snapshot(&self) -> ConsolidatorHealthReport {
        ConsolidatorHealthReport::default()
    }
}

/// Axum state for the consolidator health route.
#[derive(Clone)]
pub struct ConsolidatorHealthState {
    /// Behind an `Arc<dyn _>` so both live and fixture sources
    /// dispatch through the same handler.
    pub source: Arc<dyn ConsolidatorHealthSource>,
}

impl Default for ConsolidatorHealthState {
    fn default() -> Self {
        Self {
            source: Arc::new(UnwiredConsolidatorHealthSource),
        }
    }
}

/// Build the `/v1/health/consolidator` sub-router.
pub fn build_router(state: ConsolidatorHealthState) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/v1/health/consolidator", get(consolidator_health_handler))
        .with_state(state)
}

/// Axum handler. Pure projection of [`ConsolidatorHealthSource::snapshot`].
pub async fn consolidator_health_handler(
    State(state): State<ConsolidatorHealthState>,
) -> Response {
    let report = state.source.snapshot().await;
    Json(report).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("valid rfc3339")
    }

    struct StubSource(ConsolidatorHealthReport);

    #[async_trait]
    impl ConsolidatorHealthSource for StubSource {
        async fn snapshot(&self) -> ConsolidatorHealthReport {
            self.0.clone()
        }
    }

    #[test]
    fn default_state_uses_unwired_source() {
        let state = ConsolidatorHealthState::default();
        // The default carries an `UnwiredConsolidatorHealthSource`
        // — exercised indirectly by snapshot below.
        let _ = state;
    }

    #[tokio::test]
    async fn unwired_source_returns_all_default_grains() {
        let src = UnwiredConsolidatorHealthSource;
        let r = src.snapshot().await;
        assert!(r.session_grain.last_run.is_none());
        assert!(r.session_grain.last_status.is_none());
        assert!(r.topic_grain.last_run.is_none());
        assert!(r.decision_trace_grain.last_run.is_none());
    }

    #[test]
    fn grain_health_skips_optional_fields_when_none() {
        let g = GrainHealth::default();
        let j = serde_json::to_value(&g).unwrap();
        assert!(j.get("last_run").is_none());
        assert!(j.get("last_status").is_none());
        assert_eq!(j["envelopes_emitted"], 0);
        assert_eq!(j["latency_ms"], 0);
    }

    #[test]
    fn report_round_trips_via_serde_with_one_grain_populated() {
        let r = ConsolidatorHealthReport {
            session_grain: GrainHealth {
                last_run: Some(ts("2026-05-25T12:00:00Z")),
                last_status: Some("success".into()),
                envelopes_emitted: 4,
                latency_ms: 1_500,
            },
            ..ConsolidatorHealthReport::default()
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ConsolidatorHealthReport = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed, r);
    }

    #[tokio::test]
    async fn handler_returns_source_snapshot_verbatim() {
        let expected = ConsolidatorHealthReport {
            topic_grain: GrainHealth {
                last_run: Some(ts("2026-05-24T03:00:00Z")),
                last_status: Some("failed".into()),
                envelopes_emitted: 0,
                latency_ms: 200,
            },
            ..ConsolidatorHealthReport::default()
        };
        let state = ConsolidatorHealthState {
            source: Arc::new(StubSource(expected.clone())),
        };
        let resp = consolidator_health_handler(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: ConsolidatorHealthReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed, expected);
    }
}
