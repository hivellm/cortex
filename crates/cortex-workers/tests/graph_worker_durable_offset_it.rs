//! Phase11s §2.5 — graph-worker durable consumer-offset
//! drainage IT.
//!
//! The full live-stack drain test (Synap + Nexus via testcontainers)
//! lives behind `CORTEX_DRAIN_RECOVERY_IT=1` in §4.2. This file
//! pins the §2.3 in-process contract end-to-end:
//!
//! 1. The metadata-backed consumer reads the persisted offset on
//!    boot and seeds the tracker at `last_offset + 1`.
//! 2. Successful `ack` calls persist the new offset.
//! 3. A simulated restart (drop + rebuild the consumer) resumes
//!    from the next event after the last successfully-acked one.
//!
//! Hermetic — no Synap, no Nexus — drives the ledger directly so
//! every CI run exercises the contract.

use std::sync::{Arc, Mutex};

use cortex_storage::MetadataStore;
use cortex_workers::graph::worker::{OffsetTracker, STREAM_ENRICHED};

const CONSUMER_ID: &str = "cortex-graph-0";

/// Mirror the boot-time seed logic that `LiveSynapConsumer::with_persistent_offset`
/// runs against the metadata store: on boot, read the persisted
/// row and seed the tracker at `last_offset + 1`.
fn seed_from_ledger(
    metadata: &Arc<Mutex<MetadataStore>>,
    consumer_id: &str,
    stream: &str,
) -> Arc<OffsetTracker> {
    let tracker = Arc::new(OffsetTracker::new());
    let row = metadata
        .lock()
        .unwrap()
        .consumer_offset_lookup(consumer_id, stream)
        .unwrap();
    if let Some(row) = row {
        tracker.seed(row.last_offset.saturating_add(1));
    }
    tracker
}

#[test]
fn fresh_ledger_seeds_tracker_at_zero() {
    // Phase11s §2.3 — a worker booting against an empty ledger
    // walks the stream from the beginning. `current()` returns 0.
    let metadata = Arc::new(Mutex::new(MetadataStore::open_in_memory().unwrap()));
    let tracker = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(tracker.current(), 0);
}

#[test]
fn populated_ledger_seeds_tracker_at_next_offset() {
    // Phase11s §2.3 — a worker booting against a ledger row at
    // offset 99 must resume from offset 100 (next un-processed
    // event). Critical: the §2.3 contract is "last_offset + 1",
    // NOT "last_offset" — re-processing the last acked envelope
    // would emit duplicate Nexus writes.
    let metadata = Arc::new(Mutex::new(MetadataStore::open_in_memory().unwrap()));
    metadata
        .lock()
        .unwrap()
        .consumer_offset_upsert(CONSUMER_ID, STREAM_ENRICHED, 99, Some("01HEAD"))
        .unwrap();
    let tracker = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(tracker.current(), 100);
}

#[test]
fn simulated_restart_resumes_from_last_acked_offset() {
    // Phase11s §2.5 — full restart cycle: drive a sequence of
    // acks against the ledger, simulate a restart by reading the
    // persisted state into a fresh tracker, and assert the new
    // tracker resumes exactly where the prior one left off.
    let metadata = Arc::new(Mutex::new(MetadataStore::open_in_memory().unwrap()));
    // Initial boot — empty ledger.
    let tracker = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(tracker.current(), 0);

    // Process offsets 0..=42, persist after each successful ack.
    for offset in 0..=42 {
        tracker.advance_past(offset);
        metadata
            .lock()
            .unwrap()
            .consumer_offset_upsert(CONSUMER_ID, STREAM_ENRICHED, offset, None)
            .unwrap();
    }
    assert_eq!(tracker.current(), 43);
    drop(tracker);

    // ---- restart ----
    let tracker_after_restart = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(
        tracker_after_restart.current(),
        43,
        "post-restart tracker must resume at last_acked+1"
    );
}

#[test]
fn replay_subcommand_rewinds_cursor_for_next_boot() {
    // Phase11s §2.4 — `cortex-ops graph replay --since=N` calls
    // `consumer_offset_set(N)`. The next boot resumes from N+1.
    // Pin the contract: rewinding 500 → 100 changes resume from
    // 501 to 101.
    let metadata = Arc::new(Mutex::new(MetadataStore::open_in_memory().unwrap()));
    metadata
        .lock()
        .unwrap()
        .consumer_offset_upsert(CONSUMER_ID, STREAM_ENRICHED, 500, Some("01HEAD"))
        .unwrap();
    let pre_replay = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(pre_replay.current(), 501);

    // Operator runs `cortex-ops graph replay --since=100`.
    metadata
        .lock()
        .unwrap()
        .consumer_offset_set(CONSUMER_ID, STREAM_ENRICHED, 100)
        .unwrap();

    let post_replay = seed_from_ledger(&metadata, CONSUMER_ID, STREAM_ENRICHED);
    assert_eq!(post_replay.current(), 101);
    // The original ledger row is rewritten — `last_event_id` is
    // cleared because the replay window may discover a different
    // head event.
    let row = metadata
        .lock()
        .unwrap()
        .consumer_offset_lookup(CONSUMER_ID, STREAM_ENRICHED)
        .unwrap()
        .unwrap();
    assert!(row.last_event_id.is_none());
}
