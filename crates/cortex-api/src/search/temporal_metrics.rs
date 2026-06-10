//! Phase18 §7.2 — per-request temporal/branch/cross-project counters
//! exposed through the Prometheus `/metrics` endpoint.
//!
//! Mirrors `LoaderMetrics`: atomics for scalar counters,
//! `Mutex<BTreeMap<String, u64>>` for labelled ones. The orchestrator
//! holds an `Option<Arc<TemporalMetrics>>` — `None` makes every
//! increment a compile-time no-op so tests that build `Orchestrator::new`
//! without wiring the handle are unaffected.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Combined registry for the temporal classifier, branch-resolution,
/// and cross-project propagation wedges.
#[derive(Debug, Default)]
pub struct TemporalMetrics {
    /// `cortex_temporal_queries_total` — incremented once per
    /// classifier wedge run (i.e. once per request that has the
    /// temporal snapshot enabled).
    pub queries_total: AtomicU64,
    /// `cortex_temporal_queries_as_of_total` — incremented when the
    /// request pinned a non-now `as_of` override.
    pub queries_as_of_total: AtomicU64,
    /// `cortex_temporal_classified_total{state}` — per-state cumulative
    /// count across all classifier runs.
    pub classified_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex_temporal_action_total{action}` — per-action cumulative
    /// count (pass / boost / demote / drop).
    pub action_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex_branch_queries_total{kind}` — key `kind` ∈ {`main`,
    /// `non_main`}. Raw branch ids are intentionally collapsed to avoid
    /// unbounded cardinality.
    pub branch_total: Mutex<BTreeMap<String, u64>>,
    /// `cortex_cross_project_propagation_runs_total` — incremented
    /// once per `propagate_cross_project` call that finds at least one
    /// eligible project.
    pub cross_project_runs_total: AtomicU64,
    /// `cortex_cross_project_discovered_total` — cumulative edge count
    /// returned by the graph query in the cross-project wedge.
    pub cross_project_discovered_total: AtomicU64,
    /// `cortex_cross_project_propagated_total` — cumulative count of
    /// hits that survived the temporal classifier inside the
    /// cross-project wedge.
    pub cross_project_propagated_total: AtomicU64,
}

impl TemporalMetrics {
    /// Fresh registry with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment `queries_total` by one.
    pub fn record_query(&self) {
        self.queries_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment `queries_as_of_total` by one (call when the request
    /// pinned a non-wall-clock `as_of`).
    pub fn record_as_of(&self) {
        self.queries_as_of_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Add `n` to the per-state counter. `state` should be one of
    /// `valid` / `temporal` / `superseded` / `expired` /
    /// `not_yet_valid` / `abandoned`.
    pub fn record_state(&self, state: &str, n: u64) {
        if n == 0 {
            return;
        }
        if let Ok(mut m) = self.classified_total.lock() {
            *m.entry(state.to_string()).or_insert(0) += n;
        }
    }

    /// Add `n` to the per-action counter. `action` should be one of
    /// `pass` / `boost` / `demote` / `drop`.
    pub fn record_action(&self, action: &str, n: u64) {
        if n == 0 {
            return;
        }
        if let Ok(mut m) = self.action_total.lock() {
            *m.entry(action.to_string()).or_insert(0) += n;
        }
    }

    /// Increment the branch counter. `is_main = true` increments
    /// `kind="main"`, otherwise `kind="non_main"`.
    pub fn record_branch(&self, is_main: bool) {
        let key = if is_main { "main" } else { "non_main" };
        if let Ok(mut m) = self.branch_total.lock() {
            *m.entry(key.to_string()).or_insert(0) += 1;
        }
    }

    /// Record one cross-project propagation run: bumps `runs_total` by
    /// 1 and adds `discovered` / `propagated` to the corresponding
    /// cumulative counters.
    pub fn record_cross_project(&self, discovered: u64, propagated: u64) {
        self.cross_project_runs_total
            .fetch_add(1, Ordering::Relaxed);
        self.cross_project_discovered_total
            .fetch_add(discovered, Ordering::Relaxed);
        self.cross_project_propagated_total
            .fetch_add(propagated, Ordering::Relaxed);
    }

    // -- snapshot helpers (used by render_prom) --------------------------------

    fn classified_snapshot(&self) -> BTreeMap<String, u64> {
        self.classified_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    fn action_snapshot(&self) -> BTreeMap<String, u64> {
        self.action_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    fn branch_snapshot(&self) -> BTreeMap<String, u64> {
        self.branch_total
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Render all temporal metrics in Prometheus text exposition
    /// format (v0.0.4). Mirrors `LoaderMetrics::render_prom`.
    pub fn render_prom(&self) -> String {
        let mut out = String::new();

        // --- scalar counters ---
        out.push_str("# TYPE cortex_temporal_queries_total counter\n");
        let _ = writeln!(
            out,
            "cortex_temporal_queries_total {}",
            self.queries_total.load(Ordering::Relaxed)
        );

        out.push_str("# TYPE cortex_temporal_queries_as_of_total counter\n");
        let _ = writeln!(
            out,
            "cortex_temporal_queries_as_of_total {}",
            self.queries_as_of_total.load(Ordering::Relaxed)
        );

        // --- labelled counters ---
        out.push_str("# TYPE cortex_temporal_classified_total counter\n");
        for (state, n) in self.classified_snapshot() {
            let _ = writeln!(
                out,
                "cortex_temporal_classified_total{{state=\"{state}\"}} {n}"
            );
        }

        out.push_str("# TYPE cortex_temporal_action_total counter\n");
        for (action, n) in self.action_snapshot() {
            let _ = writeln!(
                out,
                "cortex_temporal_action_total{{action=\"{action}\"}} {n}"
            );
        }

        out.push_str("# TYPE cortex_branch_queries_total counter\n");
        for (kind, n) in self.branch_snapshot() {
            let _ = writeln!(out, "cortex_branch_queries_total{{kind=\"{kind}\"}} {n}");
        }

        // --- cross-project counters ---
        out.push_str("# TYPE cortex_cross_project_propagation_runs_total counter\n");
        let _ = writeln!(
            out,
            "cortex_cross_project_propagation_runs_total {}",
            self.cross_project_runs_total.load(Ordering::Relaxed)
        );

        out.push_str("# TYPE cortex_cross_project_discovered_total counter\n");
        let _ = writeln!(
            out,
            "cortex_cross_project_discovered_total {}",
            self.cross_project_discovered_total.load(Ordering::Relaxed)
        );

        out.push_str("# TYPE cortex_cross_project_propagated_total counter\n");
        let _ = writeln!(
            out,
            "cortex_cross_project_propagated_total {}",
            self.cross_project_propagated_total.load(Ordering::Relaxed)
        );

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_registry_is_zeroed() {
        let m = TemporalMetrics::new();
        assert_eq!(m.queries_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.queries_as_of_total.load(Ordering::Relaxed), 0);
        assert!(m.classified_snapshot().is_empty());
        assert!(m.action_snapshot().is_empty());
        assert!(m.branch_snapshot().is_empty());
        assert_eq!(m.cross_project_runs_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.cross_project_discovered_total.load(Ordering::Relaxed), 0);
        assert_eq!(m.cross_project_propagated_total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_query_and_as_of_accumulate() {
        let m = TemporalMetrics::new();
        m.record_query();
        m.record_query();
        m.record_as_of();
        assert_eq!(m.queries_total.load(Ordering::Relaxed), 2);
        assert_eq!(m.queries_as_of_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn record_state_accumulates_per_label() {
        let m = TemporalMetrics::new();
        m.record_state("valid", 3);
        m.record_state("valid", 2);
        m.record_state("superseded", 7);
        let snap = m.classified_snapshot();
        assert_eq!(snap.get("valid"), Some(&5));
        assert_eq!(snap.get("superseded"), Some(&7));
    }

    #[test]
    fn record_action_accumulates_per_label() {
        let m = TemporalMetrics::new();
        m.record_action("pass", 4);
        m.record_action("drop", 1);
        m.record_action("pass", 1);
        let snap = m.action_snapshot();
        assert_eq!(snap.get("pass"), Some(&5));
        assert_eq!(snap.get("drop"), Some(&1));
    }

    #[test]
    fn record_branch_tracks_main_vs_non_main() {
        let m = TemporalMetrics::new();
        m.record_branch(true);
        m.record_branch(true);
        m.record_branch(false);
        let snap = m.branch_snapshot();
        assert_eq!(snap.get("main"), Some(&2));
        assert_eq!(snap.get("non_main"), Some(&1));
    }

    #[test]
    fn record_zero_state_and_action_does_not_create_entry() {
        let m = TemporalMetrics::new();
        m.record_state("valid", 0);
        m.record_action("pass", 0);
        assert!(m.classified_snapshot().is_empty());
        assert!(m.action_snapshot().is_empty());
    }

    #[test]
    fn record_cross_project_accumulates() {
        let m = TemporalMetrics::new();
        m.record_cross_project(10, 6);
        m.record_cross_project(5, 3);
        assert_eq!(m.cross_project_runs_total.load(Ordering::Relaxed), 2);
        assert_eq!(m.cross_project_discovered_total.load(Ordering::Relaxed), 15);
        assert_eq!(m.cross_project_propagated_total.load(Ordering::Relaxed), 9);
    }

    #[test]
    fn render_prom_contains_expected_metric_names_and_labelled_sample() {
        let m = TemporalMetrics::new();
        m.record_query();
        m.record_state("valid", 2);
        m.record_action("pass", 2);
        m.record_branch(true);
        m.record_cross_project(4, 2);

        let txt = m.render_prom();

        assert!(txt.contains("cortex_temporal_queries_total 1"), "{txt}");
        assert!(
            txt.contains("cortex_temporal_classified_total{state=\"valid\"} 2"),
            "{txt}"
        );
        assert!(
            txt.contains("cortex_temporal_action_total{action=\"pass\"} 2"),
            "{txt}"
        );
        assert!(
            txt.contains("cortex_branch_queries_total{kind=\"main\"} 1"),
            "{txt}"
        );
        assert!(
            txt.contains("cortex_cross_project_propagation_runs_total 1"),
            "{txt}"
        );
        assert!(
            txt.contains("cortex_cross_project_discovered_total 4"),
            "{txt}"
        );
        assert!(
            txt.contains("cortex_cross_project_propagated_total 2"),
            "{txt}"
        );
    }
}
