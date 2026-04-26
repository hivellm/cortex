//! Cortex graph-writer crate.
//!
//! Consumes enriched events from Synap, maps them onto the Cortex graph
//! schema (architecture §4.2), coalesces redundant node/edge upserts within
//! a micro-batch, and writes them to **Nexus** through the official
//! `nexus-graph-sdk` (crate name `nexus-graph-sdk`, library name
//! `nexus_sdk`). The writer is a *client* — it never owns the graph nor
//! re-implements Cypher.
//!
//! See `docs/specs/07-graph-writer.md` for the full contract. The spec
//! mentions the **Bolt** protocol; in practice Nexus exposes its own
//! RPC protocol over `nexus://` URLs (selected via `ClientConfig.transport`
//! or the `NEXUS_SDK_TRANSPORT` env var) plus an HTTP fallback. This crate
//! treats those two transports as the equivalent of "Bolt vs HTTP" in the
//! spec.
//!
//! `EnrichedEvent` is reused from `cortex-embedder` to avoid duplicating
//! the post-enrichment payload definition; both workers consume the same
//! `cortex.events.enriched` stream and must therefore agree on its shape.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod coalescer;
pub mod cypher;
pub mod identity;
pub mod mapper;
pub mod metrics;
pub mod nexus_client;
pub mod patch;
pub mod schema;
pub mod worker;
pub mod writer;

pub use config::{GraphConfig, GraphTransport};
pub use coalescer::{CoalesceStats, PatchCoalescer};
pub use cypher::CypherTemplates;
// Reuse the enriched-event payload defined by the sibling embedder crate so
// both workers stay in lockstep on the schema of `cortex.events.enriched`.
pub use cortex_embedder::EnrichedEvent;
pub use identity::artifact_natural_key;
pub use mapper::map_event_to_patch;
pub use metrics::Metrics;
pub use nexus_client::{
    GraphClient, GraphClientError, LiveNexusClient, MemoryCall, MemoryNexusClient, WriteStats,
};
pub use patch::{EdgeOp, GraphPatch, GraphWriteReport, NodeOp};
pub use schema::SCHEMA_STATEMENTS;
pub use worker::Worker;
pub use writer::{GraphWriter, NexusGraphWriter};
