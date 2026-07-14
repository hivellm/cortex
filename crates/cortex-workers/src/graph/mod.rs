//! Cortex graph writer (was crate `cortex-graph`).
//!
//! Consumes enriched events from Synap, maps them onto the Cortex
//! graph schema, coalesces redundant upserts, and writes them to
//! Nexus through `nexus-graph-sdk`.

#![warn(missing_docs)]

pub mod analyzer;
pub mod analyzer_dispatch;
pub mod bitemporal;
pub mod branch;
pub mod citations;
pub mod classification;
pub mod classification_migration;
pub mod coalescer;
pub mod community;
pub mod config;
pub mod cross_project;
pub mod cypher;
pub mod dedup;
pub mod doc_annotator;
pub mod doc_drift;
pub mod doc_pass;
pub mod extraction_contract;
pub mod extractors;
pub mod identity;
pub mod indexer;
pub mod mapper;
pub mod markdown;
pub mod metrics;
pub mod migration;
pub mod nexus_client;
pub mod noncode_parsers;
pub mod patch;
pub mod projection;
pub mod reindex_alias;
pub mod resolver;
pub mod schema;
pub mod scip;
pub mod stale_sweeper;
pub mod temporal_edges;
pub mod worker;
pub mod writer;

pub use crate::embedder::EnrichedEvent;
pub use coalescer::{CoalesceStats, PatchCoalescer};
pub use config::{GraphConfig, GraphTransport};
pub use cypher::{
    edge_template_name, node_template_name, CypherLoadError, CypherTemplates, REQUIRED_TEMPLATES,
};
pub use identity::artifact_natural_key;
pub use mapper::map_event_to_patch;
pub use metrics::Metrics;
pub use nexus_client::{
    GraphClient, GraphClientError, LiveNexusClient, MemoryCall, MemoryNexusClient, WriteStats,
};
pub use patch::{EdgeConfidence, EdgeOp, GraphPatch, GraphWriteReport, NodeOp};
pub use schema::SCHEMA_STATEMENTS;
pub use stale_sweeper::{StaleEdgeSweeper, SweepReport, DEFAULT_SWEEP_INTERVAL_SECS};
pub use worker::{
    BackpressureState, ConsumedMessage, LiveSynapConsumer, LiveSynapPublisher, MemorySynapConsumer,
    MemorySynapPublisher, OffsetTracker, OutOfOrderBuffer, ReadyBatch, SynapConsumer, SynapHandle,
    SynapPublisher, Worker, BACKPRESSURE_RETRY, BACKPRESSURE_SOAK, STREAM_ENRICHED, STREAM_GRAPHED,
    STREAM_INVALID,
};
pub use writer::{GraphWriter, NexusGraphWriter};
