//! `cortex.prethink.*` counters / histograms (spec 12 §Observability).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::budget::TrimStep;

/// Pre-thinking metrics registry.
#[derive(Debug, Default)]
pub struct Metrics {
    /// `cortex.prethink.calls.total{intent}`.
    pub calls_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.bundle.bytes` histogram.
    pub bundle_bytes: Mutex<Vec<u32>>,
    /// `cortex.prethink.sections.count{section}` histogram.
    pub section_count: Mutex<BTreeMap<String, Vec<u32>>>,
    /// `cortex.prethink.truncation.applied{step}`.
    pub truncation_applied: Mutex<BTreeMap<String, u64>>,
    /// `cortex.prethink.latency_ms` histogram.
    pub latency_ms: Mutex<Vec<u64>>,
    /// `cortex.prethink.empty_bundle`.
    pub empty_bundle: AtomicU64,
    /// `cortex.prethink.timeouts`.
    pub timeouts: AtomicU64,
    /// Phase14e — `cortex_pre_thinking_fail_open_total{reason}`.
    /// Reasons: `timeout`, `network`, `unauthorised`, `internal`,
    /// `breaker_open`. Read by the doctor + the
    /// `/v1/health/pre-thinking` endpoint to surface outage
    /// counts to the operator.
    pub fail_open_total: Mutex<BTreeMap<String, u64>>,
    /// Phase14f — `cortex_pre_thinking_bundle_bytes_per_intent`
    /// histogram. Per-intent bundle-size sample so the dashboard
    /// can render p50/p95/p99 per intent.
    pub bundle_bytes_per_intent: Mutex<BTreeMap<String, Vec<u32>>>,
    /// Phase14f — `cortex_pre_thinking_helpful_total{intent,helpful}`.
    /// Driven by feedback POSTs; the dashboard derives
    /// `helpful_rate = helpful / (helpful + unhelpful)` per intent.
    pub helpful_total: Mutex<BTreeMap<(String, bool), u64>>,
    /// Phase14g — `cortex_pre_thinking_intent_mismatch_total{from,to}`.
    /// Bumped when a feedback row marks `helpful = false` and the
    /// model corrected to a different intent in the same turn.
    /// Drives the `cortex-ops intent-stats` mismatch-rate report.
    pub intent_mismatch_total: Mutex<BTreeMap<(String, String), u64>>,
    /// Phase14g — `cortex_pre_thinking_rewriter_path_total{path}`.
    /// Paths: `sonnet_hit`, `sonnet_cache_hit`, `sonnet_timeout`,
    /// `sonnet_error`, `deterministic_fallback`. Drives the
    /// cascade telemetry the rewriter doctor surfaces.
    pub rewriter_path_total: Mutex<BTreeMap<String, u64>>,
    /// phase26c §2.3 — bundle-cache hit counter. Bumped by the
    /// adapter's `BundleCache` on every cache hit so the operator
    /// can read the hit rate on `/v1/health/pre-thinking`.
    pub cache_hit_total: AtomicU64,
    /// phase26c §2.3 — bundle-cache miss counter. Bumped by the
    /// adapter's `BundleCache` on every cache miss (pipeline ran).
    pub cache_miss_total: AtomicU64,
}

impl Metrics {
    /// Fresh registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Increment the call counter for `intent`.
    pub fn incr_calls(&self, intent: &str) {
        if let Ok(mut m) = self.calls_total.lock() {
            *m.entry(intent.to_string()).or_insert(0) += 1;
        }
    }
    /// Record a bundle size observation.
    pub fn observe_bundle_bytes(&self, n: u32) {
        if let Ok(mut g) = self.bundle_bytes.lock() {
            g.push(n);
        }
    }
    /// Record a section-count observation.
    pub fn observe_section_count(&self, section: &str, n: u32) {
        if let Ok(mut m) = self.section_count.lock() {
            m.entry(section.to_string()).or_default().push(n);
        }
    }
    /// Record a truncation step.
    pub fn incr_truncation_step(&self, step: TrimStep) {
        if let Ok(mut m) = self.truncation_applied.lock() {
            *m.entry(format!("{step:?}")).or_insert(0) += 1;
        }
    }
    /// Record a latency observation.
    pub fn observe_latency_ms(&self, ms: u64) {
        if let Ok(mut g) = self.latency_ms.lock() {
            g.push(ms);
        }
    }
    /// Increment the empty-bundle counter.
    pub fn incr_empty_bundle(&self) {
        self.empty_bundle.fetch_add(1, Ordering::Relaxed);
    }
    /// Increment the timeout counter.
    pub fn incr_timeouts(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Phase14e — bump the per-reason fail-open counter. Called
    /// from the pipeline on every fail-open dispatch (real
    /// upstream failure OR breaker-open short-circuit).
    pub fn incr_fail_open(&self, reason: &str) {
        if let Ok(mut m) = self.fail_open_total.lock() {
            *m.entry(reason.to_string()).or_insert(0) += 1;
        }
    }

    /// Phase14e — snapshot the per-reason fail-open counters.
    /// Used by the doctor + the `/v1/health/pre-thinking`
    /// endpoint.
    pub fn fail_open_snapshot(&self) -> BTreeMap<String, u64> {
        self.fail_open_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Phase14f — record a per-intent bundle-byte sample.
    /// Histogram values are buffered in-memory; the
    /// dashboard projection (`bundle_bytes_quantiles_per_intent`)
    /// summarises them to p50/p95/p99.
    pub fn observe_bundle_bytes_per_intent(&self, intent: &str, n: u32) {
        if let Ok(mut m) = self.bundle_bytes_per_intent.lock() {
            m.entry(intent.to_string()).or_default().push(n);
        }
    }

    /// Phase14f — bump the helpful counter for an
    /// `(intent, helpful)` pair.
    pub fn incr_helpful(&self, intent: &str, helpful: bool) {
        if let Ok(mut m) = self.helpful_total.lock() {
            *m.entry((intent.to_string(), helpful)).or_insert(0) += 1;
        }
    }

    /// Phase14g — bump the per-(from,to) mismatch counter. Called
    /// from the feedback recorder when the operator marked a row
    /// `helpful = false` AND the corrected intent differs from
    /// the routed one.
    pub fn incr_intent_mismatch(&self, from: &str, to: &str) {
        if let Ok(mut m) = self.intent_mismatch_total.lock() {
            *m.entry((from.to_string(), to.to_string())).or_insert(0) += 1;
        }
    }

    /// Phase14g — snapshot the mismatch counters. Returns
    /// `[(from, to, count)]` sorted by count desc so the doctor
    /// surfaces the worst rules first.
    pub fn intent_mismatch_snapshot(&self) -> Vec<(String, String, u64)> {
        let map = match self.intent_mismatch_total.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<(String, String, u64)> = map
            .iter()
            .map(|((f, t), n)| (f.clone(), t.clone(), *n))
            .collect();
        out.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
        });
        out
    }

    /// Phase14g — bump the per-path counter for the rewriter
    /// cascade telemetry.
    pub fn incr_rewriter_path(&self, path: &str) {
        if let Ok(mut m) = self.rewriter_path_total.lock() {
            *m.entry(path.to_string()).or_insert(0) += 1;
        }
    }

    /// Phase14g — snapshot the per-path rewriter counters.
    pub fn rewriter_path_snapshot(&self) -> BTreeMap<String, u64> {
        self.rewriter_path_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// phase26c §2.3 — bump the bundle-cache hit counter.
    pub fn incr_cache_hit(&self) {
        self.cache_hit_total.fetch_add(1, Ordering::Relaxed);
    }

    /// phase26c §2.3 — bump the bundle-cache miss counter.
    pub fn incr_cache_miss(&self) {
        self.cache_miss_total.fetch_add(1, Ordering::Relaxed);
    }

    /// phase26c §2.3 — read the current hit/miss counts as a tuple.
    pub fn cache_counters(&self) -> (u64, u64) {
        (
            self.cache_hit_total.load(Ordering::Relaxed),
            self.cache_miss_total.load(Ordering::Relaxed),
        )
    }

    /// phase26e §3 — quantiles of the recorded pre-thinking *assembly*
    /// latency (`cortex.prethink.latency_ms`, populated by the pipeline
    /// run). Returns `(count, p50, p95, p99)`. This is the TRUE
    /// bundle-assembly latency, distinct from the dashboard's
    /// `pre_thinking_p95_ms` series — which is the p95 of generic
    /// envelope `duration_ms` and therefore reflects unrelated
    /// long-running tool_calls (phase26d gap C). Empty → all zeros.
    pub fn latency_quantiles(&self) -> (u64, u32, u32, u32) {
        let samples = match self.latency_ms.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if samples.is_empty() {
            return (0, 0, 0, 0);
        }
        let mut sorted: Vec<u32> = samples
            .iter()
            .map(|&ms| u32::try_from(ms).unwrap_or(u32::MAX))
            .collect();
        sorted.sort_unstable();
        (
            sorted.len() as u64,
            percentile(&sorted, 0.50),
            percentile(&sorted, 0.95),
            percentile(&sorted, 0.99),
        )
    }

    /// Phase14f — snapshot per-intent bundle-byte quantiles. Each
    /// inner map carries `{ count, p50, p95, p99 }`. Empty intents
    /// are omitted.
    pub fn bundle_bytes_quantiles_per_intent(&self) -> BTreeMap<String, IntentByteQuantiles> {
        let mut out: BTreeMap<String, IntentByteQuantiles> = BTreeMap::new();
        let map = match self.bundle_bytes_per_intent.lock() {
            Ok(g) => g,
            Err(_) => return out,
        };
        for (intent, samples) in map.iter() {
            if samples.is_empty() {
                continue;
            }
            let mut sorted = samples.clone();
            sorted.sort_unstable();
            let q = IntentByteQuantiles {
                count: sorted.len() as u64,
                p50: percentile(&sorted, 0.50),
                p95: percentile(&sorted, 0.95),
                p99: percentile(&sorted, 0.99),
            };
            out.insert(intent.clone(), q);
        }
        out
    }

    /// Phase14f — snapshot per-intent `(helpful, unhelpful, rate)`
    /// triples driven by feedback POSTs. `rate = helpful /
    /// (helpful + unhelpful)`; `None` when both are zero.
    pub fn helpful_rate_per_intent(&self) -> BTreeMap<String, IntentHelpfulRate> {
        let mut out: BTreeMap<String, IntentHelpfulRate> = BTreeMap::new();
        let map = match self.helpful_total.lock() {
            Ok(g) => g,
            Err(_) => return out,
        };
        // Build temporary aggregate so a single pass covers both
        // halves of the (intent, helpful) pair.
        let mut tmp: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for ((intent, helpful), n) in map.iter() {
            let e = tmp.entry(intent.clone()).or_insert((0, 0));
            if *helpful {
                e.0 += n;
            } else {
                e.1 += n;
            }
        }
        for (intent, (helpful, unhelpful)) in tmp {
            let total = helpful + unhelpful;
            let rate = if total == 0 {
                None
            } else {
                Some(helpful as f64 / total as f64)
            };
            out.insert(
                intent,
                IntentHelpfulRate {
                    helpful,
                    unhelpful,
                    rate,
                },
            );
        }
        out
    }
}

