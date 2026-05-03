//! Cortex classifier worker (was crate `cortex-classifier-worker`).
//!
//! Bridges `cortex.events.raw` (live ingestion) and
//! `cortex.events.bootstrap` (spec 09 backfill) onto
//! `cortex.events.enriched`, the stream the embedder, graph writer,
//! and full-text indexer all consume.

#![warn(missing_docs)]

pub mod config;
pub mod kinds;
pub mod worker;

pub use config::{ClassifierMode, ClassifierWorkerConfig};
pub use kinds::{kind_from_bootstrap, KindMapError};
pub use worker::{
    ConsumedMessage, LiveSynapConsumer, LiveSynapPublisher, MemorySynapConsumer,
    MemorySynapPublisher, OffsetTracker, SynapConsumer, SynapHandle, SynapPublisher, Worker,
    STREAM_BOOTSTRAP, STREAM_ENRICHED, STREAM_RAW,
};
