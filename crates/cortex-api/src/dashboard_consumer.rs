//! Dashboard event consumer (spec 21).
//!
//! Parses raw [`DashboardEvent`] JSON values from any source (typically
//! the `cortex.events.dashboard` Synap stream — wired in phase11n) and
//! forwards them to the shared [`DashboardEventBus`]. Dedupes by
//! `event_id` over a sliding window so the same event arriving from
//! both the watcher and the Synap path surfaces only once.
//!
//! The Synap pull-loop itself is not wired here — that lands in
//! phase11n once the rulebook MCP publisher exists. This module is the
//! shape the loop will hand events to.

use std::collections::VecDeque;

use serde_json::Value;

use cortex_core::DashboardEvent;

use crate::dashboard_watcher::DashboardEventBus;

/// Sliding-window capacity for the dedup ring. 1024 ids covers ~1 s of
/// peak rulebook traffic with margin; older ids fall out and would be
/// re-published if they re-appeared, which is fine — the GUI's
/// `invalidateQueries` is idempotent.
pub const DEDUP_WINDOW: usize = 1024;

/// Counters surfaced for tests and (eventually) Prometheus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerMetrics {
    /// Events successfully forwarded to the bus.
    pub forwarded: u64,
    /// Duplicates filtered out by the dedup window.
    pub deduped: u64,
    /// Payloads that failed to parse as a [`DashboardEvent`].
    pub parse_errors: u64,
}

/// Stateful consumer. Holds the dedup ring + counters; not `Sync`. Wrap
/// in a `Mutex` if multiple producer threads need to share one consumer.
pub struct DashboardEventConsumer {
    bus: DashboardEventBus,
    seen: VecDeque<String>,
    seen_capacity: usize,
    metrics: ConsumerMetrics,
}

impl DashboardEventConsumer {
    /// Build a consumer that publishes into `bus` and dedupes against a
    /// 1024-id sliding window.
    pub fn new(bus: DashboardEventBus) -> Self {
        Self::with_capacity(bus, DEDUP_WINDOW)
    }

    /// Build a consumer with an explicit dedup window. Useful for tests
    /// that want to exercise eviction at a small N.
    pub fn with_capacity(bus: DashboardEventBus, seen_capacity: usize) -> Self {
        Self {
            bus,
            seen: VecDeque::with_capacity(seen_capacity),
            seen_capacity,
            metrics: ConsumerMetrics::default(),
        }
    }

    /// Counters snapshot.
    pub fn metrics(&self) -> ConsumerMetrics {
        self.metrics
    }

    /// Ingest a single raw payload. Returns `true` when the event was
    /// forwarded, `false` when it was dropped (duplicate or parse
    /// failure). Errors update [`ConsumerMetrics`] but never propagate;
    /// the upstream loop should log and continue.
    pub fn ingest(&mut self, payload: Value) -> bool {
        let event: DashboardEvent = match serde_json::from_value(payload) {
            Ok(e) => e,
            Err(_) => {
                self.metrics.parse_errors = self.metrics.parse_errors.saturating_add(1);
                return false;
            }
        };
        self.ingest_event(event)
    }

    /// Variant that takes an already-parsed event. Useful when a caller
    /// constructs the event directly (the watcher path does this).
    pub fn ingest_event(&mut self, event: DashboardEvent) -> bool {
        if self.seen.iter().any(|id| id == &event.event_id) {
            self.metrics.deduped = self.metrics.deduped.saturating_add(1);
            return false;
        }
        if self.seen.len() == self.seen_capacity {
            self.seen.pop_front();
        }
        self.seen.push_back(event.event_id.clone());
        self.bus.publish(event);
        self.metrics.forwarded = self.metrics.forwarded.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::{DashboardEventKind, DashboardEventSource};
    use serde_json::json;

    fn make_event(id: &str, entity: &str) -> DashboardEvent {
        DashboardEvent {
            event_id: id.to_string(),
            kind: DashboardEventKind::TaskChanged,
            entity_id: entity.to_string(),
            summary: None,
            ts: "2026-05-02T00:00:00Z".to_string(),
            delta: None,
            source: DashboardEventSource::Mcp,
        }
    }

    #[tokio::test]
    async fn dedupes_repeat_event_ids() {
        let bus = DashboardEventBus::new();
        let mut rx = bus.subscribe();
        let mut consumer = DashboardEventConsumer::new(bus);

        let a = make_event("01J_A", "task_a");
        let b = make_event("01J_B", "task_b");
        let a_dup = make_event("01J_A", "task_a_again");

        assert!(consumer.ingest_event(a.clone()));
        assert!(consumer.ingest_event(b.clone()));
        assert!(!consumer.ingest_event(a_dup));

        let metrics = consumer.metrics();
        assert_eq!(metrics.forwarded, 2);
        assert_eq!(metrics.deduped, 1);
        assert_eq!(metrics.parse_errors, 0);

        let first = rx.recv().await.expect("a");
        let second = rx.recv().await.expect("b");
        assert_eq!(first.event_id, "01J_A");
        assert_eq!(second.event_id, "01J_B");
    }

    #[test]
    fn parse_error_is_counted_not_forwarded() {
        let bus = DashboardEventBus::new();
        let mut consumer = DashboardEventConsumer::new(bus);
        let bad = json!({ "definitely": "not an event" });
        assert!(!consumer.ingest(bad));
        let metrics = consumer.metrics();
        assert_eq!(metrics.forwarded, 0);
        assert_eq!(metrics.parse_errors, 1);
    }

    #[test]
    fn well_formed_payload_ingests_via_value() {
        let bus = DashboardEventBus::new();
        let mut consumer = DashboardEventConsumer::new(bus);
        let payload = json!({
            "event_id": "01J_OK",
            "kind": "task.changed",
            "entity_id": "phase11m_x",
            "ts": "2026-05-02T00:00:00Z",
            "source": "mcp"
        });
        assert!(consumer.ingest(payload));
        assert_eq!(consumer.metrics().forwarded, 1);
    }

    #[test]
    fn evicts_oldest_id_when_window_fills() {
        let bus = DashboardEventBus::new();
        let mut consumer = DashboardEventConsumer::with_capacity(bus, 2);
        assert!(consumer.ingest_event(make_event("01_A", "a")));
        assert!(consumer.ingest_event(make_event("01_B", "b")));
        assert!(consumer.ingest_event(make_event("01_C", "c"))); // evicts A
        // A should re-ingest because it fell out of the window.
        assert!(consumer.ingest_event(make_event("01_A", "a")));
        assert_eq!(consumer.metrics().forwarded, 4);
        assert_eq!(consumer.metrics().deduped, 0);
    }
}
