//! Phase14a §2 — per-grain [`super::Consolidator`] implementations.
//!
//! Each module ships one wrapper that turns a [`super::orchestrator`]
//! call (`run_session` / `run_topic` / `run_decision_trace`) into the
//! daemon-facing [`super::Consolidator`] surface. The wrappers are
//! trigger-driven: the daemon (§3) reads a [`super::orchestrator::Trigger`]
//! off the bus and dispatches it to the grain whose
//! [`super::Consolidator::grain`] matches.
//!
//! The [`crate::producer::EnvelopeProducer`] half of the trait
//! composition is intentionally a no-op `produce()` body — the
//! producer-checkpoint write per run is the daemon's responsibility
//! (see `consolidator_trait` docs §ADR-014 pure-reader contract).
//! The trait composition only exists so the daemon can resolve the
//! grain's stable name + lean on the existing
//! `record_producer_checkpoint` schema without forking a parallel
//! cursor store.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod session;
pub mod topic;

pub use session::{LiveSessionInputFetcher, SessionGrain, SessionInputFetcher};
pub use topic::{LiveTopicClusterFetcher, TopicClusterFetcher, TopicGrain};
