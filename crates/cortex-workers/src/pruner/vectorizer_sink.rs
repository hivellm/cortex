//! Phase11o §2.2 — Vectorizer demotion sink.
//!
//! Calls
//! [`crate::embedder::vectorizer_client::VectorizerClient::move_vectors`]
//! once per `(src, dst)` pair; chunks each batch at
//! [`super::MOVE_BATCH_SIZE`] ids so a single transient 5xx never
//! drops a whole day of demotions.

use crate::embedder::vectorizer_client::{VectorizerClient, VectorizerClientError};
use vectorizer_sdk::models::MoveReport;

use super::{tier_pair_key, DemotionAction, PruneReport, MOVE_BATCH_SIZE};

/// Run every vector-side demotion in `actions` against `client`.
/// Skips actions whose `to` tier is `Expired` (those are routed
/// through the purge sink, not a move).
///
/// Returns a fresh [`PruneReport`] populated with the per-tier
/// counts; the caller merges this into the outer run summary so
/// the cron-side report can layer Meili + purge counters on top.
pub async fn demote(
    client: &dyn VectorizerClient,
    actions: &[DemotionAction],
) -> Result<PruneReport, VectorizerClientError> {
    let mut report = PruneReport {
        consolidations_seen: actions.len() as u64,
        ..PruneReport::default()
    };

    for action in actions {
        // Expired actions are not moved — they hard-purge through
        // the purge sink. Counted upstream by the caller.
        if action.dst_collection().is_none() {
            continue;
        }
        let Some(src) = action.src_collection() else {
            // Source already expired — nothing to read from.
            continue;
        };
        let Some(dst) = action.dst_collection() else {
            continue;
        };
        if action.vector_ids.is_empty() {
            continue;
        }
        for chunk in action.vector_ids.chunks(MOVE_BATCH_SIZE) {
            let ids: Vec<String> = chunk.to_vec();
            let move_report: MoveReport = client.move_vectors(src, dst, &ids).await?;
            let key = tier_pair_key(action.from, action.to);
            *report.events_demoted_per_tier.entry(key).or_insert(0) += move_report.moved as u64;
            report.events_failed += move_report.failed as u64;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::vectorizer_client::MemoryVectorizerClient;
    use crate::pruner::PruneTier;

    #[tokio::test]
    async fn demotes_warm_to_cold_via_move_vectors() {
        let client = MemoryVectorizerClient::default();
        // Pre-populate the warm collection with two ids so the
        // memory client can move them. We inject directly into the
        // store so the test doesn't need to construct a full
        // `Chunk` (which carries classifier-side metadata the
        // pruner never touches).
        {
            let mut stored = client.dedup_keys_per_collection.lock().unwrap();
            let map = stored
                .entry(cortex_storage::names::COLLECTION_CONSOLIDATION_PQ.to_string())
                .or_default();
            map.insert(
                "k1".to_string(),
                crate::embedder::vectorizer_client::StoredVec {
                    dedup_key: "k1".to_string(),
                    server_id: "srv-k1".to_string(),
                },
            );
            map.insert(
                "k2".to_string(),
                crate::embedder::vectorizer_client::StoredVec {
                    dedup_key: "k2".to_string(),
                    server_id: "srv-k2".to_string(),
                },
            );
        }

        let action = DemotionAction {
            consolidation_id: "c1".into(),
            from: PruneTier::Warm,
            to: PruneTier::Cold,
            vector_ids: vec!["k1".into(), "k2".into()],
        };
        let report = demote(&client, std::slice::from_ref(&action))
            .await
            .unwrap();
        assert_eq!(
            report
                .events_demoted_per_tier
                .get("warm->cold")
                .copied()
                .unwrap_or(0),
            2
        );
        assert_eq!(report.events_failed, 0);
        assert_eq!(
            client.stored_count_for(cortex_storage::names::COLLECTION_CONSOLIDATION_PQ),
            0
        );
        assert_eq!(
            client.stored_count_for(cortex_storage::names::COLLECTION_COLD_BINARY),
            2
        );
    }

    #[tokio::test]
    async fn missing_ids_count_as_failures_without_aborting() {
        let client = MemoryVectorizerClient::default();
        // No upsert — all ids will miss in src.
        let action = DemotionAction {
            consolidation_id: "c1".into(),
            from: PruneTier::Hot,
            to: PruneTier::Warm,
            vector_ids: vec!["missing-1".into(), "missing-2".into()],
        };
        let report = demote(&client, std::slice::from_ref(&action))
            .await
            .unwrap();
        assert_eq!(report.events_failed, 2);
        assert!(
            report
                .events_demoted_per_tier
                .get("hot->warm")
                .copied()
                .unwrap_or(0)
                == 0
        );
    }

    #[tokio::test]
    async fn expired_actions_are_skipped() {
        let client = MemoryVectorizerClient::default();
        let action = DemotionAction {
            consolidation_id: "c1".into(),
            from: PruneTier::Cold,
            to: PruneTier::Expired,
            vector_ids: vec!["k1".into()],
        };
        let report = demote(&client, std::slice::from_ref(&action))
            .await
            .unwrap();
        // No move call should have been issued.
        assert!(report.events_demoted_per_tier.is_empty());
    }
}
