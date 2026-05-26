//! Phase14e — `/v1/health/pre-thinking` endpoint.
//!
//! Surfaces the pre-thinking circuit-breaker state +
//! `cortex_pre_thinking_fail_open_total{reason}` counters so the
//! operator can spot outages without scraping logs.
//!
//! ADR-014 pure-reader contract: handler returns the typed report
//! verbatim from a [`PreThinkingHealthSource`] implementation.
//! Default boot wiring uses [`UnwiredPreThinkingHealthSource`] —
//! state reads as `closed` + zero counters until main.rs threads a
//! live source over a shared `Arc<cortex_pre_thinking::Breaker>` +
//! `Arc<cortex_pre_thinking::Metrics>`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

/// Phase14f §4.3 — per-intent bundle-byte quantile row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-export",
    ts(export, export_to = "../../gui-types/")
)]
pub struct IntentByteQuantilesView {
    /// Sample count.
    #[serde(default)]
    pub count: u64,
    /// 50th percentile (median bundle bytes for this intent).
    #[serde(default)]
    pub p50: u32,
    /// 95th percentile bundle bytes.
    #[serde(default)]
    pub p95: u32,
    /// 99th percentile bundle bytes.
    #[serde(default)]
    pub p99: u32,
}

/// Phase14f §4.3 — per-intent helpful-rate row driven by feedback
/// POSTs. `rate = helpful / (helpful + unhelpful)`; `None` when
/// no feedback has been recorded for this intent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-export",
    ts(export, export_to = "../../gui-types/")
)]
pub struct IntentHelpfulRateView {
    /// helpful=true count.
    #[serde(default)]
    pub helpful: u64,
    /// helpful=false count.
    #[serde(default)]
    pub unhelpful: u64,
    /// `helpful / (helpful + unhelpful)`; serialised as `null`
    /// when both halves are zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

/// Top-level wire response for `/v1/health/pre-thinking`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-export",
    ts(export, export_to = "../../gui-types/")
)]
pub struct PreThinkingHealthReport {
    /// Current breaker state (`closed` / `open` / `halfopen`).
    pub breaker_state: String,
    /// Failures observed in the current breaker window.
    pub failures_in_window: u32,
    /// Per-reason fail-open counts since process boot.
    pub fail_open_total: BTreeMap<String, u64>,
    /// Sum across every reason — convenience field so the
    /// dashboard can render a single counter.
    pub fail_open_sum: u64,
    /// Phase14f §4.1 — per-intent bundle-bytes quantiles. Empty
    /// when no samples have been observed yet.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bundle_bytes_per_intent: BTreeMap<String, IntentByteQuantilesView>,
    /// Phase14f §4.2 — per-intent helpful-rate counters.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub helpful_rate_per_intent: BTreeMap<String, IntentHelpfulRateView>,
}

/// Source the handler reads from.
#[async_trait]
pub trait PreThinkingHealthSource: Send + Sync {
    /// Return the latest [`PreThinkingHealthReport`].
    async fn snapshot(&self) -> PreThinkingHealthReport;
}

/// Default source returning the all-empty report. Used at boot
/// until the live source is threaded in.
pub struct UnwiredPreThinkingHealthSource;

#[async_trait]
impl PreThinkingHealthSource for UnwiredPreThinkingHealthSource {
    async fn snapshot(&self) -> PreThinkingHealthReport {
        PreThinkingHealthReport {
            breaker_state: "closed".into(),
            failures_in_window: 0,
            fail_open_total: BTreeMap::new(),
            fail_open_sum: 0,
            bundle_bytes_per_intent: BTreeMap::new(),
            helpful_rate_per_intent: BTreeMap::new(),
        }
    }
}

/// Axum state for the pre-thinking health route.
#[derive(Clone)]
pub struct PreThinkingHealthState {
    /// Behind an `Arc<dyn _>` so both live and fixture sources
    /// dispatch through the same handler.
    pub source: Arc<dyn PreThinkingHealthSource>,
}

impl Default for PreThinkingHealthState {
    fn default() -> Self {
        Self {
            source: Arc::new(UnwiredPreThinkingHealthSource),
        }
    }
}

/// Build the `/v1/health/pre-thinking` sub-router.
pub fn build_router(state: PreThinkingHealthState) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/v1/health/pre-thinking", get(pre_thinking_health_handler))
        .with_state(state)
}

/// Axum handler. Pure projection of [`PreThinkingHealthSource::snapshot`].
pub async fn pre_thinking_health_handler(
    State(state): State<PreThinkingHealthState>,
) -> Response {
    let report = state.source.snapshot().await;
    Json(report).into_response()
}

// Phase14e — the live source backed by `cortex_pre_thinking::
// {Breaker, Metrics}` handles cannot live in this crate because
// cortex-pre-thinking already depends on cortex-api (circular).
// The live wiring ships in cortex-pre-thinking itself (or the
// adapter that owns both handles), as
// `cortex_pre_thinking::health_source::LivePreThinkingHealthSource`,
// which implements this trait directly.

#[cfg(test)]
mod tests {
    use super::*;

    struct StubSource(PreThinkingHealthReport);

    #[async_trait]
    impl PreThinkingHealthSource for StubSource {
        async fn snapshot(&self) -> PreThinkingHealthReport {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn unwired_source_returns_closed_zero_counters() {
        let src = UnwiredPreThinkingHealthSource;
        let r = src.snapshot().await;
        assert_eq!(r.breaker_state, "closed");
        assert_eq!(r.failures_in_window, 0);
        assert!(r.fail_open_total.is_empty());
        assert_eq!(r.fail_open_sum, 0);
    }

    #[tokio::test]
    async fn handler_returns_source_snapshot_verbatim() {
        let mut total = BTreeMap::new();
        total.insert("timeout".to_string(), 3u64);
        total.insert("breaker_open".to_string(), 2u64);
        let expected = PreThinkingHealthReport {
            breaker_state: "open".into(),
            failures_in_window: 5,
            fail_open_total: total,
            fail_open_sum: 5,
            bundle_bytes_per_intent: BTreeMap::new(),
            helpful_rate_per_intent: BTreeMap::new(),
        };
        let state = PreThinkingHealthState {
            source: Arc::new(StubSource(expected.clone())),
        };
        let resp = pre_thinking_health_handler(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: PreThinkingHealthReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed, expected);
    }

}
