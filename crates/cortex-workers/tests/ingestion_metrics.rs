//! Integration tests for `cortex_workers::ingestion::metrics`.

use cortex_workers::ingestion::Metrics;
use std::sync::atomic::Ordering;

#[test]
fn renders_as_prometheus_text() {
    let m = Metrics::default();
    m.events_received.fetch_add(5, Ordering::Relaxed);
    m.events_routed_raw.fetch_add(4, Ordering::Relaxed);
    m.events_routed_bootstrap.fetch_add(1, Ordering::Relaxed);
    let out = m.render();
    assert!(out.contains("cortex_events_received 5"));
    assert!(out.contains("cortex_events_routed{stream=\"raw\"} 4"));
    assert!(out.contains("cortex_events_routed{stream=\"bootstrap\"} 1"));
}
