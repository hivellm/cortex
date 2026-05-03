#![cfg(feature = "claude-archive")]
//! Phase11k §4.3 — governance watcher integration test.
//!
//! Modifies a fixture ADR file and asserts the change reaches the
//! emitter within 2 s. The acceptance criterion in the proposal
//! ("change reaches the index within 2 s") presumes the live stack;
//! the IT here exercises the watcher's own latency budget against
//! an in-memory emitter so the contract holds without booting Meili
//! / Synap. The live-stack variant is gated by `CORTEX_GOVERNANCE_IT=1`
//! and lives in `crates/cortex-api/tests/`.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use cortex_workers::claude_archive::governance_watcher::{
    GovernanceChange, GovernanceWatcher, MemoryGovernanceEmitter,
};
use tempfile::TempDir;
use tokio::sync::Mutex;

#[tokio::test]
async fn fixture_adr_change_reaches_emitter_within_2_seconds() {
    let root = TempDir::new().unwrap();
    let adr_rel = ".rulebook/decisions/0001-pick-meili.md";
    fs::create_dir_all(root.path().join(".rulebook/decisions")).unwrap();
    fs::write(
        root.path().join(adr_rel),
        "# ADR-0001\nStatus: accepted\n\nWe pick Meili.\n",
    )
    .unwrap();

    let emitter = MemoryGovernanceEmitter::new();
    let handle = emitter.clone();
    let watcher = Arc::new(Mutex::new(GovernanceWatcher::with_defaults(
        root.path(),
        Box::new(emitter),
    )));

    // Initial seeding tick — the ADR is captured for the first time.
    {
        let mut w = watcher.lock().await;
        w.tick_once();
    }
    assert_eq!(handle.captured().len(), 1, "seeding tick emits the ADR");

    // Mutate the ADR — flip status from accepted to superseded. The
    // watcher's next tick MUST observe a content-hash drift and emit
    // a fresh upsert. Drive at the §4.1 spec'd 1 Hz cadence so the
    // 2-second SLA leaves room for two ticks.
    let started = std::time::Instant::now();
    fs::write(
        root.path().join(adr_rel),
        "# ADR-0001\nStatus: superseded\n\nSuperseded by ADR-0002.\n",
    )
    .unwrap();

    let mut observed = false;
    while started.elapsed() < Duration::from_secs(2) {
        {
            let mut w = watcher.lock().await;
            w.tick_once();
        }
        // The emitter logs every Upserted change; the seeding emit
        // counts as the first, the post-write emit as the second.
        let upserts = handle
            .captured()
            .iter()
            .filter(|c| matches!(c, GovernanceChange::Upserted { .. }))
            .count();
        if upserts >= 2 {
            observed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        observed,
        "watcher must observe ADR mutation within 2 s; captured = {:?}",
        handle.captured(),
    );
    let elapsed_ms = started.elapsed().as_millis();
    assert!(
        elapsed_ms < 2_000,
        "elapsed {}ms exceeded the 2 s SLA",
        elapsed_ms,
    );

    // Pin the wire shape: the second upsert MUST reference the
    // mutated body (post-write content) so the dashboard's
    // re-published envelope reflects the new status.
    let captured = handle.captured();
    let last_upsert = captured
        .iter()
        .rev()
        .find_map(|c| match c {
            GovernanceChange::Upserted { body, .. } => Some(body.clone()),
            _ => None,
        })
        .expect("at least one upsert");
    assert!(
        last_upsert.contains("superseded"),
        "post-write upsert must carry the mutated body; got `{last_upsert}`",
    );
}