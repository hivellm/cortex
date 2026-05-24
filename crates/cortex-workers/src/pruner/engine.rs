//! Phase11o §2.5 — sweep engine.
//!
//! Glues the three sinks ([`super::vectorizer_sink`],
//! [`super::meili_sink`], [`super::purge`]) into one nightly run.
//! The cron-side bin (`cortex-ops consolidation-prune`) loads
//! consolidations from Meili, hands them off to [`run_sweep`], and
//! prints / logs the [`super::PruneReport`].

use chrono::{DateTime, Utc};

use crate::embedder::vectorizer_client::VectorizerClient;

use super::meili_sink::{MeiliPruneError, MeiliPruneOps};
use super::{plan_demotion, tier_pair_key, vectorizer_sink, DemotionAction, PruneReport};

/// One row the engine consumes — derived from a Meili
/// `cortex_consolidations` document. The caller owns the
/// Meili-side decoding so the engine stays decoupled from the
/// transport (HTTP `GET /indexes/{uid}/documents` vs. an
/// in-memory test fixture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationDoc {
    /// Primary key in `cortex_consolidations`.
    pub event_id: String,
    /// When the consolidation was minted; bucketed into a
    /// [`super::PruneTier`] via [`super::PruneTier::from_age_days`].
    pub occurred_at: DateTime<Utc>,
    /// Vector ids (stable Vectorizer primary keys) referenced by
    /// this consolidation's `source_event_ids`. The engine moves
    /// these between collections; `delete_vectors` on the purge
    /// path uses the same list.
    pub vector_ids: Vec<String>,
}

