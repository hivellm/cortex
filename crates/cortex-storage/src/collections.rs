//! Declarative Vectorizer collection schemas. Consumed by the embedder worker
//! (spec 06) and by the local-stack seed script (spec 03).
//!
//! These are *data*, not calls. The actual `ensure_collection` against
//! Vectorizer lives in the embedder.

use serde::Serialize;

/// Tier a collection sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionTier {
    /// Newest, FP32 vectors, recall-tuned HNSW.
    Hot,
    /// Older, PQ-compressed, memory-friendly HNSW.
    Warm,
    /// Oldest, binary-quantized, cheap.
    Cold,
}

/// HNSW parameters for a collection.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HnswParams {
    /// Graph connectivity.
    pub m: u32,
    /// Search effort.
    pub ef_search: u32,
}

/// Declarative schema for one Vectorizer collection.
#[derive(Debug, Clone, Serialize)]
pub struct CollectionSchema {
    /// Fully-qualified name, e.g. `cortex.turn.fp32`.
    pub name: &'static str,
    /// Vectors of what (one-line description).
    pub description: &'static str,
    /// Cost/retention tier.
    pub tier: CollectionTier,
    /// Vector dimension (all collections share the same embedding model).
    pub dim: u32,
    /// Quantization encoding.
    pub encoding: &'static str,
    /// HNSW parameters.
    pub hnsw: HnswParams,
}

/// Embedding model used across every collection.
pub const EMBED_MODEL: &str = "nomic-embed-text-v1.5";
/// Vector dimension for [`EMBED_MODEL`].
pub const EMBED_DIM: u32 = 768;

/// Every Cortex collection, in bootstrap order.
pub const COLLECTIONS: &[CollectionSchema] = &[
    CollectionSchema {
        name: crate::names::COLLECTION_TURN_FP32,
        description: "Turn summaries (hot, ≤30 days)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_TURN_PQ,
        description: "Turn summaries (warm, 30–365 days)",
        tier: CollectionTier::Warm,
        dim: EMBED_DIM,
        encoding: "pq",
        hnsw: HnswParams { m: 16, ef_search: 64 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_TOOL_CALL_FP32,
        description: "Tool call summaries (hot)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_TOOL_CALL_PQ,
        description: "Tool call summaries (warm)",
        tier: CollectionTier::Warm,
        dim: EMBED_DIM,
        encoding: "pq",
        hnsw: HnswParams { m: 16, ef_search: 64 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_CODE_CHUNK_FP32,
        description: "Code chunks (current HEAD)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_CODE_CHUNK_PQ,
        description: "Code chunks (historical)",
        tier: CollectionTier::Warm,
        dim: EMBED_DIM,
        encoding: "pq",
        hnsw: HnswParams { m: 16, ef_search: 64 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_DOC_CHUNK_FP32,
        description: "Doc chunks",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_DECISION_FP32,
        description: "Decisions (recall-tuned)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 48, ef_search: 256 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_ANALYSIS_FP32,
        description: "Analyses (recall-tuned)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 48, ef_search: 256 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_MEMORY_FP32,
        description: "Memory entries",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_LAW_FP32,
        description: "Law definitions",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 48, ef_search: 256 },
    },
    // phase10e — Rulebook MCP captures (`rulebook_knowledge_add` /
    // `rulebook_learn_capture`). Single hot tier; the corpus is
    // small + dense, demoting to PQ would lose precision the
    // agent needs when re-reading the entry verbatim.
    CollectionSchema {
        name: crate::names::COLLECTION_KNOWLEDGE_FP32,
        description: "Pattern / anti-pattern entries (Rulebook knowledge)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_LEARNING_FP32,
        description: "Implementation learnings (Rulebook learnings)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 32, ef_search: 128 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_COLD_BINARY,
        description: "Binary-quantized fallback (any kind, >365 days)",
        tier: CollectionTier::Cold,
        dim: EMBED_DIM,
        encoding: "binary",
        hnsw: HnswParams { m: 8, ef_search: 32 },
    },
];

