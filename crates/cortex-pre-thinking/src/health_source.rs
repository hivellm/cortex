//! Phase14e — live `PreThinkingHealthSource` impl backed by the
//! shared `Arc<Breaker>` + `Arc<Metrics>` handles the pipeline +
//! adapter own. Lives here (not in `cortex-api`) to avoid the
//! circular dep: `cortex-pre-thinking` already pulls `cortex-api`.
//!
//! Production wiring: main.rs builds the breaker + metrics once,
//! hands both clones to the adapter's pipeline runner AND wraps
//! them in [`LivePreThinkingHealthSource`] for the
//! `/v1/health/pre-thinking` endpoint.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::health::pre_thinking::{
    IntentByteQuantilesView, IntentHelpfulRateView, IntentMismatchView, PreThinkingHealthReport,
    PreThinkingHealthSource,
};

use crate::breaker::{Breaker, BreakerState};
use crate::Metrics;

/// Live source projecting the shared breaker + metrics into the
/// cortex-api health-endpoint wire shape.
pub struct LivePreThinkingHealthSource {
    /// Shared breaker handle — the same instance the pipeline
    /// guards calls through.
    pub breaker: Arc<Breaker>,
    /// Shared metrics registry — the same instance the pipeline
    /// bumps `fail_open_total{reason}` on.
    pub metrics: Arc<Metrics>,
}

impl LivePreThinkingHealthSource {
    /// Construct from shared handles.
    pub fn new(breaker: Arc<Breaker>, metrics: Arc<Metrics>) -> Self {
        Self { breaker, metrics }
    }
}

#[async_trait]
impl PreThinkingHealthSource for LivePreThinkingHealthSource {
    async fn snapshot(&self) -> PreThinkingHealthReport {
        let state = match self.breaker.state() {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "halfopen",
        };
        let fail_open_total = self.metrics.fail_open_snapshot();
        let fail_open_sum: u64 = fail_open_total.values().copied().sum();
        let bundle_bytes_per_intent = self
            .metrics
            .bundle_bytes_quantiles_per_intent()
            .into_iter()
            .map(|(k, q)| {
                (
                    k,
                    IntentByteQuantilesView {
                        count: q.count,
                        p50: q.p50,
                        p95: q.p95,
                        p99: q.p99,
                    },
                )
            })
            .collect();
        let helpful_rate_per_intent = self
            .metrics
            .helpful_rate_per_intent()
            .into_iter()
            .map(|(k, r)| {
                (
                    k,
                    IntentHelpfulRateView {
                        helpful: r.helpful,
                        unhelpful: r.unhelpful,
                        rate: r.rate,
                    },
                )
            })
            .collect();
        let intent_mismatch_top = self
            .metrics
            .intent_mismatch_snapshot()
            .into_iter()
            .map(|(from, to, count)| IntentMismatchView { from, to, count })
            .collect();
        let rewriter_path_total = self.metrics.rewriter_path_snapshot();
        let (cache_hit_total, cache_miss_total) = self.metrics.cache_counters();
        PreThinkingHealthReport {
            breaker_state: state.to_string(),
            failures_in_window: self.breaker.failures_in_window(),
            fail_open_total,
            fail_open_sum,
            bundle_bytes_per_intent,
            helpful_rate_per_intent,
            intent_mismatch_top,
            rewriter_path_total,
            cache_hit_total,
            cache_miss_total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breaker::{BreakerConfig, FailReason};
    use std::time::Duration;

    #[tokio::test]
    async fn live_source_reflects_breaker_and_metrics() {
        let breaker = Arc::new(Breaker::with_config(BreakerConfig {
            threshold: 2,
            window: Duration::from_secs(60),
            cooldown: Duration::from_secs(30),
        }));
        let metrics = Arc::new(Metrics::new());
        for _ in 0..2 {
            breaker.guard().unwrap().record_failure();
        }
        metrics.incr_fail_open(FailReason::Timeout.as_str());
        metrics.incr_fail_open(FailReason::Timeout.as_str());
        metrics.incr_fail_open(FailReason::BreakerOpen.as_str());
        let src = LivePreThinkingHealthSource::new(breaker.clone(), metrics.clone());
        let r = src.snapshot().await;
        assert_eq!(r.breaker_state, "open");
        assert_eq!(r.fail_open_sum, 3);
        assert_eq!(r.fail_open_total.get("timeout").copied(), Some(2));
        assert_eq!(r.fail_open_total.get("breaker_open").copied(), Some(1));
        assert_eq!(r.failures_in_window, 2);
    }

    #[tokio::test]
    async fn closed_breaker_yields_closed_state() {
        let breaker = Arc::new(Breaker::new());
        let metrics = Arc::new(Metrics::new());
        let src = LivePreThinkingHealthSource::new(breaker, metrics);
        let r = src.snapshot().await;
        assert_eq!(r.breaker_state, "closed");
        assert_eq!(r.fail_open_sum, 0);
    }
}
