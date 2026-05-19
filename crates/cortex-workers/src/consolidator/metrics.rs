//! Consolidator metrics — phase12a §3.
//!
//! In-process atomic counters that back the
//! `cortex_consolidator_publish_failures_total{reason}` and
//! `cortex_consolidator_publish_ok_total` metric families documented in
//! `docs/specs/12-pre-thinking-injection.md` § Publishing observability.
//!
//! The exporter wiring (Prometheus / OpenTelemetry) lives outside this
//! crate; consumers read these counters via [`Metrics::snapshot`].
//!
//! ## Why these counters
//!
//! Phase12a §1-§2 closed the silent envelope-drop hole that previously
//! lived at every `publish_consolidation()` failure path. Logs and the
//! JSONL fallback give per-event visibility, but operators also need a
//! cardinality-bounded counter so dashboards can alarm on "fallback
//! file is filling" without scanning logs. One counter per `reason`
//! label satisfies that without exploding cardinality (4 fixed
//! reasons + 1 success label = 5 series total).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

/// Stable label set the consolidator publish path emits. Keeping the
/// values fixed here prevents accidental cardinality growth — adding
/// a new reason is a deliberate code change with a paired test.
pub const REASON_ENV_UNSET: &str = "env_unset";
/// `reqwest::Client::builder().build()` failure (TLS init, etc.).
pub const REASON_CLIENT_BUILD: &str = "client_build";
/// Server returned a non-2xx HTTP status.
pub const REASON_NON_2XX: &str = "non_2xx";
/// Transport-level failure (connect refused, timeout, DNS, etc.).
pub const REASON_NETWORK: &str = "network";

/// Process-wide singleton. The bin reads it via [`metrics()`].
static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::default);

/// Borrow the process-wide consolidator metrics registry.
pub fn metrics() -> &'static Metrics {
    &METRICS
}

/// Consolidator metrics registry. Per-reason failure counters plus a
/// matching success counter so dashboards can compute a failure ratio
/// without touching the JSONL fallback.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex_consolidator_publish_failures_total{reason="env_unset"}`.
    publish_failures_env_unset: AtomicU64,
    /// `cortex_consolidator_publish_failures_total{reason="client_build"}`.
    publish_failures_client_build: AtomicU64,
    /// `cortex_consolidator_publish_failures_total{reason="non_2xx"}`.
    publish_failures_non_2xx: AtomicU64,
    /// `cortex_consolidator_publish_failures_total{reason="network"}`.
    publish_failures_network: AtomicU64,
    /// `cortex_consolidator_publish_ok_total`.
    publish_ok: AtomicU64,
}

impl Metrics {
    /// Increment the failure counter matching `reason`. Unknown labels
    /// are silently ignored — the publish path passes string literals
    /// from this module's constants, so an unknown label is a code
    /// bug the test suite catches via [`tests::known_reasons_route`].
    pub fn record_publish_failure(&self, reason: &str) {
        let counter = match reason {
            REASON_ENV_UNSET => &self.publish_failures_env_unset,
            REASON_CLIENT_BUILD => &self.publish_failures_client_build,
            REASON_NON_2XX => &self.publish_failures_non_2xx,
            REASON_NETWORK => &self.publish_failures_network,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the success counter.
    pub fn record_publish_ok(&self) {
        self.publish_ok.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the current failure count for `reason`. Returns `0` for
    /// unknown labels.
    pub fn publish_failures(&self, reason: &str) -> u64 {
        let counter = match reason {
            REASON_ENV_UNSET => &self.publish_failures_env_unset,
            REASON_CLIENT_BUILD => &self.publish_failures_client_build,
            REASON_NON_2XX => &self.publish_failures_non_2xx,
            REASON_NETWORK => &self.publish_failures_network,
            _ => return 0,
        };
        counter.load(Ordering::Relaxed)
    }

    /// Read the current success count.
    pub fn publish_ok_total(&self) -> u64 {
        self.publish_ok.load(Ordering::Relaxed)
    }

    /// Atomic snapshot of all five counters. Suitable for the
    /// Prometheus exporter — emit one line per non-zero entry.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            env_unset: self.publish_failures(REASON_ENV_UNSET),
            client_build: self.publish_failures(REASON_CLIENT_BUILD),
            non_2xx: self.publish_failures(REASON_NON_2XX),
            network: self.publish_failures(REASON_NETWORK),
            ok: self.publish_ok_total(),
        }
    }
}

/// Read-side snapshot of [`Metrics`]. Cheap to clone; no atomics in the
/// snapshot type itself so consumers can pass it across threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetricsSnapshot {
    /// `reason="env_unset"` count.
    pub env_unset: u64,
    /// `reason="client_build"` count.
    pub client_build: u64,
    /// `reason="non_2xx"` count.
    pub non_2xx: u64,
    /// `reason="network"` count.
    pub network: u64,
    /// `cortex_consolidator_publish_ok_total`.
    pub ok: u64,
}

