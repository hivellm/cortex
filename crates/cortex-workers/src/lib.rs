//! Cortex workers — consolidated event-pipeline workers.
//!
//! Single crate hosting the four daemons that consume `cortex.events.*`
//! and write to backends:
//!
//! - [`classifier`] — bridges raw + bootstrap → enriched (was crate
//!   `cortex-classifier-worker`)
//! - [`embedder`] — chunks enriched events and writes vectors to
//!   Vectorizer (was crate `cortex-embedder`)
//! - [`fulltext`] — maps enriched events to Meilisearch documents
//!   (was crate `cortex-fulltext`)
//! - [`graph`] — maps enriched events to Nexus nodes/edges (was
//!   crate `cortex-graph`)
//!
//! Each module preserves the public API of its previous crate so
//! callers update imports with a one-line rewrite
//! (`crate::embedder::Worker` → `crate::embedder::Worker`).
//! `EnrichedEvent` is canonically defined inside [`embedder`] and
//! re-exported by [`fulltext`] + [`graph`].

#![forbid(unsafe_code)]

pub mod admin_health;
pub mod classifier;
pub mod classifier_worker;
#[cfg(feature = "claude-archive")]
pub mod claude_archive;
pub mod consolidator;
pub mod embedder;
pub mod fulltext;
pub mod graph;
pub mod ingestion;
pub mod retention;
pub mod topic_cards;
