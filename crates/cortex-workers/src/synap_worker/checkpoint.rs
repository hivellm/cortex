//! Phase14h §1.4 — cursor checkpointing via `producer_checkpoints`.
//!
//! Re-uses the Phase A primitive from `cortex_storage::MetadataStore`.
//!
//! Workers call [`CursorCheckpoint::record`] after every
//! successful ack to persist the latest offset; on resume they
//! call [`CursorCheckpoint::resume_offset`] to seed the local
//! offset tracker so a kill-resume cycle does not rewind to
//! offset `0`.
//!
//! The checkpoint slot is keyed by `(producer_name, scope)`:
//!
//! - `producer_name` = `synap_consumer:{worker_name}` so the
//!   namespace never collides with the upstream producers that
//!   already own slots in the table.
//! - `scope` = the Synap room/stream name.
//!
//! `last_event_id` carries the offset as a stringified `u64`;
//! `last_occurred_at` = `accumulated_at` (the runtime does not
//! observe the original event's timestamp at this layer).

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use cortex_storage::{MetadataError, MetadataStore};

/// Wrapper around `MetadataStore::record_producer_checkpoint`
/// that locks the shared store + builds the conventional
/// `synap_consumer:{worker}` namespace.
pub struct CursorCheckpoint {
    worker: String,
    store: Arc<Mutex<MetadataStore>>,
}

impl CursorCheckpoint {
    /// Build a new handle for `worker_name`.
    pub fn new(worker_name: impl Into<String>, store: Arc<Mutex<MetadataStore>>) -> Self {
        Self {
            worker: worker_name.into(),
            store,
        }
    }

    /// Resolve the namespaced producer label
    /// (`synap_consumer:{worker}`).
    pub fn producer_label(&self) -> String {
        format!("synap_consumer:{}", self.worker)
    }

    /// Persist `offset` for `room`. The runtime calls this after
    /// every successful ack. Returns the recorded
    /// `accumulated_at` timestamp so callers can log it.
    pub fn record(&self, room: &str, offset: u64) -> Result<DateTime<Utc>, MetadataError> {
        let now = Utc::now();
        let producer = self.producer_label();
        let guard = self.store.lock().map_err(|e| {
            MetadataError::Internal(format!("cursor checkpoint mutex poisoned: {e}"))
        })?;
        guard.record_producer_checkpoint(&producer, room, &offset.to_string(), now, now)?;
        Ok(now)
    }

    /// Look up the last persisted offset for `room`. Returns
    /// `Ok(None)` when no checkpoint has been written yet
    /// (fresh worker boot). Returns `Ok(Some(n))` so callers
    /// can resume at `n + 1`.
    pub fn resume_offset(&self, room: &str) -> Result<Option<u64>, MetadataError> {
        let producer = self.producer_label();
        let guard = self.store.lock().map_err(|e| {
            MetadataError::Internal(format!("cursor checkpoint mutex poisoned: {e}"))
        })?;
        let row = guard.latest_producer_checkpoint(&producer, room)?;
        Ok(row.and_then(|r| r.last_event_id.parse::<u64>().ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_store() -> (TempDir, Arc<Mutex<MetadataStore>>) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("metadata.db");
        let store = MetadataStore::open(&path).unwrap();
        (dir, Arc::new(Mutex::new(store)))
    }

    #[test]
    fn resume_offset_is_none_for_fresh_store() {
        let (_dir, store) = fresh_store();
        let ckpt = CursorCheckpoint::new("embedder", store);
        let resume = ckpt.resume_offset("cortex.events.enriched").unwrap();
        assert_eq!(resume, None);
    }

    #[test]
    fn record_then_resume_round_trips_the_offset() {
        let (_dir, store) = fresh_store();
        let ckpt = CursorCheckpoint::new("fulltext", store);
        ckpt.record("cortex.events.enriched", 1234).unwrap();
        let resume = ckpt.resume_offset("cortex.events.enriched").unwrap();
        assert_eq!(resume, Some(1234));
    }

    #[test]
    fn newer_offset_supersedes_older_within_same_scope() {
        let (_dir, store) = fresh_store();
        let ckpt = CursorCheckpoint::new("graph", store);
        ckpt.record("cortex.events.enriched", 10).unwrap();
        ckpt.record("cortex.events.enriched", 99).unwrap();
        assert_eq!(
            ckpt.resume_offset("cortex.events.enriched").unwrap(),
            Some(99)
        );
    }

    #[test]
    fn scope_discriminates_per_room() {
        let (_dir, store) = fresh_store();
        let ckpt = CursorCheckpoint::new("classifier", store);
        ckpt.record("cortex.events.raw", 7).unwrap();
        ckpt.record("cortex.events.bootstrap", 42).unwrap();
        assert_eq!(ckpt.resume_offset("cortex.events.raw").unwrap(), Some(7));
        assert_eq!(
            ckpt.resume_offset("cortex.events.bootstrap").unwrap(),
            Some(42)
        );
    }

    #[test]
    fn producer_label_is_namespaced_per_worker() {
        let (_dir, store) = fresh_store();
        let ckpt_e = CursorCheckpoint::new("embedder", store.clone());
        let ckpt_f = CursorCheckpoint::new("fulltext", store);
        assert_eq!(ckpt_e.producer_label(), "synap_consumer:embedder");
        assert_eq!(ckpt_f.producer_label(), "synap_consumer:fulltext");
        ckpt_e.record("cortex.events.enriched", 5).unwrap();
        assert_eq!(
            ckpt_f.resume_offset("cortex.events.enriched").unwrap(),
            None
        );
        assert_eq!(
            ckpt_e.resume_offset("cortex.events.enriched").unwrap(),
            Some(5)
        );
    }
}
