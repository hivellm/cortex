//! Phase11r §2 — topic-cards module public re-exports.
//!
//! Consumers can `use cortex_workers::topic_cards::prelude::*` to pull in the
//! most-used types without navigating the sub-module tree.

pub use super::contradictions::{scan as scan_contradictions, EvidenceFacts, HydratedEvidence};
pub use super::orchestrator::{Orchestrator, OrchestratorError, GRAIN_LABEL};
pub use super::producer::{produce, ProduceInput, ProducedTopicCard, ProducerError};
pub use super::synthesiser::{RewriteOutput, SynthesiserError, TopicCardSynthesiser};
pub use super::templates::{render_rewrite_prompt, RewriteSlots, OUTPUT_CONTRACT};
pub use super::trigger::{
    HoldReason, Trigger, TriggerDecision, TRIGGER_AGE_DAYS, TRIGGER_DISTANCE_THRESHOLD,
    TRIGGER_EVENTS_THRESHOLD,
};
