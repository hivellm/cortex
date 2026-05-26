//! Phase14b §3 — [`ColdTierPrune`] wraps the
//! [`super::identity_prune::run_identity_cascade`] driver plus the
//! ADR-013 Vectorizer [`crate::embedder::vectorizer_prune::reencode_collection`]
//! step behind the [`Sweep`] trait. Runs weekly Sunday 05:00 UTC
//! with a default cutoff of 365 days; cascades per-event deletes
//! across Meili + Nexus + per-event Vectorizer rows + parquet
//! archive partitions, then re-encodes the cold-binary Vectorizer
//! collection to evict stragglers the per-event cascade missed.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use cron::Schedule;

use crate::embedder::vectorizer_prune::{
    reencode_collection, PrunePredicate, VectorizerPruneOps,
};
use crate::retention::identity_prune::{
    render_cascade_summary, run_identity_cascade, CascadePolicy, IdentityCascadeOps,
    IdentitySource,
};
use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

/// Canonical sweep name.
pub const COLD_TIER_PRUNE_NAME: &str = "cold_tier_prune";

/// Default cron schedule — Sunday 05:00 UTC weekly. Picked one
/// hour after the daily hot-tier prune (04:00 UTC) so the cold-
/// tier pass observes the post-hot state on Sundays. The `cron`
/// crate keys day-of-week 1..=7 with Sunday=1.
pub const COLD_TIER_PRUNE_SCHEDULE: &str = "0 0 5 * * 1 *";

/// Default cutoff in days — events older than this leave EVERY
/// backend (Meili / Nexus / Vectorizer / archive).
pub const COLD_TIER_DEFAULT_CUTOFF_DAYS: i64 = 365;

/// Canonical Vectorizer cold-binary collection name. Cold-tier
/// prune is the sole caller of `reencode_collection` per ADR-013.
pub const COLD_BINARY_COLLECTION: &str = "cortex.cold.binary";

/// Identity-driven cold-tier prune Sweep. Drops every backend +
/// re-encodes the cold-binary Vectorizer collection.
pub struct ColdTierPrune {
    source: Arc<dyn IdentitySource>,
    cascade_ops: Arc<dyn IdentityCascadeOps>,
    vectorizer_ops: Arc<dyn VectorizerPruneOps>,
    cutoff_days: i64,
}

impl ColdTierPrune {
    /// Build a cold-tier prune with the default 365-day cutoff.
    pub fn new(
        source: Arc<dyn IdentitySource>,
        cascade_ops: Arc<dyn IdentityCascadeOps>,
        vectorizer_ops: Arc<dyn VectorizerPruneOps>,
    ) -> Self {
        Self {
            source,
            cascade_ops,
            vectorizer_ops,
            cutoff_days: COLD_TIER_DEFAULT_CUTOFF_DAYS,
        }
    }

    /// Override the cutoff (for tests + operator overrides).
    #[must_use]
    pub fn with_cutoff_days(mut self, days: i64) -> Self {
        self.cutoff_days = days;
        self
    }

    /// Active cutoff in days. Public so the dashboard can render it.
    pub fn cutoff_days(&self) -> i64 {
        self.cutoff_days
    }
}

