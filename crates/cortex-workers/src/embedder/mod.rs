//! Cortex embedder (was crate `cortex-embedder`).
//!
//! Consumes enriched events from Synap, chunks their payloads, and
//! writes vectors via the Vectorizer SDK with deterministic
//! `dedup_key`s. Defines [`embedder::EnrichedEvent`] — the canonical
//! post-enrichment payload reused by the fulltext + graph workers.

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
pub mod token_cache;
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
pub use token_cache::{parse_jwt_exp_ms, TokenCache, DEFAULT_TOKEN_TTL_SECS, REFRESH_BUFFER_SECS};
pub use vectorizer_client::{
    with_retry, CollectionSchema, LiveVectorizerClient, MemoryCall, MemoryVectorizerClient, Metric,
    UpsertReport, UpsertedChunk, VectorizerClient, VectorizerClientError,
};
pub use worker::{
    BackpressureState, ConsumedMessage, LiveSynapConsumer, LiveSynapPublisher, MemorySynapConsumer,
    MemorySynapPublisher, OffsetTracker, SynapConsumer, SynapHandle, SynapPublisher, Worker,
    STREAM_EMBEDDED, STREAM_ENRICHED, STREAM_INVALID,
};
