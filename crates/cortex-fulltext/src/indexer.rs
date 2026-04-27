//! `FulltextIndexer` trait and the Meili-backed default implementation.
//!
//! Mirrors the spec-08 §Inputs/Outputs contract:
//!
//! ```ignore
//! #[async_trait]
//! pub trait FulltextIndexer {
//!     async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport>;
//! }
//! ```
//!
//! The Meili-backed impl groups events by their target index, calls
//! the per-kind builder for each one, and posts each group to Meili
//! through [`MeiliClient::upsert_documents`]. The `await_task` flag
//! controls whether the indexer waits on the Meili task to complete
//! (bootstrap fail-fast) or fires-and-forgets (live traffic).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::builders::{build_doc, BuildOutcome};
use crate::config::FulltextConfig;
use crate::document::Document;
use crate::meili_client::{MeiliClient, MeiliError};
use crate::metrics::Metrics;
use crate::routing::index_for_event;
use crate::EnrichedEvent;

/// Per-batch report returned by [`FulltextIndexer::index_batch`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexReport {
    /// Total documents handed to Meilisearch this batch.
    pub documents_upserted: u32,
    /// Documents the builder declined (empty body and no summary).
    pub documents_skipped: u32,
    /// Documents whose body was truncated to honour the size cap.
    pub documents_truncated: u32,
    /// Per-index counts.
    pub by_index: BTreeMap<String, u32>,
    /// Wall-clock latency of the batch in milliseconds.
    pub latency_ms: u32,
}

/// Indexer trait — exactly the signature in spec 08 §Inputs/Outputs.
#[async_trait]
pub trait FulltextIndexer: Send + Sync {
    /// Translate `events` into Meilisearch documents and upsert them.
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError>;
}

/// Meili-backed [`FulltextIndexer`].
#[derive(Clone)]
pub struct MeiliFulltextIndexer {
    config: FulltextConfig,
    client: Arc<dyn MeiliClient>,
    metrics: Arc<Metrics>,
}

impl MeiliFulltextIndexer {
    /// Construct a new indexer.
    pub fn new(
        config: FulltextConfig,
        client: Arc<dyn MeiliClient>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            config,
            client,
            metrics,
        }
    }

    /// Borrow the runtime configuration.
    pub fn config(&self) -> &FulltextConfig {
        &self.config
    }

    /// Borrow the metrics registry.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }
}

#[async_trait]
impl FulltextIndexer for MeiliFulltextIndexer {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        let start = Instant::now();
        let mut by_index: BTreeMap<String, Vec<Document>> = BTreeMap::new();
        let mut skipped: u32 = 0;
        let mut truncated: u32 = 0;

        // 1. Build per-event documents and group by target index.
        for event in events {
            let outcome = build_doc(event, /* bootstrap */ false, self.config.max_body_bytes);
            match outcome {
                BuildOutcome::Skipped => {
                    skipped = skipped.saturating_add(1);
                    self.metrics.incr_skipped_empty();
                }
                BuildOutcome::Ready(doc) => {
                    if doc.truncated {
                        truncated = truncated.saturating_add(1);
                        self.metrics.incr_truncated();
                    }
                    let index_name = index_for_event(&self.config.index_prefix, event);
                    self.metrics.incr_routed(&index_name);
                    by_index.entry(index_name).or_default().push(*doc);
                }
            }
        }

        // 2. Per-index upsert with the configured batch size.
        let mut total_upserted: u32 = 0;
        let mut by_index_counts: BTreeMap<String, u32> = BTreeMap::new();
        let batch_size = self.config.upsert_batch.max(1);
        for (index, docs) in by_index {
            let mut count_for_index: u32 = 0;
            for chunk in docs.chunks(batch_size) {
                let upsert_started = Instant::now();
                let report = self.client.upsert_documents(&index, chunk).await?;
                let latency_ms = u32::try_from(upsert_started.elapsed().as_millis())
                    .unwrap_or(u32::MAX);
                self.metrics.observe_upsert_latency(&index, latency_ms);
                self.metrics
                    .observe_batch_size(report.documents_upserted);
                self.metrics
                    .incr_documents(&index, u64::from(report.documents_upserted));
                count_for_index = count_for_index.saturating_add(report.documents_upserted);
                total_upserted = total_upserted.saturating_add(report.documents_upserted);

                if self.config.await_task {
                    self.client
                        .wait_task(report.task_uid, Duration::from_secs(60))
                        .await?;
                }
            }
            by_index_counts.insert(index, count_for_index);
        }

        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        Ok(IndexReport {
            documents_upserted: total_upserted,
            documents_skipped: skipped,
            documents_truncated: truncated,
            by_index: by_index_counts,
            latency_ms,
        })
    }
}
