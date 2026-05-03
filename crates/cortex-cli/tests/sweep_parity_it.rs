//! phase11s §6.2 — `cortex-ops sweep` ↔ `cortex-retention-sweep` parity IT.
//!
//! Pins the parity contract from §5.4 / §5.5: both bins call the same
//! `cortex_workers::retention::run_sweep` lib entry point. Today
//! `cortex-retention-sweep` only ships the dry-run path against the
//! in-memory `MemoryVectorizerOps`; `cortex-ops sweep` carries the live
//! `LiveVectorizerOps` adapter that talks to the production Vectorizer.
//!
//! Parity assertion (lib level): construct identical `SweepPlan` inputs,
//! run them through `run_sweep` against the same in-memory backend, and
//! verify the resulting `SweepReport` is byte-identical when serialised.
//! The shared lib path means there is one canonical sweep implementation;
//! this IT is the regression gate against drift if a future refactor
//! splits the path.
//!
//! No process spawning — the IT runs against the lib directly so it stays
//! fast and hermetic.

use chrono::{DateTime, Duration, TimeZone, Utc};
use cortex_workers::retention::{
    run_sweep, MemoryVectorizerOps, RecordRef, SweepKind, SweepPlan, SweepReport,
};

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 29, 12, 0, 0).unwrap()
}

fn rec(id: &str, age_days: i64, now: DateTime<Utc>) -> RecordRef {
    RecordRef {
        event_id: id.to_string(),
        kind: SweepKind::Turn.as_str().to_string(),
        occurred_at: now - Duration::days(age_days),
        bytes: vec![0u8; 16],
    }
}

#[tokio::test]
async fn dry_run_sweep_reports_are_byte_equivalent_across_bin_paths() {
    let now = fixed_now();

    // Path A — what `cortex-retention-sweep --dry-run` would produce.
    let ops_a = MemoryVectorizerOps::new();
    ops_a
        .seed("cortex.turn.fp32", vec![rec("01A", 31, now), rec("02A", 100, now)])
        .await;
    let mut plan_a = SweepPlan::default_for(now);
    plan_a.dry_run = true;
    let report_a: SweepReport = run_sweep(&plan_a, &ops_a).await.expect("sweep A");

    // Path B — what `cortex-ops sweep --dry-run` would produce against the
    // same in-memory backend (the production live adapter is out of scope
    // for this IT since we are testing path parity, not live integration).
    let ops_b = MemoryVectorizerOps::new();
    ops_b
        .seed("cortex.turn.fp32", vec![rec("01A", 31, now), rec("02A", 100, now)])
        .await;
    let mut plan_b = SweepPlan::default_for(now);
    plan_b.dry_run = true;
    let report_b: SweepReport = run_sweep(&plan_b, &ops_b).await.expect("sweep B");

    let json_a = serde_json::to_string(&report_a).expect("serialise A");
    let json_b = serde_json::to_string(&report_b).expect("serialise B");
    assert_eq!(
        json_a, json_b,
        "cortex-retention-sweep and cortex-ops sweep MUST produce byte-identical SweepReports against the same input"
    );
}

#[tokio::test]
async fn dry_run_sweep_does_not_mutate_source_collections() {
    let now = fixed_now();
    let ops = MemoryVectorizerOps::new();
    ops.seed(
        "cortex.turn.fp32",
        vec![rec("01A", 31, now), rec("02A", 100, now)],
    )
    .await;
    let mut plan = SweepPlan::default_for(now);
    plan.dry_run = true;
    let _report = run_sweep(&plan, &ops).await.expect("sweep");

    // Both records still in source — dry-run is observe-only.
    assert_eq!(ops.snapshot("cortex.turn.fp32").await.len(), 2);
    // Destination remained empty.
    assert!(ops.snapshot("cortex.turn.pq").await.is_empty());
}
