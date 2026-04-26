//! Graph-writer worker.
//!
//! Consumes `cortex.events.enriched` from Synap, runs every event
//! through the [`crate::GraphWriter`], and republishes the per-batch
//! [`crate::GraphWriteReport`] on `cortex.events.graphed`. The full
//! Synap consumer + publisher wiring (mirroring
//! [`cortex_embedder::worker`]) lands alongside the integration-test
//! suite in the next round; this skeleton runs an idle wait loop so the
//! `cortex-graph-worker` binary can already be brought up against a
//! live Nexus to validate transport + schema bootstrap.
//!
//! The shape of [`Worker`] is identical to [`cortex_embedder::Worker`]
//! intentionally: same `run_pool` shutdown contract, same `Arc`-shared
//! collaborators. Once the consumer trait is plumbed in, only this
//! module needs to change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::config::GraphConfig;
use crate::metrics::Metrics;
use crate::writer::GraphWriter;

/// Default stream name for enriched events the writer consumes.
pub const STREAM_ENRICHED: &str = "cortex.events.enriched";
/// Stream name for per-batch graph-write reports.
pub const STREAM_GRAPHED: &str = "cortex.events.graphed";
/// Stream name for events that could not be written to the graph.
pub const STREAM_INVALID: &str = "cortex.events.invalid";

/// Graph-writer worker.
pub struct Worker {
    config: GraphConfig,
    writer: Arc<dyn GraphWriter>,
    metrics: Arc<Metrics>,
}

impl Worker {
    /// Construct a worker bound to a [`GraphWriter`] and metrics
    /// registry.
    pub fn new(
        config: GraphConfig,
        writer: Arc<dyn GraphWriter>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            config,
            writer,
            metrics,
        }
    }

    /// Borrow the runtime configuration.
    pub fn config(&self) -> &GraphConfig {
        &self.config
    }

    /// Borrow the metrics registry.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Run the worker pool until `shutdown` flips to `true`.
    ///
    /// The current loop polls the shutdown flag at `flush_ms`
    /// intervals; when the Synap consumer + publisher land in the
    /// next round each iteration will pull a batch, hand it to
    /// [`GraphWriter::write_batch`], and republish the report.
    pub async fn run_pool(self: Arc<Self>, shutdown: Arc<AtomicBool>) -> Result<()> {
        let flush = Duration::from_millis(self.config.flush_ms.max(1));
        tracing::info!(
            workers = self.config.workers,
            patch_batch = self.config.patch_batch,
            flush_ms = self.config.flush_ms,
            "cortex-graph worker pool starting (idle skeleton)"
        );

        while !shutdown.load(Ordering::Relaxed) {
            // Touch the writer + metrics so the fields are always
            // exercised — keeps the live build honest about its
            // collaborators even before the consume loop lands.
            let _ = Arc::clone(&self.writer);
            let _ = Arc::clone(&self.metrics);
            tokio::time::sleep(flush).await;
        }

        tracing::info!("cortex-graph worker pool exiting");
        Ok(())
    }
}
