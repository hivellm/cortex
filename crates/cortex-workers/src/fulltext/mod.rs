//! Cortex full-text indexer (was crate `cortex-fulltext`).
//!
//! Consumes enriched events from Synap, builds Meilisearch documents
//! per spec 08, and upserts them into the per-kind index family.

#![warn(missing_docs)]

pub mod audit;
pub mod body;
pub mod boot_replay;
pub mod builders;
pub mod config;
pub mod document;
pub mod indexer;
pub mod meili_client;
pub mod metrics;
pub mod routing;
pub mod settings;
pub mod sweep;
pub mod worker;

pub use crate::embedder::EnrichedEvent;
pub use body::{select_body, BodySource, SelectedBody, OVERSIZE_BODY_BYTES};
pub use boot_replay::{missing_partitions, replay_missing_partitions, Partition, ReplayReport};
pub use builders::{build_doc, BuildOutcome, TITLE_MAX_CHARS};
pub use config::FulltextConfig;
pub use document::{bootstrap_doc_id, live_doc_id, Document};
pub use indexer::{FulltextIndexer, IndexReport, MeiliFulltextIndexer};
pub use meili_client::{
    IndexStat, LiveMeiliClient, MeiliClient, MeiliError, MemoryCall, MemoryMeiliClient, TaskStatus,
    TaskUid, UpsertReport,
};
pub use metrics::Metrics;
pub use routing::{
    family_for, family_for_event, index_for, index_for_event, index_name, is_canonical_index_name,
    FAMILIES,
};
pub use settings::{load_settings_v1, settings_v1_json, SETTINGS_V1};
pub use sweep::{sweep_stale_indexes, SweepReport};
pub use worker::{
    BackpressureState, ConsumedMessage, LiveSynapConsumer, LiveSynapPublisher, MemorySynapConsumer,
    MemorySynapPublisher, OffsetTracker, SynapConsumer, SynapHandle, SynapPublisher, Worker,
    BACKPRESSURE_SOAK, STREAM_ENRICHED, STREAM_FULLTEXT_INDEXED, STREAM_INVALID,
};
