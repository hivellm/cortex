//! Phase11o §2.3 — Meili demotion sink.
//!
//! Drops the high-cost fields (`body`, `summary`, `outcome_distribution`)
//! on cold-tier rows. The keyword lane keeps a minimal record so
//! `/v1/query` filtering still works, but the disk footprint stays
//! bounded — the dropped fields can run to several KB per
//! consolidation in the long tail.
//!
//! Hard-purge of the Meili row itself happens in
//! [`super::purge`], not here. This sink is for the cold-tier
//! transition only.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::DemotionAction;

/// Per-row update the Meili sink applies to a cold-tier
/// consolidation. The `event_id` is required (Meili primary key);
/// every other field stays as-is, while the three named fields are
/// nulled.
pub const COLD_TIER_DROPPED_FIELDS: &[&str] = &["body", "summary", "outcome_distribution"];

/// Minimal trait the pruner needs from the Meili transport. Kept
/// local to the pruner so the global [`crate::fulltext::MeiliClient`]
/// trait isn't perturbed for one feature.
#[async_trait]
pub trait MeiliPruneOps: Send + Sync {
    /// Apply a partial update to `index`. Each entry in `docs` must
    /// carry the index's primary key (`event_id` for
    /// `cortex_consolidations`). Fields the caller wants nulled
    /// must be present in the JSON object as `null`.
    async fn update_documents(&self, index: &str, docs: &[Value]) -> Result<(), MeiliPruneError>;

    /// Hard-delete documents by primary key from `index`. Idempotent;
    /// missing ids are not surfaced as errors.
    async fn delete_documents(
        &self,
        index: &str,
        ids: &[String],
    ) -> Result<(), MeiliPruneError>;
}

/// Errors surfaced by [`MeiliPruneOps`].
#[derive(Debug, thiserror::Error)]
pub enum MeiliPruneError {
    /// HTTP / transport-level failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Server returned a non-success status.
    #[error("server: {0}")]
    Server(String),
}

/// Run cold-tier field stripping for every action in `actions` whose
/// `to` tier is [`super::PruneTier::Cold`]. Other actions are
/// ignored (the vectorizer sink already handled their warm-tier
/// move). Returns the count of consolidations whose Meili rows were
/// updated.
pub async fn demote(
    client: &dyn MeiliPruneOps,
    index: &str,
    actions: &[DemotionAction],
) -> Result<u64, MeiliPruneError> {
    use super::PruneTier;
    let cold_targets: Vec<&DemotionAction> = actions
        .iter()
        .filter(|a| a.to == PruneTier::Cold)
        .collect();
    if cold_targets.is_empty() {
        return Ok(0);
    }
    let docs: Vec<Value> = cold_targets
        .iter()
        .map(|a| {
            let mut obj = Map::new();
            obj.insert("event_id".into(), Value::String(a.consolidation_id.clone()));
            for field in COLD_TIER_DROPPED_FIELDS {
                obj.insert((*field).to_string(), Value::Null);
            }
            Value::Object(obj)
        })
        .collect();
    client.update_documents(index, &docs).await?;
    Ok(cold_targets.len() as u64)
}

/// Hard-delete `event_ids` from `index`. Used by the purge sink for
/// expired consolidations and by the `/cortex forget` MCP path.
pub async fn purge(
    client: &dyn MeiliPruneOps,
    index: &str,
    event_ids: &[String],
) -> Result<(), MeiliPruneError> {
    if event_ids.is_empty() {
        return Ok(());
    }
    client.delete_documents(index, event_ids).await
}

/// Build the partial-update payload (a list of JSON objects) for a
/// set of cold-tier consolidations. Public so callers / tests can
/// assert on the wire shape without a real Meili.
pub fn cold_tier_payload(consolidation_ids: &[&str]) -> Vec<Value> {
    consolidation_ids
        .iter()
        .map(|id| {
            let mut obj = json!({ "event_id": id });
            if let Some(map) = obj.as_object_mut() {
                for field in COLD_TIER_DROPPED_FIELDS {
                    map.insert((*field).to_string(), Value::Null);
                }
            }
            obj
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pruner::PruneTier;
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

    #[test]
    fn cold_tier_payload_drops_three_fields() {
        let docs = cold_tier_payload(&["c1", "c2"]);
        assert_eq!(docs.len(), 2);
        for doc in &docs {
            let map = doc.as_object().unwrap();
            assert!(map.contains_key("event_id"));
            assert_eq!(map.get("body"), Some(&Value::Null));
            assert_eq!(map.get("summary"), Some(&Value::Null));
            assert_eq!(map.get("outcome_distribution"), Some(&Value::Null));
        }
    }

    #[tokio::test]
    async fn demote_only_touches_cold_actions() {
        let client = RecordingMeili::default();
        let actions = vec![
            DemotionAction {
                consolidation_id: "c-warm".into(),
                from: PruneTier::Hot,
                to: PruneTier::Warm,
                vector_ids: vec![],
            },
            DemotionAction {
                consolidation_id: "c-cold-1".into(),
                from: PruneTier::Warm,
                to: PruneTier::Cold,
                vector_ids: vec![],
            },
            DemotionAction {
                consolidation_id: "c-cold-2".into(),
                from: PruneTier::Warm,
                to: PruneTier::Cold,
                vector_ids: vec![],
            },
        ];
        let touched = demote(&client, "cortex_consolidations", &actions)
            .await
            .unwrap();
        assert_eq!(touched, 2);
        let updates = client.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, "cortex_consolidations");
        assert_eq!(updates[0].1.len(), 2);
    }

    #[tokio::test]
    async fn purge_routes_through_delete_documents() {
        let client = RecordingMeili::default();
        purge(&client, "cortex_consolidations", &["c-x".into(), "c-y".into()])
            .await
            .unwrap();
        let deletes = client.deletes.lock().unwrap();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].0, "cortex_consolidations");
        assert_eq!(deletes[0].1, vec!["c-x".to_string(), "c-y".into()]);
    }

    #[tokio::test]
    async fn purge_empty_id_list_is_noop() {
        let client = RecordingMeili::default();
        purge(&client, "cortex_consolidations", &[]).await.unwrap();
        assert!(client.deletes.lock().unwrap().is_empty());
    }
}
