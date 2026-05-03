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
    // phase11j — consolidation summaries. Hot tier carries the recent
    // window + every Deep-depth consolidation (high-recall HNSW so the
    // pre-thinking renderer's "Consolidated context" lane finds them
    // on tight similarity gates); warm tier holds older Shallow
    // consolidations for cost.
    CollectionSchema {
        name: crate::names::COLLECTION_CONSOLIDATION_FP32,
        description: "Consolidation summaries (hot, recent + Deep)",
        tier: CollectionTier::Hot,
        dim: EMBED_DIM,
        encoding: "fp32",
        hnsw: HnswParams { m: 48, ef_search: 256 },
    },
    CollectionSchema {
        name: crate::names::COLLECTION_CONSOLIDATION_PQ,
        description: "Consolidation summaries (warm, older Shallow)",
        tier: CollectionTier::Warm,
        dim: EMBED_DIM,
        encoding: "pq",
        hnsw: HnswParams { m: 16, ef_search: 64 },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::{COLLECTION_CONSOLIDATION_FP32, COLLECTION_CONSOLIDATION_PQ};

    #[test]
    fn consolidation_collections_are_declared() {
        // Phase11j §3.5 — both tiers must be present so the embedder
        // worker can `ensure_collection` them at boot. Drop here is
        // load-bearing for the consolidator → Vectorizer write path.
        let names: Vec<&str> = COLLECTIONS.iter().map(|c| c.name).collect();
        assert!(
            names.contains(&COLLECTION_CONSOLIDATION_FP32),
            "COLLECTIONS must include cortex.consolidation.fp32",
        );
        assert!(
            names.contains(&COLLECTION_CONSOLIDATION_PQ),
            "COLLECTIONS must include cortex.consolidation.pq",
        );
    }

    #[test]
    fn consolidation_hot_tier_uses_recall_tuned_hnsw() {
        // Phase11j §3.5 — pre-thinking's "Consolidated context" lane
        // ranks consolidations on tight similarity gates; the hot
        // collection mirrors decisions/analyses recall settings
        // (m=48, ef_search=256) so the lane returns the right rows.
        let hot = COLLECTIONS
            .iter()
            .find(|c| c.name == COLLECTION_CONSOLIDATION_FP32)
            .expect("hot consolidation collection declared");
        assert_eq!(hot.tier, CollectionTier::Hot);
        assert_eq!(hot.encoding, "fp32");
        assert_eq!(hot.hnsw.m, 48);
        assert_eq!(hot.hnsw.ef_search, 256);
    }
}
