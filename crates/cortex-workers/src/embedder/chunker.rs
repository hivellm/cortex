//! Chunker trait and shared chunk types.

use crate::classifier::Severity;
use anyhow::Result;
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
    // Phase18 §2.7 — bitemporal payload extension. Mirrors the
    // Meili schema (§2.6) so the temporal classifier (§3) can
    // pre-filter Vectorizer KNN hits on the same axes the Meili
    // keyword lane uses. Every `*_unix` value is second-precision
    // epoch per ADR-018; `valid_to_unix` and `superseded_at_unix`
    // default to absent (NULL = still valid).
    /// Project scope copied off `EnrichedEvent.context_repo`
    /// (lower-cased; `"unknown"` fallback). Filterable so a
    /// `cortex_vector_search?project_id=cortex` scopes the KNN walk
    /// to one repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch scope (`"main"` until §2.12 ships per-project branches).
    /// Filterable for branch-isolated retrievals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// `proposed | active | superseded | deprecated | abandoned | merged`
    /// per ADR-018. Filterable so the classifier drops superseded /
    /// abandoned rows from default retrievals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// When the fact starts being true (epoch seconds, UTC).
    /// Sortable for time-window scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from_unix: Option<i64>,
    /// When the fact stops; absent = still valid (ADR-018 §1.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to_unix: Option<i64>,
    /// When Cortex stopped believing the version; absent = still
    /// believed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at_unix: Option<i64>,
}

impl ChunkMetadata {
    /// Phase18 §2.7 — stamp the bitemporal payload columns from an
    /// `EnrichedEvent`. Mirrors `graph::bitemporal::stamp_one_node`
    /// and `fulltext::builders::apply_bitemporal_projection` so the
    /// three writers (graph, Meili, Vectorizer) carry the same
    /// values per event. `valid_to_unix` and `superseded_at_unix`
    /// stay absent (NULL = still valid per ADR-018); the SUPERSEDES
    /// backfill stamps them later on the target row of each
    /// supersession event. Lifecycle defaults to `active`; callers
    /// that already know a payload-derived override (e.g.
    /// `DecisionPayload.status = "superseded"`) should set
    /// `metadata.lifecycle` BEFORE calling this method.
    pub fn stamp_bitemporal(&mut self, event: &crate::embedder::EnrichedEvent) {
        if self.project_id.is_none() {
            self.project_id = Some(
                event
                    .context_repo
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_else(|| "unknown".to_string()),
            );
        }
        if self.branch_id.is_none() {
            self.branch_id = Some("main".to_string());
        }
        if self.lifecycle.is_none() {
            self.lifecycle = Some("active".to_string());
        }
        if self.valid_from_unix.is_none() && event.occurred_at_ms > 0 {
            self.valid_from_unix = Some(event.occurred_at_ms / 1_000);
        }
        // `valid_to_unix` + `superseded_at_unix` stay absent —
        // ADR-018 §1.1 treats NULL as "still valid / still believed".
    }
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