/// Errors surfaced by [`run_sweep`].
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// Vector-side failure (transport / SDK / non-retriable 4xx).
    #[error("vectorizer: {0}")]
    Vectorizer(#[from] crate::embedder::vectorizer_client::VectorizerClientError),
    /// Meili-side failure.
    #[error("meili: {0}")]
    Meili(#[from] MeiliPruneError),
}

/// Run one sweep. Returns the merged [`PruneReport`] across the
/// vectorizer + meili legs (the purge leg is owned by the
/// `/cortex forget` MCP path, not the nightly cron). Wall-clock
/// duration is captured in `report.last_run_duration_ms`.
pub async fn run_sweep(
    docs: &[ConsolidationDoc],
    now: DateTime<Utc>,
    vectorizer: &dyn VectorizerClient,
    meili: &dyn MeiliPruneOps,
    meili_index: &str,
) -> Result<PruneReport, SweepError> {
    let started = std::time::Instant::now();

    // Plan every demotion up front so the two sinks see the same
    // action list and the report keys line up.
    let actions: Vec<DemotionAction> = docs
        .iter()
        .filter_map(|d| plan_demotion(&d.event_id, d.occurred_at, now, d.vector_ids.clone()))
        .collect();

    // 1. Vectorizer leg: warm + cold transitions.
    let mut report = vectorizer_sink::demote(vectorizer, &actions).await?;
    report.consolidations_seen = docs.len() as u64;

    // 2. Meili leg: cold-tier field stripping. Updates the same
    //    rows whose vectors moved to `cortex.cold.binary`.
    let cold_touched = super::meili_sink::demote(meili, meili_index, &actions).await?;
    if cold_touched > 0 {
        // The cold leg is recorded in the same `events_demoted_per_tier`
        // map under a "warm->cold:meili" sub-key so the health JSON
        // can show vector + meili work side by side.
        *report
            .events_demoted_per_tier
            .entry(format!(
                "{}:meili",
                tier_pair_key(super::PruneTier::Warm, super::PruneTier::Cold)
            ))
            .or_insert(0) += cold_touched;
    }

    // 3. Expired tier: hard-purge through the meili sink. Vector
    //    deletion happens via `vectorizer_sink` already (skipped
    //    for `Expired`) so we only drop the meili row + count.
    let expired_ids: Vec<String> = actions
        .iter()
        .filter(|a| a.to == super::PruneTier::Expired)
        .map(|a| a.consolidation_id.clone())
        .collect();
    if !expired_ids.is_empty() {
        super::meili_sink::purge(meili, meili_index, &expired_ids).await?;
        report.events_purged = expired_ids.len() as u64;
    }

    report.last_run_duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::vectorizer_client::MemoryVectorizerClient;
    use crate::pruner::meili_sink::MeiliPruneError;
    use async_trait::async_trait;
    use chrono::TimeZone;
    use serde_json::Value;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingMeili {
        updates: Mutex<Vec<(String, Vec<Value>)>>,
        deletes: Mutex<Vec<(String, Vec<String>)>>,
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
                .unwrap()
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
                .unwrap()
                .push((index.to_string(), ids.to_vec()));
            Ok(())
        }
    }

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn sweep_executes_warm_cold_and_expired_legs() {
        let vec_client = MemoryVectorizerClient::default();
        // Pre-populate the warm + cold collections with one vector
        // each so the engine can move them.
        {
            let mut stored = vec_client.dedup_keys_per_collection.lock().unwrap();
            stored
                .entry(cortex_storage::names::COLLECTION_CONSOLIDATION_FP32.into())
                .or_default()
                .insert("v-warm".into(), "srv-warm".into());
            stored
                .entry(cortex_storage::names::COLLECTION_CONSOLIDATION_PQ.into())
                .or_default()
                .insert("v-cold".into(), "srv-cold".into());
        }
        let meili = RecordingMeili::default();
        let now = ts(2026, 5, 4);
        let docs = vec![
            // Warm tier (10 d): hot→warm move.
            ConsolidationDoc {
                event_id: "c-warm".into(),
                occurred_at: ts(2026, 4, 24),
                vector_ids: vec!["v-warm".into()],
            },
            // Cold tier (120 d): warm→cold move + meili strip.
            ConsolidationDoc {
                event_id: "c-cold".into(),
                occurred_at: ts(2026, 1, 4),
                vector_ids: vec!["v-cold".into()],
            },
            // Expired (500 d): cold→expired purge through meili.
            ConsolidationDoc {
                event_id: "c-expired".into(),
                occurred_at: ts(2024, 12, 21),
                vector_ids: vec!["v-x".into()],
            },
            // Hot (3 d): no action.
            ConsolidationDoc {
                event_id: "c-hot".into(),
                occurred_at: ts(2026, 5, 1),
                vector_ids: vec!["v-h".into()],
            },
        ];

        let report = run_sweep(
            &docs,
            now,
            &vec_client,
            &meili,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        .unwrap();

        assert_eq!(report.consolidations_seen, 4);
        assert_eq!(
            report
                .events_demoted_per_tier
                .get("hot->warm")
                .copied()
                .unwrap_or(0),
            1
        );
        assert_eq!(
            report
                .events_demoted_per_tier
                .get("warm->cold")
                .copied()
                .unwrap_or(0),
            1
        );
        assert_eq!(
            report
                .events_demoted_per_tier
                .get("warm->cold:meili")
                .copied()
                .unwrap_or(0),
            1
        );
        assert_eq!(report.events_purged, 1);
        // The `Cold→Expired` action does not call `move_vectors`
        // (the dst is `None`), so no per-id failure is counted.
        // The expired path is purely the meili-side `purge` call
        // already asserted below.
        assert_eq!(report.events_failed, 0);

        let updates = meili.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.len(), 1);
        let deletes = meili.deletes.lock().unwrap();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].1, vec!["c-expired".to_string()]);
    }

    #[tokio::test]
    async fn empty_doc_list_is_a_clean_noop() {
        let vec_client = MemoryVectorizerClient::default();
        let meili = RecordingMeili::default();
        let report = run_sweep(
            &[],
            ts(2026, 5, 4),
            &vec_client,
            &meili,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        .unwrap();
        assert_eq!(report.consolidations_seen, 0);
        assert_eq!(report.events_purged, 0);
        assert!(report.events_demoted_per_tier.is_empty());
        assert!(meili.updates.lock().unwrap().is_empty());
        assert!(meili.deletes.lock().unwrap().is_empty());
    }
}
