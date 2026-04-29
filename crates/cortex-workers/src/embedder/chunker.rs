//! Chunker trait and shared chunk types.

use anyhow::Result;
use cortex_classifier::Severity;
use cortex_core::events::Kind;
use serde::{Deserialize, Serialize};

use super::embedder::EnrichedEvent;

/// A unit of content that will be embedded and upserted into Vectorizer.
///
/// The struct carries the **client-side dedup key** (see
/// [`super::identity::dedup_key`]) which is stored as `metadata.dedup_key`
/// on the upserted vector. The **server-assigned UUID** is the actual
/// primary id and is only available post-upsert, surfaced through
/// [`super::vectorizer_client::UpsertedChunk::server_id`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Deterministic dedup key, stored as metadata. The server-assigned
    /// UUID is the actual primary id and is returned post-upsert via
    /// [`super::vectorizer_client::UpsertedChunk`].
    pub dedup_key: String,
    /// Parent enriched-event id.
    pub parent_event_id: String,
    /// Parent event content hash (pre-chunking).
    pub parent_content_hash: String,
    /// SHA-256 hash of this chunk's text.
    pub chunk_content_hash: String,
    /// Destination Vectorizer collection.
    pub collection: String,
    /// Text that will be embedded.
    pub text: String,
    /// Metadata stored alongside the vector.
    pub metadata: ChunkMetadata,
}

/// Metadata written to Vectorizer next to the embedded text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Coarse event kind.
    pub kind: Kind,
    /// Multi-label topics from the classifier.
    pub topics: Vec<String>,
    /// Severity from the classifier.
    pub severity: Severity,
    /// Optional repo hint.
    pub repo: Option<String>,
    /// Optional path hint.
    pub path: Option<String>,
    /// Code symbol name (fn/struct/class), when known.
    pub symbol: Option<String>,
    /// Byte range within the parent payload.
    pub byte_range: Option<(u32, u32)>,
    /// Source language (for code chunks).
    pub language: Option<String>,
    /// Which chunker produced this record.
    pub source: ChunkSource,
    /// Prompt template version, when `source == Summary`.
    pub prompt_version: Option<String>,
}

/// Which chunker produced a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkSource {
    /// Tree-sitter code chunker (top-level declaration).
    Code,
    /// Markdown section chunker.
    Doc,
    /// Classifier-provided summary for an oversize payload.
    Summary,
    /// Sliding-window fallback (unknown language / oversize).
    FallbackWindow,
    /// Full oversize raw text stored alongside a summary; never re-embedded.
    RawOversize,
}

/// Trait for anything that turns an enriched event into a list of chunks.
pub trait Chunker: Send + Sync {
    /// Return the chunks produced from `event`.
    fn chunk(&self, event: &EnrichedEvent) -> Result<Vec<Chunk>>;
}
