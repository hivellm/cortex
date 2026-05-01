//! Phase11j §2.4-§2.6 — producers.
//!
//! Each producer takes a grain-specific input, renders the matching
//! [`crate::templates::Template`], runs it through a
//! [`crate::summariser::Summariser`], parses the JSON response into
//! a [`cortex_core::events::ConsolidationPayload`], and surfaces the
//! result. The §2.1 skeleton ships the trait + shared error surface
//! so the producer bodies (§2.4..§2.7) can land without reshaping
//! the surrounding modules.

pub mod decision_trace;
pub mod session;
pub mod topic;

use cortex_core::events::ConsolidationPayload;

/// Outcome of a producer run.
#[derive(Debug, Clone)]
pub struct ProducedConsolidation {
    /// The fully-shaped payload, ready for envelope wrapping.
    pub payload: ConsolidationPayload,
    /// USD cents the summariser charged for this run.
    pub cost_cents: u32,
}

/// Errors a producer surfaces. Keeps a clean separation from the
/// summariser-side errors so the orchestrator can route on the
/// right axis.
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    /// Input set was empty / below the per-grain minimum.
    #[error("empty input set: {0}")]
    EmptyInput(String),
    /// Summariser returned an error after retry.
    #[error("summariser: {0}")]
    Summariser(#[from] crate::summariser::SummariserError),
    /// Summariser returned a body the parser could not turn into a
    /// valid payload (missing keys, wrong types, etc.).
    #[error("summariser response did not match contract: {0}")]
    InvalidResponse(String),
    /// Validator rejected the produced payload (cross-field rule
    /// from `cortex_core::validate::validate_consolidation_payload`).
    #[error("validator rejected payload: {0}")]
    ValidationFailed(String),
}

/// Validate a produced payload against the §1.5 invariants. The
/// orchestrator calls this after every producer run so a hallucinated
/// model output never escapes the consolidator. Wraps the
/// `cortex_core::validate::validate_consolidation_payload` helper +
/// the field-length rules the JSON Schema enforces during `validate_event`.
pub fn validate_produced(payload: &ConsolidationPayload) -> Result<(), ProducerError> {
    if payload.title.chars().count() > cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS {
        return Err(ProducerError::ValidationFailed(format!(
            "title len ({}) exceeds {} chars",
            payload.title.chars().count(),
            cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS
        )));
    }
    let summary_len = payload.summary_markdown.len();
    if summary_len < cortex_core::events::CONSOLIDATION_SUMMARY_MIN_BYTES
        || summary_len > cortex_core::events::CONSOLIDATION_SUMMARY_MAX_BYTES
    {
        return Err(ProducerError::ValidationFailed(format!(
            "summary_markdown len ({summary_len}) outside [{}, {}]",
            cortex_core::events::CONSOLIDATION_SUMMARY_MIN_BYTES,
            cortex_core::events::CONSOLIDATION_SUMMARY_MAX_BYTES
        )));
    }
    cortex_core::validate::validate_consolidation_payload(payload)
        .map_err(|e| ProducerError::ValidationFailed(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{
        ConsolidationDepth, ConsolidationGrain, ConsolidationScope, TimeSpan,
    };
    use std::collections::BTreeMap;

    fn ok_payload() -> ConsolidationPayload {
        ConsolidationPayload {
            consolidation_id: "01CON".into(),
            grain: ConsolidationGrain::Session,
            scope: ConsolidationScope::SessionId("sess-A".into()),
            title: "ok".into(),
            summary_markdown: "x".repeat(400),
            takeaways: vec![],
            source_event_ids: vec![],
            source_event_count: 0,
            model: "claude-haiku-4-5".into(),
            depth: ConsolidationDepth::Shallow,
            outcome_distribution: BTreeMap::new(),
            temporal_span: TimeSpan {
                start_ms: 0,
                end_ms: 0,
                duration_ms: 0,
            },
            repos: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn validate_produced_accepts_payload_passing_every_rule() {
        validate_produced(&ok_payload()).expect("valid payload");
    }

    #[test]
    fn validate_produced_rejects_oversize_title() {
        let mut p = ok_payload();
        p.title = "x".repeat(81);
        let err = validate_produced(&p).expect_err("oversize title");
        assert!(format!("{err}").contains("title"));
    }

    #[test]
    fn validate_produced_rejects_summary_below_floor() {
        let mut p = ok_payload();
        p.summary_markdown = "too short".into();
        let err = validate_produced(&p).expect_err("undersize summary");
        assert!(format!("{err}").contains("summary_markdown"));
    }
}