impl MetricsSnapshot {
    /// Sum across all four failure reasons.
    pub fn failures_total(&self) -> u64 {
        self.env_unset + self.client_build + self.non_2xx + self.network
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_reasons_route_to_distinct_counters() {
        let m = Metrics::default();
        m.record_publish_failure(REASON_ENV_UNSET);
        m.record_publish_failure(REASON_CLIENT_BUILD);
        m.record_publish_failure(REASON_CLIENT_BUILD);
        m.record_publish_failure(REASON_NON_2XX);
        m.record_publish_failure(REASON_NETWORK);
        assert_eq!(m.publish_failures(REASON_ENV_UNSET), 1);
        assert_eq!(m.publish_failures(REASON_CLIENT_BUILD), 2);
        assert_eq!(m.publish_failures(REASON_NON_2XX), 1);
        assert_eq!(m.publish_failures(REASON_NETWORK), 1);
        assert_eq!(m.publish_ok_total(), 0);
    }

    #[test]
    fn unknown_reason_is_dropped_silently() {
        let m = Metrics::default();
        m.record_publish_failure("not_a_real_reason");
        let snap = m.snapshot();
        assert_eq!(snap.failures_total(), 0);
        assert_eq!(snap.ok, 0);
    }

    #[test]
    fn record_publish_ok_advances_success_counter_only() {
        let m = Metrics::default();
        m.record_publish_ok();
        m.record_publish_ok();
        assert_eq!(m.publish_ok_total(), 2);
        assert_eq!(m.snapshot().failures_total(), 0);
    }

    #[test]
    fn snapshot_returns_atomic_view_of_all_counters() {
        let m = Metrics::default();
        m.record_publish_failure(REASON_ENV_UNSET);
        m.record_publish_failure(REASON_NETWORK);
        m.record_publish_failure(REASON_NETWORK);
        m.record_publish_ok();
        m.record_publish_ok();
        m.record_publish_ok();
        let snap = m.snapshot();
        assert_eq!(snap.env_unset, 1);
        assert_eq!(snap.network, 2);
        assert_eq!(snap.non_2xx, 0);
        assert_eq!(snap.client_build, 0);
        assert_eq!(snap.ok, 3);
        assert_eq!(snap.failures_total(), 3);
    }

    #[test]
    fn process_wide_singleton_persists_across_calls() {
        // The static accessor is shared across the whole process, so
        // we cannot reset between tests. Read the baseline first, bump
        // each reason once, and assert the delta to keep the test
        // resilient to order-dependent test execution.
        let baseline = metrics().snapshot();
        metrics().record_publish_failure(REASON_ENV_UNSET);
        metrics().record_publish_failure(REASON_CLIENT_BUILD);
        metrics().record_publish_failure(REASON_NON_2XX);
        metrics().record_publish_failure(REASON_NETWORK);
        metrics().record_publish_ok();
        let after = metrics().snapshot();
        assert_eq!(after.env_unset - baseline.env_unset, 1);
        assert_eq!(after.client_build - baseline.client_build, 1);
        assert_eq!(after.non_2xx - baseline.non_2xx, 1);
        assert_eq!(after.network - baseline.network, 1);
        assert_eq!(after.ok - baseline.ok, 1);
    }
}
