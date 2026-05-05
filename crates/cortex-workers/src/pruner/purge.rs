//! Phase11o §2.4 — Hard-purge sink.
//!
//! Cascades a single `event_id` removal across every backend that
//! could carry the row:
//!
//! - **Vectorizer** — calls
//!   [`crate::embedder::vectorizer_client::VectorizerClient::delete_vectors`]
//!   on every collection where the event might live (typically
//!   `cortex.consolidation.{fp32,pq}` + `cortex.cold.binary` for an
//!   expired consolidation).
//! - **Meili** — calls [`super::meili_sink::purge`] to drop the row
//!   from `cortex_consolidations`.
//! - **Nexus** — runs `DELETE` on the matching node by `event_id`
//!   via the supplied [`NexusPurgeOps`] handle.
//! - **Parquet archive** — rewrites the partition without the row;
//!   handled outside this sink by the file rotator (the
//!   sink calls back through the [`ArchivePurgeOps`] trait).
//!
//! The sink is structured as a fan-out across the four backends so a
//! single failing dependency surfaces a precise error rather than
//! silently partial-purging. Each backend is reached through a
//! narrow trait so tests can drive the cascade without the live
//! services.

use async_trait::async_trait;

use crate::embedder::vectorizer_client::{VectorizerClient, VectorizerClientError};

use super::meili_sink::{MeiliPruneError, MeiliPruneOps};

/// Minimal trait the purge sink needs from Nexus.
#[async_trait]
pub trait NexusPurgeOps: Send + Sync {
    /// Delete the node identified by `event_id`. Idempotent — a
    /// missing node must NOT surface as an error so re-runs of the
    /// purge stay clean.
    async fn delete_node_by_event_id(&self, event_id: &str) -> Result<(), NexusPurgeError>;
}

/// Errors surfaced by [`NexusPurgeOps`].
#[derive(Debug, thiserror::Error)]
pub enum NexusPurgeError {
    /// HTTP / transport-level failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Server returned a non-success status.
    #[error("server: {0}")]
    Server(String),
}

/// Minimal trait the purge sink needs from the Parquet archive
/// rotator. Implementations rewrite the partition without the row.
#[async_trait]
pub trait ArchivePurgeOps: Send + Sync {
    /// Drop every line referencing `event_id` from the archive's
    /// hour-partitioned files. Idempotent.
    async fn drop_event(&self, event_id: &str) -> Result<(), ArchivePurgeError>;
}

/// Errors surfaced by [`ArchivePurgeOps`].
#[derive(Debug, thiserror::Error)]
pub enum ArchivePurgeError {
    /// Filesystem-level failure.
    #[error("io: {0}")]
    Io(String),
    /// Decoding / re-encoding failure on a parquet partition.
    #[error("decode: {0}")]
    Decode(String),
}

