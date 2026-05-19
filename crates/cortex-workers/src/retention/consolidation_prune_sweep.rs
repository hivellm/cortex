//! Phase13a §3.7 — `ConsolidationPruneSweep` wraps the existing
//! consolidation pruner ([`run_sweep`]) behind the [`Sweep`] trait.
//!
//! The wrapper holds:
//!
//! - `provider` — `Arc<dyn ConsolidationDocProvider>` returns the
//!   `cortex_consolidations` documents the engine evaluates.
//!   Production wires a Meili-backed paginator (the
//!   `cortex-ops consolidation-prune` bin already implements this);
//!   tests author scenarios via [`StaticConsolidationDocs`].
//! - `vectorizer` — `Arc<dyn VectorizerClient>` for the warm/cold
//!   demotions.
//! - `meili` — `Arc<dyn MeiliPruneOps>` for the cold-tier field
//!   stripping + expired-row purge.
//! - `meili_index` — `String` (`"cortex_consolidations"` in
//!   production).
//!
//! Legacy [`PruneReport`] folds into [`SweepReport`]:
//!
//! | Legacy field | Trait field |
//! |---|---|
//! | `consolidations_seen` | `tier_transitions["consolidations_seen"]` |
//! | `events_demoted_per_tier[<key>]` | `tier_transitions["demoted:<key>"]` |
//! | `last_run_duration_ms` | `tier_transitions["duration_ms"]` |
//!
//! `rows_processed = sum(events_demoted_per_tier.values())`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cron::Schedule;

use crate::embedder::vectorizer_client::VectorizerClient;
use crate::pruner::engine::{run_sweep, ConsolidationDoc};
use crate::pruner::meili_sink::MeiliPruneOps;
use crate::sweep::{Sweep, SweepCtx, SweepReport, SweepReportView};

use super::scheduler::parse_schedule;

/// Canonical sweep name.
pub const CONSOLIDATION_PRUNE_NAME: &str = "consolidation_prune";

/// Default schedule — daily 03:00 UTC, matches `default_jobs()`
/// row `retention.consolidation_prune` (5-field `"0 3 * * *"`).
pub const CONSOLIDATION_PRUNE_SCHEDULE: &str = "0 3 * * *";

/// Provider for the per-run document slice. Production wires a
/// Meili-backed paginator; tests author static slices.
#[async_trait]
pub trait ConsolidationDocProvider: Send + Sync {
    /// Return every document in `cortex_consolidations` the sweep
    /// should evaluate at `now`.
    async fn docs(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<ConsolidationDoc>>;
}

/// Static doc provider — returns a fixed slice. Used by tests +
/// any caller that has already materialised the document set.
pub struct StaticConsolidationDocs {
    docs: Vec<ConsolidationDoc>,
}

impl StaticConsolidationDocs {
    /// Build a static provider with the supplied document slice.
    pub fn new(docs: Vec<ConsolidationDoc>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl ConsolidationDocProvider for StaticConsolidationDocs {
    async fn docs(&self, _now: DateTime<Utc>) -> anyhow::Result<Vec<ConsolidationDoc>> {
        Ok(self.docs.clone())
    }
}

/// Consolidation-prune sweep wrapped behind the [`Sweep`] trait.
pub struct ConsolidationPruneSweep {
    provider: Arc<dyn ConsolidationDocProvider>,
    vectorizer: Arc<dyn VectorizerClient>,
    meili: Arc<dyn MeiliPruneOps>,
    meili_index: String,
}

impl ConsolidationPruneSweep {
    /// Build the sweep over the supplied dependencies.
    pub fn new(
        provider: Arc<dyn ConsolidationDocProvider>,
        vectorizer: Arc<dyn VectorizerClient>,
        meili: Arc<dyn MeiliPruneOps>,
        meili_index: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            vectorizer,
            meili,
            meili_index: meili_index.into(),
        }
    }
}

#[async_trait]
impl Sweep for ConsolidationPruneSweep {
    fn name(&self) -> &'static str {
        CONSOLIDATION_PRUNE_NAME
    }

    fn schedule(&self) -> Schedule {
        parse_schedule(CONSOLIDATION_PRUNE_SCHEDULE).expect(
            "CONSOLIDATION_PRUNE_SCHEDULE is a constant valid 5-field cron expression",
        )
    }