#[async_trait]
impl Sweep for ColdTierPrune {
    fn name(&self) -> &'static str {
        COLD_TIER_PRUNE_NAME
    }

    fn schedule(&self) -> Schedule {
        Schedule::from_str(COLD_TIER_PRUNE_SCHEDULE)
            .expect("COLD_TIER_PRUNE_SCHEDULE is a constant valid 7-field cron expression")
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let started = ctx.now;
        let report = SweepReport::started(COLD_TIER_PRUNE_NAME, started);
        let cutoff = ctx.now - Duration::days(self.cutoff_days);

        if ctx.config.dry_run {
            let expired = self
                .source
                .expired_identities(cutoff)
                .await
                .map_err(|e| anyhow::anyhow!("dry-run expired query: {e}"))?;
            let next = report.with_tier_transition("cold:dry_run", expired.len() as u64);
            return Ok(next.finish_success(ctx.now, expired.len() as u64, 0));
        }

        // Per-event cascade — Meili + Nexus + per-event vector +
        // archive partition rewrite.
        let cascade = run_identity_cascade(
            Arc::clone(&self.source),
            Arc::clone(&self.cascade_ops),
            CascadePolicy::COLD,
            cutoff,
        )
        .await;
        let cascade_report = match cascade {
            Ok(c) => c,
            Err(err) => return Ok(report.finish_failed(ctx.now, 0, 0, err.to_string())),
        };

        // Collection-level Vectorizer re-encode for cold.binary —
        // ADR-013 §Decision. The predicate keeps every survivor
        // whose payload `occurred_at_ms` is at-or-after the cutoff
        // instant.
        let cutoff_ms = cutoff.timestamp_millis();
        let predicate: PrunePredicate = Arc::new(move |payload| {
            let ts = payload
                .get("occurred_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            ts >= cutoff_ms
        });
        let reencode = reencode_collection(
            self.vectorizer_ops.as_ref(),
            COLD_BINARY_COLLECTION,
            predicate,
        )
        .await;

        let cascade_summary = render_cascade_summary(CascadePolicy::COLD, &cascade_report);
        match reencode {
            Ok(r) => {
                let next = report
                    .with_tier_transition("cold:cascade_ok", cascade_report.ok)
                    .with_tier_transition("cold:cascade_failed", cascade_report.failed)
                    .with_tier_transition("cold:vectorizer_kept", r.kept)
                    .with_tier_transition("cold:vectorizer_dropped", r.dropped);
                tracing::info!(
                    sweep = COLD_TIER_PRUNE_NAME,
                    %cascade_summary,
                    re_kept = r.kept,
                    re_dropped = r.dropped,
                    "cold-tier prune done"
                );
                if cascade_report.failed > 0 {
                    Ok(next.finish_failed(
                        ctx.now,
                        cascade_report.processed,
                        0,
                        format!(
                            "{cascade_summary} ({} of {} cascade legs failed); reencode kept {}/dropped {}",
                            cascade_report.failed, cascade_report.processed, r.kept, r.dropped
                        ),
                    ))
                } else {
                    Ok(next.finish_success(ctx.now, cascade_report.processed + r.dropped, 0))
                }
            }
            Err(err) => Ok(report
                .with_tier_transition("cold:cascade_ok", cascade_report.ok)
                .with_tier_transition("cold:cascade_failed", cascade_report.failed)
                .finish_failed(
                    ctx.now,
                    cascade_report.processed,
                    0,
                    format!("{cascade_summary}; vectorizer reencode failed: {err}"),
                )),
        }
    }

    fn report_view(&self, report: &SweepReport) -> SweepReportView {
        report.view()
    }
}