/// Aggregate error returned by [`forget`]. Each variant pinpoints
/// the backend that failed so the caller can retry the cascade
/// without re-running the successful legs.
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    /// Vectorizer leg failed.
    #[error("vectorizer: {0}")]
    Vectorizer(#[from] VectorizerClientError),
    /// Meili leg failed.
    #[error("meili: {0}")]
    Meili(#[from] MeiliPruneError),
    /// Nexus leg failed.
    #[error("nexus: {0}")]
    Nexus(#[from] NexusPurgeError),
    /// Parquet archive leg failed.
    #[error("archive: {0}")]
    Archive(#[from] ArchivePurgeError),
}

/// Confirmation token the `/cortex forget` MCP tool requires. The
/// caller must echo back exactly this value or the cascade refuses
/// to start. Surfaces as the `confirmation` field on the MCP tool
/// descriptor (phase11j §5.3).
pub const REQUIRED_CONFIRMATION_TOKEN: &str = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE";

/// Outcome of a single [`forget`] cascade.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// Vectors removed across every collection probed.
    pub vectors_removed: u64,
    /// Whether the Meili row was deleted (false ⇒ the row was
    /// already absent before the call).
    pub meili_deleted: bool,
    /// Whether the Nexus node was deleted (false ⇒ already absent).
    pub nexus_deleted: bool,
    /// Whether the archive partition was rewritten (false ⇒ no
    /// matching row was found in any partition).
    pub archive_rewritten: bool,
}

/// Hard-purge `event_id` across every backend.
///
/// `vectorizer_collections` is the list of collections the caller
/// wants probed for the id (typically all three consolidation
/// collections for an expired consolidation, or just the per-kind
/// hot collection for an arbitrary `/cortex forget` call).
///
/// Returns the [`PurgeError`] of the first backend that fails. The
/// successful legs are not rolled back — re-running the cascade is
/// safe (every leg is idempotent).
#[allow(clippy::too_many_arguments)]
pub async fn forget(
    event_id: &str,
    vectorizer_collections: &[&str],
    vectorizer: &dyn VectorizerClient,
    meili: &dyn MeiliPruneOps,
    meili_index: &str,
    nexus: &dyn NexusPurgeOps,
    archive: &dyn ArchivePurgeOps,
    confirmation: &str,
) -> Result<PurgeReport, PurgeError> {
    if confirmation != REQUIRED_CONFIRMATION_TOKEN {
        return Err(PurgeError::Vectorizer(VectorizerClientError::Other(
            format!(
                "missing or wrong confirmation token (expected {REQUIRED_CONFIRMATION_TOKEN:?})"
            ),
        )));
    }

    let mut report = PurgeReport::default();
    let ids = [event_id.to_string()];

    // 1. Vectorizer — probe every collection. `delete_vectors`
    //    surfaces missing-id failures inside the report (no abort),
    //    so we add up the `deleted` count across collections.
    for collection in vectorizer_collections {
        let dr = vectorizer.delete_vectors(collection, &ids).await?;
        report.vectors_removed += dr.deleted as u64;
    }

    // 2. Meili — drop the row from the index. The `delete_documents`
    //    contract is idempotent, so we conservatively flip
    //    `meili_deleted = true` whenever the call succeeds (we
    //    don't have a per-id miss/hit signal from the server).
    super::meili_sink::purge(meili, meili_index, &[event_id.to_string()]).await?;
    report.meili_deleted = true;

    // 3. Nexus — delete the matching node. Idempotent; missing-node
    //    errors are absorbed inside the trait impl.
    nexus.delete_node_by_event_id(event_id).await?;
    report.nexus_deleted = true;

    // 4. Archive — rewrite the partition.
    archive.drop_event(event_id).await?;
    report.archive_rewritten = true;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::vectorizer_client::MemoryVectorizerClient;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingMeili {
        deletes: Mutex<Vec<(String, Vec<String>)>>,
    }

    #[async_trait]
    impl MeiliPruneOps for RecordingMeili {
        async fn update_documents(
            &self,
            _index: &str,
            _docs: &[serde_json::Value],
        ) -> Result<(), MeiliPruneError> {
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

    #[derive(Default)]
    struct RecordingNexus {
        deleted: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl NexusPurgeOps for RecordingNexus {
        async fn delete_node_by_event_id(&self, event_id: &str) -> Result<(), NexusPurgeError> {
            self.deleted.lock().unwrap().push(event_id.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingArchive {
        dropped: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ArchivePurgeOps for RecordingArchive {
        async fn drop_event(&self, event_id: &str) -> Result<(), ArchivePurgeError> {
            self.dropped.lock().unwrap().push(event_id.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn forget_requires_confirmation_token() {
        let vec = MemoryVectorizerClient::default();
        let meili = RecordingMeili::default();
        let nexus = RecordingNexus::default();
        let archive = RecordingArchive::default();

        let err = forget(
            "01HXEVT",
            &[cortex_storage::names::COLLECTION_CONSOLIDATION_FP32],
            &vec,
            &meili,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
            &nexus,
            &archive,
            "wrong-token",
        )
        .await
        .unwrap_err();

        assert!(format!("{err}").contains("confirmation"));
        assert!(meili.deletes.lock().unwrap().is_empty());
        assert!(nexus.deleted.lock().unwrap().is_empty());
        assert!(archive.dropped.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn forget_cascades_across_every_backend() {
        let vec = MemoryVectorizerClient::default();
        let meili = RecordingMeili::default();
        let nexus = RecordingNexus::default();
        let archive = RecordingArchive::default();

        let report = forget(
            "01HXEVT",
            &[
                cortex_storage::names::COLLECTION_CONSOLIDATION_FP32,
                cortex_storage::names::COLLECTION_CONSOLIDATION_PQ,
            ],
            &vec,
            &meili,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
            &nexus,
            &archive,
            REQUIRED_CONFIRMATION_TOKEN,
        )
        .await
        .unwrap();

        assert!(report.meili_deleted);
        assert!(report.nexus_deleted);
        assert!(report.archive_rewritten);

        assert_eq!(meili.deletes.lock().unwrap().len(), 1);
        assert_eq!(*nexus.deleted.lock().unwrap(), vec!["01HXEVT".to_string()]);
        assert_eq!(
            *archive.dropped.lock().unwrap(),
            vec!["01HXEVT".to_string()]
        );
    }
}
