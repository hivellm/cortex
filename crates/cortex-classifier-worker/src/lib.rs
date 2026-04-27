//! Cortex classifier worker.
//!
//! Bridges `cortex.events.raw` (live ingestion) and
//! `cortex.events.bootstrap` (spec 09 backfill) onto
//! `cortex.events.enriched`, the stream the embedder, graph writer,
//! and full-text indexer all consume.
//!
//! The library half exposes the [`Worker`] loop and the Synap
//! consumer/publisher abstractions; the binary in `main.rs` wires
//! a default classifier stack ([`cortex_classifier::ClassifierStack`])
//! and runs the pool until ctrl-c.
//!
//! # Why a separate crate
//!
//! `cortex-embedder` already depends on `cortex-classifier` for
//! [`cortex_classifier::ClassifierOutput`]. The worker needs to publish
//! the same `EnrichedEvent` struct the embedder consumes, so it
//! depends on `cortex-embedder`. Putting the worker inside
//! `cortex-classifier` would create a `classifier -> embedder ->
//! classifier` cycle. A standalone crate keeps both crates pure
//! libraries and the worker dedicated to the wire bridge.

#![forbid(unsafe_code)]
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
