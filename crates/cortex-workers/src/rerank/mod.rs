//! Cross-encoder reranker abstraction (phase17 §2.1).
//!
//! Defines the [`Reranker`] trait and [`Candidate`] input type. The
//! concrete TEI / BGE-reranker-v2-m3 implementation lives in
//! [`crate::rerank::bge_v2m3`].

pub mod bge_v2m3;

use async_trait::async_trait;

/// Single input item sent to the reranker.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Document ID propagated from the fusion lane (for ordering results back).
    pub doc_id: String,
    /// Full text content the reranker scores against the query.
    pub text: String,
}

/// Errors that a reranker implementation can return.
#[derive(Debug, thiserror::Error)]
pub enum RerankerError {
    /// HTTP-level failure (connection refused, non-2xx status, …).
    #[error("http: {0}")]
    Http(String),
    /// Response body could not be deserialised.
    #[error("decode: {0}")]
    Decode(String),
    /// Request timed out.
    #[error("timeout after {ms}ms")]
    Timeout { ms: u64 },
}

/// A cross-encoder reranker.
///
/// Implementations call an external scoring endpoint (e.g. TEI) and
/// return one relevance score per candidate in input order. Scores are
/// typically logits or probabilities; higher = more relevant. The
/// orchestrator overwrites `LaneHit::score` with the returned value.
#[async_trait]
pub trait Reranker: Send + Sync + 'static {
    /// Score every candidate against `query`.
    ///
    /// Returns a `Vec<f32>` of the same length as `candidates`, in the
    /// same order. On timeout or service error the caller falls back to
    /// the pre-rerank fusion order (fail-open — §2.5).
    async fn score(&self, query: &str, candidates: &[Candidate])
        -> Result<Vec<f32>, RerankerError>;
}
