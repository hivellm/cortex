//! Metrics counters surfaced by the MCP server. Spec 18
//! §Observability.
//!
//! Implemented as atomics; the exporter wiring (Prometheus / OTel)
//! lands with spec 13 on the daemon side. For now the counters are
//! readable so tests can assert on them.

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters keyed by tool name. The label set is small + fixed
/// so a few hand-rolled atomics beat the dependency on a metrics
/// crate at this layer.
#[derive(Debug, Default)]
pub struct Metrics {
    handshakes: AtomicU64,
    invocations_query: AtomicU64,
    invocations_pre_thinking: AtomicU64,
    invocations_status: AtomicU64,
    errors_query: AtomicU64,
    errors_pre_thinking: AtomicU64,
    errors_status: AtomicU64,
    latency_sum_ms_query: AtomicU64,
    latency_sum_ms_pre_thinking: AtomicU64,
    latency_sum_ms_status: AtomicU64,
}

impl Metrics {
    /// Build an empty metric set.
    pub fn new() -> Self {
        Self::default()
    }

    /// `cortex.plugin.session.handshakes`.
    pub fn incr_handshake(&self) {
        self.handshakes.fetch_add(1, Ordering::Relaxed);
    }

    /// `cortex.plugin.tool.invocations{tool=…}`.
    pub fn incr_invocation(&self, tool: &str) {
        match tool {
            "cortex_query" => &self.invocations_query,
            "cortex_pre_thinking" => &self.invocations_pre_thinking,
            "cortex_status" => &self.invocations_status,
            _ => return,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// `cortex.plugin.tool.errors{tool=…}`.
    pub fn incr_error(&self, tool: &str) {
        match tool {
            "cortex_query" => &self.errors_query,
            "cortex_pre_thinking" => &self.errors_pre_thinking,
            "cortex_status" => &self.errors_status,
            _ => return,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    /// `cortex.plugin.tool.latency_ms{tool=…}` — naive sum so callers
    /// can derive the average. Histograms wait for spec 13.
    pub fn observe_latency(&self, tool: &str, ms: u64) {
        let target = match tool {
            "cortex_query" => &self.latency_sum_ms_query,
            "cortex_pre_thinking" => &self.latency_sum_ms_pre_thinking,
            "cortex_status" => &self.latency_sum_ms_status,
            _ => return,
        };
        target.fetch_add(ms, Ordering::Relaxed);
    }

    /// Read-only view for tests + CLI dumps.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            handshakes: self.handshakes.load(Ordering::Relaxed),
            invocations_query: self.invocations_query.load(Ordering::Relaxed),
            invocations_pre_thinking: self.invocations_pre_thinking.load(Ordering::Relaxed),
            invocations_status: self.invocations_status.load(Ordering::Relaxed),
            errors_query: self.errors_query.load(Ordering::Relaxed),
            errors_pre_thinking: self.errors_pre_thinking.load(Ordering::Relaxed),
            errors_status: self.errors_status.load(Ordering::Relaxed),
            latency_sum_ms_query: self.latency_sum_ms_query.load(Ordering::Relaxed),
            latency_sum_ms_pre_thinking: self.latency_sum_ms_pre_thinking.load(Ordering::Relaxed),
            latency_sum_ms_status: self.latency_sum_ms_status.load(Ordering::Relaxed),
        }
    }
}

/// Plain-old-data snapshot for inspection.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct MetricsSnapshot {
    pub handshakes: u64,
    pub invocations_query: u64,
    pub invocations_pre_thinking: u64,
    pub invocations_status: u64,
    pub errors_query: u64,
    pub errors_pre_thinking: u64,
    pub errors_status: u64,
    pub latency_sum_ms_query: u64,
    pub latency_sum_ms_pre_thinking: u64,
    pub latency_sum_ms_status: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment_per_tool() {
        let m = Metrics::new();
        m.incr_handshake();
        m.incr_invocation("cortex_query");
        m.incr_invocation("cortex_query");
        m.incr_invocation("cortex_status");
        m.incr_error("cortex_pre_thinking");
        m.observe_latency("cortex_query", 12);
        let s = m.snapshot();
        assert_eq!(s.handshakes, 1);
        assert_eq!(s.invocations_query, 2);
        assert_eq!(s.invocations_status, 1);
        assert_eq!(s.errors_pre_thinking, 1);
        assert_eq!(s.latency_sum_ms_query, 12);
    }

    #[test]
    fn unknown_tool_label_is_ignored() {
        let m = Metrics::new();
        m.incr_invocation("cortex_unknown");
        m.incr_error("cortex_unknown");
        m.observe_latency("cortex_unknown", 5);
        let s = m.snapshot();
        assert_eq!(s.invocations_query, 0);
        assert_eq!(s.errors_query, 0);
    }
}
