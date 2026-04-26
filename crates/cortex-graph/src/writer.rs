//! High-level [`GraphWriter`] trait + [`NexusGraphWriter`] implementation.
//!
//! Mirrors the trait signature in `docs/specs/07-graph-writer.md`
//! §Inputs/Outputs. The writer orchestrates the per-batch flow:
//!
//! 1. Map every event to a [`GraphPatch`] via [`crate::map_event_to_patch`].
//! 2. Coalesce all patches into one through [`crate::coalescer::coalesce`].
//! 3. Hand the coalesced patch to the [`GraphClient`] inside one
//!    Cypher transaction.
//! 4. Emit metrics + return the [`GraphWriteReport`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::coalescer::coalesce;
use crate::config::GraphConfig;
use crate::cypher::CypherTemplates;
use crate::mapper::map_event_to_patch;
use crate::metrics::Metrics;
use crate::nexus_client::{GraphClient, GraphClientError};
use crate::patch::GraphWriteReport;
use crate::EnrichedEvent;

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
        let start = Instant::now();
        let patches: Vec<_> = events.iter().map(map_event_to_patch).collect();
        let (patch, stats) = coalesce(patches);

        let mut by_label: BTreeMap<String, u32> = BTreeMap::new();
        for node in &patch.nodes {
            *by_label.entry(node.label.clone()).or_insert(0) += 1;
            self.metrics.incr_nodes_upserted(&node.label, 1);
        }
        for edge in &patch.edges {
            self.metrics.incr_edges_upserted(&edge.edge_type, 1);
        }
        self.metrics
            .incr_dedup_hits_nodes(u64::from(stats.node_dedup_hits));
        self.metrics
            .incr_dedup_hits_edges(u64::from(stats.edge_dedup_hits));

        let write_stats = self
            .client
            .run_write_tx(&patch, self.templates.as_ref())
            .await?;

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
