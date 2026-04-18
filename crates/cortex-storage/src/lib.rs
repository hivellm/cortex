//! Cortex storage layout — source of truth for every backend namespace.
//!
//! This crate declares **where** things live:
//!
//! - Vectorizer collection names + per-collection schema ([`collections`]).
//! - Nexus labels + index bootstrap Cypher ([`graph`]).
//! - Meilisearch index names + `settings.v1.json` ([`fulltext`]).
//! - Synap streams, pub/sub topics, KV namespaces ([`streams`]).
//! - Parquet archive partition layout ([`archive`]).
//! - SQLite metadata schema ([`metadata`]).
//! - SQLite-backed CAS blob store ([`cas`]).
//!
//! Worker crates (05, 06, 07, 08) consume the constants and schemas declared
//! here — they never hardcode their own.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod archive;
pub mod cas;
pub mod collections;
pub mod fulltext;
pub mod graph;
pub mod metadata;
pub mod names;
pub mod streams;

pub use archive::{archive_filename, archive_partition, ArchiveLayout, ArchiveRotation};
pub use cas::{CasBlob, CasContentType, CasError, CasStore};
pub use collections::{CollectionSchema, CollectionTier, COLLECTIONS};
pub use metadata::{MetadataError, MetadataStore};
pub use names::*;