    async fn run(&self, ctx: &SweepCtx) -> anyhow::Result<SweepReport> {
        let started_at = ctx.now;
        let docs = self.provider.docs(ctx.now).await?;
        let outcome = run_sweep(
            &docs,
            ctx.now,
            self.vectorizer.as_ref(),
            self.meili.as_ref(),
            &self.meili_index,
        )
        .await;

        let report = SweepReport::started(CONSOLIDATION_PRUNE_NAME, started_at);
        match outcome {
            Ok(legacy) => {
                let mut rows_processed = 0u64;
                let mut next = report
                    .with_tier_transition("consolidations_seen", legacy.consolidations_seen)
                    .with_tier_transition("duration_ms", legacy.last_run_duration_ms);
                for (key, count) in &legacy.events_demoted_per_tier {
                    rows_processed += count;
                    next = next.with_tier_transition(&format!("demoted:{key}"), *count);
                }
                Ok(next.finish_success(ctx.now, rows_processed, 0))
            }
            Err(err) => Ok(report.finish_failed(ctx.now, 0, 0, err.to_string())),
        }
    }

    fn report_view(&self, report: &SweepReport) -> SweepReportView {
        report.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::vectorizer_client::MemoryVectorizerClient;
    use crate::pruner::engine::ConsolidationDoc;
    use crate::pruner::meili_sink::{MeiliPruneError, MeiliPruneOps};
    use crate::sweep::SweepStatus;
    use async_trait::async_trait;
    use chrono::Utc;
    use cortex_storage::MetadataStore;
    use serde_json::Value;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::Mutex;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-19T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_ctx(now: DateTime<Utc>) -> SweepCtx {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        SweepCtx::new(handle, "cortex.sweep.consolidation_prune").with_now(now)
    }

    /// In-memory `MeiliPruneOps` recorder — captures every partial
    /// update + hard delete so assertions can verify the engine
    /// walked the expected indexes. Mirrors the pattern already
    /// used in `pruner::engine` and `pruner::meili_sink` tests.
    #[derive(Default)]
    struct RecordingMeili {
        updates: StdMutex<Vec<(String, Vec<Value>)>>,
        deletes: StdMutex<Vec<(String, Vec<String>)>>,
    }

    #[async_trait]
    impl MeiliPruneOps for RecordingMeili {
        async fn update_documents(
            &self,
            index: &str,
            docs: &[Value],
        ) -> Result<(), MeiliPruneError> {
            self.updates
                .lock()
                .expect("updates lock")
                .push((index.to_string(), docs.to_vec()));
            Ok(())
        }
        async fn delete_documents(
            &self,
            index: &str,
            ids: &[String],
        ) -> Result<(), MeiliPruneError> {
            self.deletes
                .lock()
                .expect("deletes lock")
                .push((index.to_string(), ids.to_vec()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn consolidation_prune_empty_provider_yields_zero_rows() {
        let provider = Arc::new(StaticConsolidationDocs::new(Vec::new()));
        let sweep = ConsolidationPruneSweep::new(
            provider,
            Arc::new(MemoryVectorizerClient::default()),
            Arc::new(RecordingMeili::default()),
            "cortex_consolidations",
        );
        let ctx = make_ctx(fixed_now());
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.name, CONSOLIDATION_PRUNE_NAME);
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(report.rows_processed, 0);
        assert_eq!(*report.tier_transitions.get("consolidations_seen").unwrap(), 0);
    }

    #[tokio::test]
    async fn consolidation_prune_with_docs_emits_consolidations_seen() {
        let now = fixed_now();
        let docs = vec![
            ConsolidationDoc {
                event_id: "01HOT".into(),
                occurred_at: now - chrono::Duration::days(1),
                vector_ids: Vec::new(),
            },
            ConsolidationDoc {
                event_id: "01WARM".into(),
                occurred_at: now - chrono::Duration::days(10),
                vector_ids: Vec::new(),
            },
        ];
        let provider = Arc::new(StaticConsolidationDocs::new(docs));
        let sweep = ConsolidationPruneSweep::new(
            provider,
            Arc::new(MemoryVectorizerClient::default()),
            Arc::new(RecordingMeili::default()),
            "cortex_consolidations",
        );
        let ctx = make_ctx(now);
        let report = sweep.run(&ctx).await.unwrap();
        assert_eq!(report.status, SweepStatus::Success);
        assert_eq!(*report.tier_transitions.get("consolidations_seen").unwrap(), 2);
    }

    #[test]
    fn consolidation_prune_schedule_parses_and_name_is_canonical() {
        let provider = Arc::new(StaticConsolidationDocs::new(Vec::new()));
        let sweep = ConsolidationPruneSweep::new(
            provider,
            Arc::new(MemoryVectorizerClient::default()),
            Arc::new(RecordingMeili::default()),
            "cortex_consolidations",
        );
        let _ = sweep.schedule();
        assert_eq!(sweep.name(), "consolidation_prune");
    }
}