/// Helper used by callers that want to convert a `DateTime<Utc>`
/// cutoff into the `occurred_at_ms` form the cold-tier predicate
/// expects. Exposed so production embedder payloads can be probed
/// in tests without re-deriving the formula.
pub fn occurred_at_ms_for(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::vectorizer_prune::{MemoryVectorizerPruneOps, VectorRecord};
    use crate::retention::identity_prune::{RecordingCascadeOps, StaticIdentitySource};
    use crate::sweep::SweepStatus;
    use cortex_storage::identity::EventIdentity;
    use cortex_storage::MetadataStore;
    use serde_json::json;
    use tokio::sync::Mutex as TokioMutex;

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("rfc3339")
    }

    fn row(event: &str, partition: &str) -> EventIdentity {
        EventIdentity {
            event_id: event.into(),
            nexus_id: Some(format!("nxs-{event}")),
            vec_id: Some(format!("vec-{event}")),
            meili_id: Some(format!("mli-{event}")),
            archive_partition: Some(partition.into()),
        }
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(TokioMutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.cold_tier_prune").with_now(now)
    }

    fn vec_record(id: &str, occurred_at_ms: i64) -> VectorRecord {
        VectorRecord {
            id: id.into(),
            vector: vec![0.0, 0.1, 0.2, 0.3],
            payload: json!({"event_id": id, "occurred_at_ms": occurred_at_ms}),
        }
    }

    #[tokio::test]
    async fn cold_tier_prune_cascades_all_backends_and_reencodes_cold_binary() {
        let now = ts("2026-05-25T05:00:00Z");
        // Identity rows: one expired (~3 years old), one fresh.
        let source = Arc::new(StaticIdentitySource::new(vec![
            row(
                "e-old",
                "events/year=2023/month=01/day=01/hour=00/raw.parquet",
            ),
            row(
                "e-young",
                "events/year=2026/month=05/day=15/hour=00/raw.parquet",
            ),
        ]));
        let cascade_ops = Arc::new(RecordingCascadeOps::new());
        let vec_ops = Arc::new(MemoryVectorizerPruneOps::new());
        // Cold-binary collection: 4 survivors (≥ cutoff), 2 expired.
        let cutoff_ms = (now - Duration::days(COLD_TIER_DEFAULT_CUTOFF_DAYS)).timestamp_millis();
        vec_ops
            .seed(
                COLD_BINARY_COLLECTION,
                vec![
                    vec_record("alive-1", cutoff_ms + 1_000),
                    vec_record("alive-2", cutoff_ms + 60_000),
                    vec_record("alive-3", cutoff_ms + 3_600_000),
                    vec_record("alive-4", cutoff_ms + 86_400_000),
                    vec_record("dead-1", cutoff_ms - 10_000_000),
                    vec_record("dead-2", cutoff_ms - 20_000_000),
                ],
            )
            .await;
        let sweep = ColdTierPrune::new(source.clone(), cascade_ops.clone(), vec_ops.clone());
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.name, COLD_TIER_PRUNE_NAME);

        // Cascade reached every backend including archive.
        assert_eq!(cascade_ops.meili_calls().await, vec!["mli-e-old"]);
        assert_eq!(cascade_ops.nexus_calls().await, vec!["nxs-e-old"]);
        assert_eq!(cascade_ops.vector_calls().await, vec!["vec-e-old"]);
        assert_eq!(cascade_ops.archive_calls().await.len(), 1);
        assert!(cascade_ops.archive_calls().await[0].0.contains("year=2023"));

        // Re-encode kept the 4 survivors, dropped the 2 expired.
        let live = vec_ops.snapshot(COLD_BINARY_COLLECTION).await;
        assert_eq!(live.len(), 4);
        assert!(live.iter().all(|r| r.id.starts_with("alive")));

        // Per-tier-transition counters populated.
        assert_eq!(*report.tier_transitions.get("cold:cascade_ok").unwrap_or(&0), 1);
        assert_eq!(*report.tier_transitions.get("cold:vectorizer_kept").unwrap_or(&0), 4);
        assert_eq!(*report.tier_transitions.get("cold:vectorizer_dropped").unwrap_or(&0), 2);

        // Young identity preserved, old identity dropped.
        let known = source.known_ids().await;
        assert!(!known.contains("e-old"));
        assert!(known.contains("e-young"));
    }

    #[tokio::test]
    async fn cold_tier_prune_handles_100_events_30_hot_70_cold_scenario() {
        // Phase14b §3.4 — 100 events: 30 hot + 70 cold. Post-prune
        // asserts the cold ones are gone everywhere.
        let now = ts("2026-05-25T05:00:00Z");
        let mut rows = Vec::with_capacity(100);
        for i in 0..30 {
            // hot — within the last year
            rows.push(row(
                &format!("hot-{i:03}"),
                &format!("events/year=2026/month=04/day=01/hour={:02}/raw.parquet", i % 24),
            ));
        }
        for i in 0..70 {
            // cold — > 365 days old
            rows.push(row(
                &format!("cold-{i:03}"),
                &format!("events/year=2024/month=01/day=01/hour={:02}/raw.parquet", i % 24),
            ));
        }
        let source = Arc::new(StaticIdentitySource::new(rows));
        let cascade_ops = Arc::new(RecordingCascadeOps::new());
        let vec_ops = Arc::new(MemoryVectorizerPruneOps::new());
        let sweep = ColdTierPrune::new(source.clone(), cascade_ops.clone(), vec_ops);
        let ctx = make_ctx(now);

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 70, "every cold event cascaded");

        // Every cold backend leg fired once per cold event.
        assert_eq!(cascade_ops.meili_calls().await.len(), 70);
        assert_eq!(cascade_ops.nexus_calls().await.len(), 70);
        assert_eq!(cascade_ops.vector_calls().await.len(), 70);
        assert_eq!(cascade_ops.archive_calls().await.len(), 70);

        // Identity rows: only the 30 hot survivors remain.
        let known = source.known_ids().await;
        assert_eq!(known.len(), 30);
        assert!(known.iter().all(|id| id.starts_with("hot-")));
    }

    #[tokio::test]
    async fn cold_tier_prune_dry_run_short_circuits_before_any_leg() {
        let now = ts("2026-05-25T05:00:00Z");
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-old",
            "events/year=2024/month=01/day=01/hour=00/raw.parquet",
        )]));
        let cascade_ops = Arc::new(RecordingCascadeOps::new());
        let vec_ops = Arc::new(MemoryVectorizerPruneOps::new());
        let sweep = ColdTierPrune::new(source.clone(), cascade_ops.clone(), vec_ops);
        let mut ctx = make_ctx(now);
        ctx.config.dry_run = true;

        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 1);
        assert!(cascade_ops.meili_calls().await.is_empty());
        assert!(source.known_ids().await.contains("e-old"));
    }

    #[test]
    fn cold_tier_prune_schedule_parses_and_cutoff_default_is_365_days() {
        let source = Arc::new(StaticIdentitySource::new(vec![]));
        let cascade_ops = Arc::new(RecordingCascadeOps::new());
        let vec_ops = Arc::new(MemoryVectorizerPruneOps::new());
        let sweep = ColdTierPrune::new(source, cascade_ops, vec_ops);
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "cold_tier_prune");
        assert_eq!(sweep.cutoff_days(), COLD_TIER_DEFAULT_CUTOFF_DAYS);
    }

    #[test]
    fn occurred_at_ms_for_is_millis_helper() {
        let t = ts("2026-05-25T05:00:00Z");
        assert_eq!(occurred_at_ms_for(t), t.timestamp_millis());
    }
}
