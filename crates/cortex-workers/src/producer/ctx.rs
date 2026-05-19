//! [`ProducerCtx`] — the shared environment passed to every
//! [`EnvelopeProducer::produce`](super::EnvelopeProducer::produce)
//! invocation.
//!
//! Design note (ADR-010 §2.2). The proposal text reads "ProducerCtx
//! carries handles to Synap, Meili, Nexus, Vectorizer plus the
//! producer-name string". Realised pragmatically: the ctx carries
//! only what is shared across every producer:
//!
//! - the metadata-store handle (every producer writes a
//!   `producer_checkpoints` row; the SQLite connection is the one
//!   piece of infrastructure no producer can skip),
//! - the reference clock (`now`) so producers are time-travellable
//!   under test without a global mock,
//! - the producer-name `&'static str` for logging,
//! - a logger target string the producer prefixes its tracing
//!   spans with.
//!
//! Per-backend handles (Synap publisher, Meili / Nexus / Vectorizer
//! clients) live on each `impl EnvelopeProducer` struct.
//! Constructors inject them. The ctx is an environment, not a
//! service locator — same shape as the `SweepCtx` from ADR-009.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use cortex_storage::MetadataStore;
use tokio::sync::Mutex;

/// Thread-safe handle to the metadata store. Wraps
/// `Arc<Mutex<MetadataStore>>` because `MetadataStore`'s rusqlite
/// connection is `Send` but not `Sync`, and the supervisor hands
/// the same handle to every producer across tokio tasks.
pub type ProducerMetadataHandle = Arc<Mutex<MetadataStore>>;

/// Environment handed to every
/// [`EnvelopeProducer::produce`](super::EnvelopeProducer::produce).
///
/// `Debug` is hand-written because `MetadataStore` (the SQLite
/// connection wrapper) does not implement it.
#[derive(Clone)]
pub struct ProducerCtx {
    /// SQLite-backed metadata store. Producers write append-only
    /// `producer_checkpoints` rows through this handle.
    pub metadata: ProducerMetadataHandle,
    /// Reference time. Production wires `Utc::now()`; tests pin to
    /// a fixed instant so checkpoint timestamps stay deterministic.
    pub now: DateTime<Utc>,
    /// Logger target string the producer prefixes its tracing
    /// spans with — e.g., `"cortex.producer.bootstrap"`.
    pub logger_target: &'static str,
}

impl ProducerCtx {
    /// Build a ctx with `Utc::now()` and the supplied logger
    /// target — the shape the production daemon instantiates per
    /// supervisor tick.
    pub fn new(metadata: ProducerMetadataHandle, logger_target: &'static str) -> Self {
        Self {
            metadata,
            now: Utc::now(),
            logger_target,
        }
    }

    /// Builder shim — pin the reference clock. Used by integration
    /// tests that time-travel.
    #[must_use]
    pub fn with_now(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }
}

impl std::fmt::Debug for ProducerCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProducerCtx")
            .field("metadata", &"<MetadataStore>")
            .field("now", &self.now)
            .field("logger_target", &self.logger_target)
            .finish()
    }
}
