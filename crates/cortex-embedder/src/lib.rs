//! Cortex embedder crate.
//!
//! Consumes enriched events from Synap, chunks their payloads (code via
//! Tree-sitter, docs by section, fallback by sliding window), and writes
//! vectors into the Vectorizer service with a stable, deterministic
//! per-chunk `dedup_key` (stored as metadata) so re-runs are idempotent.
//! The server assigns its own primary UUID per stored vector; Cortex
//! surfaces the mapping via [`crate::vectorizer_client::UpsertedChunk`].
//!
//! This crate is the **client**; it never owns an index or runs embedding
//! models. See `docs/specs/06-embedder.md` for the full contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod chunker;
pub mod chunker_code;
pub mod chunker_doc;
pub mod chunker_fallback;
pub mod config;
pub mod embedder;
pub mod identity;
pub mod metrics;
pub mod routing;
pub mod vectorizer_client;
pub mod worker;

pub use chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
pub use chunker_code::CodeChunker;
pub use chunker_doc::DocChunker;
pub use chunker_fallback::FallbackChunker;
pub use config::EmbedderConfig;
pub use embedder::{EmbedError, EmbedReport, Embedder, EnrichedEvent, VectorizerEmbedder};
pub use identity::dedup_key;
pub use metrics::Metrics;
pub use routing::collection_for;
pub use vectorizer_client::{
    with_retry, CollectionSchema, LiveVectorizerClient, MemoryCall, MemoryVectorizerClient,
    Metric, UpsertReport, UpsertedChunk, VectorizerClient, VectorizerClientError,
};
pub use worker::{
    BackpressureState, ConsumedMessage, LiveSynapConsumer, LiveSynapPublisher,
    MemorySynapConsumer, MemorySynapPublisher, OffsetTracker, SynapConsumer, SynapHandle,
    SynapPublisher, Worker, STREAM_EMBEDDED, STREAM_ENRICHED, STREAM_INVALID,
};
