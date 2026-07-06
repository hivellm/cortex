//! Phase11p — Live read-path sources for the consolidator.
//!
//! Three sources land here, one per `ConsolidationGrain`:
//!
//! - [`session::LiveSessionSource`] reads a session's full envelope
//!   set from the parquet archive via
//!   [`cortex_storage::archive::scan_envelopes_by_session`].
//! - [`topic::LiveTopicSource`] reads turn envelopes inside a
//!   per-repo time window, runs HDBSCAN over their inline
//!   `payload.embedding` arrays, and emits one `TopicCluster` per
//!   non-noise label.
//! - [`decision_trace::LiveDecisionTraceSource`] walks the
//!   `parent_event_id` chain from a `Kind::Decision` envelope up to
//!   `MAX_HOPS = 16` hops via
//!   [`cortex_storage::archive::scan_envelope_by_event_id`].
//!
//! Each source returns the producer-facing input type
//! (`SessionInput`, `Vec<TopicCluster>`, `DecisionTraceInput`)
//! already shaped for `Orchestrator::run_session` /
//! `run_topic` / `run_decision_trace`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod community;
pub mod decision_trace;
pub mod session;
pub mod topic;

pub use community::LiveCommunitySource;
pub use decision_trace::LiveDecisionTraceSource;
pub use session::LiveSessionSource;
pub use topic::LiveTopicSource;

/// Failure modes shared across the three live sources.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// Storage-layer failure (archive walk / parse / cycle detected).
    #[error("storage: {0}")]
    Storage(String),
    /// Vectorizer client failure (reserved for future per-collection
    /// reads on the topic source; the current archive-backed path
    /// does not exercise this variant).
    #[error("vectorizer: {0}")]
    Vectorizer(String),
    /// HDBSCAN clustering failure — propagated from the upstream
    /// crate when its `cluster()` method returns an error.
    #[error("cluster: {0}")]
    Cluster(String),
    /// The lookup completed cleanly but found nothing — the caller
    /// decides whether to treat as benign (the bin's `run-session`
    /// prints a clear message and exits 0) or as an error.
    #[error("empty result")]
    EmptyResult,
}

impl From<cortex_storage::archive::ScanError> for SourceError {
    fn from(e: cortex_storage::archive::ScanError) -> Self {
        SourceError::Storage(e.to_string())
    }
}
