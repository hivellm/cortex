//! Phase11j §2.6 — Decision-trace producer.
//!
//! Walks `parent_event_id` from a `Kind::Decision` envelope up to
//! `MAX_HOPS` ancestors. Output: one consolidation with
//! `grain = DecisionTrace` covering the chain root → decision.
//! Auto-promoted to Opus by the orchestrator (deeper grain → higher
//! fidelity threshold).

use cortex_core::events::Envelope;

/// Phase11j §2.6 — maximum number of `parent_event_id` hops the
/// producer walks. Bounds the prompt size and the cost ceiling.
pub const MAX_HOPS: usize = 16;

/// Input the decision-trace producer reads.
#[derive(Debug, Clone)]
pub struct DecisionTraceInput {
    /// The decision envelope that triggered the run.
    pub decision: Envelope,
    /// Ancestor envelopes ordered root → decision.parent. Capped at
    /// [`MAX_HOPS`]; the orchestrator clips before invoking the
    /// producer so the cap is honoured before any prompt rendering.
    pub chain: Vec<Envelope>,
}

impl DecisionTraceInput {
    /// Validate the input shape before invoking the summariser.
    pub fn ensure_well_formed(&self) -> Result<(), super::ProducerError> {
        if self.chain.len() > MAX_HOPS {
            return Err(super::ProducerError::EmptyInput(format!(
                "chain {} exceeds MAX_HOPS = {}",
                self.chain.len(),
                MAX_HOPS
            )));
        }
        if self.decision.kind != cortex_core::events::Kind::Decision {
            return Err(super::ProducerError::InvalidResponse(format!(
                "trigger envelope is not Kind::Decision (got {:?})",
                self.decision.kind
            )));
        }
        Ok(())
    }
}
