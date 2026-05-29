//! Phase18 §3 — temporal + branch retrieval classifier.
//!
//! The classifier runs after fusion (BM25 + dense + graph) and
//! before the cross-encoder reranker (phase17 P2). It walks every
//! candidate hit, switches on the bitemporal columns (`valid_from`,
//! `valid_to`, `superseded_at`, `lifecycle`) per ADR-018..023, and
//! emits a per-hit decision: pass / drop / boost.

pub mod branch_filter;
pub mod classifier;