/// Phase14f — per-intent bundle-byte quantile row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentByteQuantiles {
    /// Sample count.
    pub count: u64,
    /// 50th percentile (median).
    pub p50: u32,
    /// 95th percentile.
    pub p95: u32,
    /// 99th percentile.
    pub p99: u32,
}

/// Phase14f — per-intent helpful-rate row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntentHelpfulRate {
    /// Helpful=true count.
    pub helpful: u64,
    /// Helpful=false count.
    pub unhelpful: u64,
    /// `helpful / (helpful + unhelpful)`; `None` when both zero.
    pub rate: Option<f64>,
}

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_bytes_quantiles_per_intent_picks_p50_p95_p99() {
        let m = Metrics::new();
        for n in 1u32..=100 {
            m.observe_bundle_bytes_per_intent("explain", n);
        }
        let q = m.bundle_bytes_quantiles_per_intent();
        let row = q.get("explain").unwrap();
        assert_eq!(row.count, 100);
        // round((100-1)*0.50) = 50 → sorted[50] = 51
        assert_eq!(row.p50, 51);
        // round((100-1)*0.95) = 94 → sorted[94] = 95
        assert_eq!(row.p95, 95);
        // round((100-1)*0.99) = 98 → sorted[98] = 99
        assert_eq!(row.p99, 99);
    }

    #[test]
    fn latency_quantiles_picks_p50_p95_p99_and_is_empty_safe() {
        // phase26e §3 — empty histogram → all zeros.
        let m = Metrics::new();
        assert_eq!(m.latency_quantiles(), (0, 0, 0, 0));
        // 1..=100 ms → same percentile indices as the bundle-bytes test.
        for n in 1u64..=100 {
            m.observe_latency_ms(n);
        }
        let (count, p50, p95, p99) = m.latency_quantiles();
        assert_eq!(count, 100);
        assert_eq!(p50, 51);
        assert_eq!(p95, 95);
        assert_eq!(p99, 99);
    }

    #[test]
    fn helpful_rate_per_intent_computes_ratio() {
        let m = Metrics::new();
        for _ in 0..3 {
            m.incr_helpful("explain", true);
        }
        m.incr_helpful("explain", false);
        m.incr_helpful("law_check", true);
        let r = m.helpful_rate_per_intent();
        let e = r.get("explain").unwrap();
        assert_eq!(e.helpful, 3);
        assert_eq!(e.unhelpful, 1);
        assert!((e.rate.unwrap() - 0.75).abs() < 1e-9);
        let lc = r.get("law_check").unwrap();
        assert_eq!(lc.rate, Some(1.0));
    }

    #[test]
    fn empty_intent_returns_zero_quantile_row_only_when_present() {
        let m = Metrics::new();
        let q = m.bundle_bytes_quantiles_per_intent();
        assert!(q.is_empty());
    }

    #[test]
    fn intent_mismatch_snapshot_orders_by_count_desc() {
        let m = Metrics::new();
        for _ in 0..2 {
            m.incr_intent_mismatch("explain", "decision_lookup");
        }
        m.incr_intent_mismatch("explain", "law_check");
        for _ in 0..5 {
            m.incr_intent_mismatch("pre_change_context", "similar_problems");
        }
        let snap = m.intent_mismatch_snapshot();
        assert_eq!(snap.len(), 3);
        // worst pair first
        assert_eq!(snap[0].0, "pre_change_context");
        assert_eq!(snap[0].1, "similar_problems");
        assert_eq!(snap[0].2, 5);
        // tie broken by from then to (asc)
        assert_eq!(snap[1].2, 2);
        assert_eq!(snap[1].1, "decision_lookup");
        assert_eq!(snap[2].2, 1);
    }

    #[test]
    fn rewriter_path_snapshot_aggregates_by_path() {
        let m = Metrics::new();
        m.incr_rewriter_path("sonnet_hit");
        m.incr_rewriter_path("sonnet_hit");
        m.incr_rewriter_path("sonnet_cache_hit");
        m.incr_rewriter_path("deterministic_fallback");
        let snap = m.rewriter_path_snapshot();
        assert_eq!(snap.get("sonnet_hit").copied(), Some(2));
        assert_eq!(snap.get("sonnet_cache_hit").copied(), Some(1));
        assert_eq!(snap.get("deterministic_fallback").copied(), Some(1));
    }
}
