//! High-level [`GraphWriter`] trait + [`NexusGraphWriter`] implementation.
//!
//! Mirrors the trait signature in `docs/specs/07-graph-writer.md`
//! §Inputs/Outputs. The writer orchestrates the per-batch flow:
//!
//! 1. Map every event to a [`GraphPatch`] via [`super::map_event_to_patch`].
//! 2. Coalesce all patches into one through [`super::coalescer::coalesce`].
//! 3. Hand the coalesced patch to the [`GraphClient`] inside one
//!    Cypher transaction.
//! 4. Emit metrics + return the [`GraphWriteReport`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use super::coalescer::coalesce;
use super::config::GraphConfig;
use super::cypher::CypherTemplates;
use super::mapper::map_event_to_patch;
use super::metrics::Metrics;
use super::nexus_client::{GraphClient, GraphClientError};
use super::patch::GraphWriteReport;
use crate::embedder::EnrichedEvent;

/// Writer trait — exactly the signature in spec 07 §Inputs/Outputs.
#[async_trait]
pub trait GraphWriter: Send + Sync {
    /// Translate `events` into graph upserts and write them to Nexus
    /// inside one Cypher transaction. Returns a per-batch report
    /// suitable for surfacing as a metrics span.
    async fn write_batch(
        &self,
        events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError>;

    /// Write a list of pre-computed patches in one transaction. Used by
    /// the worker's out-of-order recovery path to inject synthetic
    /// orphan-Turn nodes alongside the regular per-event patches.
    /// Default implementations may delegate to `write_batch` after
    /// reconstructing events; concrete implementations override to
    /// avoid the round-trip.
    async fn write_patches(
        &self,
        patches: Vec<super::patch::GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError>;
}

/// Production [`GraphWriter`] backed by a [`GraphClient`] +
/// [`CypherTemplates`] registry.
#[derive(Clone)]
pub struct NexusGraphWriter {
    client: Arc<dyn GraphClient>,
    templates: Arc<CypherTemplates>,
    metrics: Arc<Metrics>,
    config: GraphConfig,
}

impl NexusGraphWriter {
    /// Construct a writer wrapping `client` with the given templates,
    /// metrics registry, and runtime configuration.
    pub fn new(
        config: GraphConfig,
        client: Arc<dyn GraphClient>,
        templates: Arc<CypherTemplates>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            client,
            templates,
            metrics,
            config,
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
}

#[async_trait]
impl GraphWriter for NexusGraphWriter {
    async fn write_batch(
        &self,
        events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        let patches: Vec<_> = events.iter().map(map_event_to_patch).collect();
        self.write_patches(patches).await
    }

    async fn write_patches(
        &self,
        patches: Vec<super::patch::GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        let start = Instant::now();
        let (patch, stats) = coalesce(patches);

        let mut by_label: BTreeMap<String, u32> = BTreeMap::new();
        for node in &patch.nodes {
            *by_label.entry(node.label.clone()).or_insert(0) += 1;
            self.metrics.incr_nodes_upserted(&node.label, 1);
        }
        // Tally each edge's edge_type *before* the write so we can
        // back out the persisted count per type from `write_stats.
        // edges_dropped`. The per-type metric must reflect what
        // actually landed in Nexus, not what the patch attempted, or
        // the dashboard returns to the same lying-success class
        // phase1 + phase2 spent the day fixing.
        let mut attempted_by_type: BTreeMap<String, u32> = BTreeMap::new();
        for edge in &patch.edges {
            *attempted_by_type.entry(edge.edge_type.clone()).or_insert(0) += 1;
        }
        self.metrics
            .incr_dedup_hits_nodes(u64::from(stats.node_dedup_hits));
        self.metrics
            .incr_dedup_hits_edges(u64::from(stats.edge_dedup_hits));

        let write_stats = self
            .client
            .run_write_tx(&patch, self.templates.as_ref())
            .await?;

        // Reconcile attempted vs persisted per edge_type. Increment
        // `edges_upserted` for the persisted slice and
        // `edges_dropped` for the silently-lost slice; emit one WARN
        // per edge_type that had any drops so a recurrence is loud
        // in the worker log.
        for (edge_type, attempted) in &attempted_by_type {
            let dropped = write_stats
                .edges_dropped
                .get(edge_type)
                .copied()
                .unwrap_or(0);
            let persisted = attempted.saturating_sub(dropped);
            if persisted > 0 {
                self.metrics
                    .incr_edges_upserted(edge_type, u64::from(persisted));
            }
            if dropped > 0 {
                for _ in 0..dropped {
                    self.metrics.incr_edges_dropped(edge_type);
                }
                tracing::warn!(
                    edge_type = %edge_type,
                    attempted = attempted,
                    persisted = persisted,
                    dropped = dropped,
                    "Nexus dropped {dropped} of {attempted} {edge_type} edges in this batch"
                );
            }
        }

        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        self.metrics.observe_tx_latency(latency_ms);
        self.metrics
            .observe_tx_size(write_stats.nodes_upserted + write_stats.edges_upserted);

        Ok(GraphWriteReport {
            nodes_upserted: write_stats.nodes_upserted,
            edges_upserted: write_stats.edges_upserted,
            nodes_deduped: stats.node_dedup_hits,
            edges_deduped: stats.edge_dedup_hits,
            by_label,
            latency_ms,
        })
    }
}
