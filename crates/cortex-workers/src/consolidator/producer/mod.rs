//! Phase11j §2.4-§2.6 — producers.
//!
//! Each producer takes a grain-specific input, renders the matching
//! [`super::templates::Template`], runs it through a
//! [`super::summariser::Summariser`], parses the JSON response into
//! a [`cortex_core::events::ConsolidationPayload`], and surfaces the
//! result. The §2.1 skeleton ships the trait + shared error surface
//! so the producer bodies (§2.4..§2.7) can land without reshaping
//! the surrounding modules.

pub mod community;
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
    /// Prompt tokens the summariser consumed (phase14a §2.4 — fed
    /// into [`super::cost_telemetry::CostLedger::record_full`] via
    /// [`super::consolidator_trait::ConsolidatorCtx::record_cost`]).
    pub input_tokens: u64,
    /// Completion tokens the summariser produced (same plumbing).
    pub output_tokens: u64,
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
    Summariser(#[from] super::summariser::SummariserError),
    /// Summariser returned a body the parser could not turn into a
    /// valid payload (missing keys, wrong types, etc.).
    #[error("summariser response did not match contract: {0}")]
    InvalidResponse(String),
    /// Validator rejected the produced payload (cross-field rule
    /// from `cortex_core::validate::validate_consolidation_payload`).
    #[error("validator rejected payload: {0}")]
    ValidationFailed(String),
}

/// Phase11j §2.10 — derive a stable `consolidation_id` from the
/// `(grain, scope)` pair so a re-run of the same producer against
/// the same input does NOT emit a fresh id. The shape is
/// `cons-{grain_short}-{sha256[:24]}` — short enough to read in a
/// log line, long enough to avoid collisions across the corpus.
pub fn derive_consolidation_id(
    grain: cortex_core::events::ConsolidationGrain,
    scope: &cortex_core::events::ConsolidationScope,
) -> String {
    use cortex_core::events::{ConsolidationGrain as G, ConsolidationScope as S};
    use sha2::{Digest, Sha256};
    let grain_short = match grain {
        G::Session => "ses",
        G::Topic => "top",
        G::DecisionTrace => "dec",
        G::Community => "com",
    };
    let scope_key = match scope {
        S::SessionId(s) => format!("session_id:{s}"),
        S::Topic(s) => format!("topic:{s}"),
        S::DecisionId(s) => format!("decision_id:{s}"),
        S::Community {
            community_id,
            level,
        } => format!("community:{community_id}@{level}"),
    };
    let mut hasher = Sha256::new();
    hasher.update(grain_short.as_bytes());
    hasher.update(b"|");
    hasher.update(scope_key.as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("cons-{grain_short}-{}", &hex[..24])
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
    if !(cortex_core::events::CONSOLIDATION_SUMMARY_MIN_BYTES
        ..=cortex_core::events::CONSOLIDATION_SUMMARY_MAX_BYTES)
        .contains(&summary_len)
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
        p.title = "x".repeat(cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS + 1);
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

    #[test]
    fn derive_consolidation_id_is_deterministic_per_grain_and_scope() {
        let s1 = ConsolidationScope::SessionId("01HXSESS01".into());
        let s2 = ConsolidationScope::SessionId("01HXSESS01".into());
        let id1 = derive_consolidation_id(ConsolidationGrain::Session, &s1);
        let id2 = derive_consolidation_id(ConsolidationGrain::Session, &s2);
        assert_eq!(id1, id2, "same (grain, scope) must produce same id");
        assert!(id1.starts_with("cons-ses-"));
    }

    #[test]
    fn derive_consolidation_id_differs_across_grains_and_scopes() {
        let same_scope = ConsolidationScope::SessionId("01HXSESS01".into());
        let session_id = derive_consolidation_id(ConsolidationGrain::Session, &same_scope);
        let topic_scope = ConsolidationScope::Topic("hnsw".into());
        let topic_id = derive_consolidation_id(ConsolidationGrain::Topic, &topic_scope);
        let other_session = ConsolidationScope::SessionId("01HXSESS02".into());
        let other_session_id = derive_consolidation_id(ConsolidationGrain::Session, &other_session);
        assert_ne!(session_id, topic_id, "grain disambiguates");
        assert_ne!(session_id, other_session_id, "scope disambiguates");
        assert!(topic_id.starts_with("cons-top-"));
    }
}
