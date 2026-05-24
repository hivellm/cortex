//! Phase11r §2.4 — topic-card rewrite pipeline.
//!
//! [`produce`] is the single entry point: it renders the rewrite prompt,
//! calls the synthesiser, validates the output, and builds a
//! [`TopicCardPayload`] with a monotonically-incremented `revision`.

use chrono::Utc;
use cortex_core::events::{
    Contradiction, ContradictionStatus, EvidenceRef, TopicCardPayload,
    TOPIC_CARD_OPEN_QUESTIONS_MAX, TOPIC_CARD_SYNTHESIS_MAX_BYTES, TOPIC_CARD_SYNTHESIS_MIN_BYTES,
};

use super::synthesiser::{RewriteOutput, SynthesiserError, TopicCardSynthesiser};
use super::templates::RewriteSlots;

/// Input to the producer — may either be the first emit (no card yet)
/// or an incremental rewrite of an existing card.
#[derive(Debug, Clone)]
pub struct ProduceInput {
    /// Kebab-case topic slug (e.g. `auth-rewrite`).
    pub topic_slug: String,
    /// Repository scope (usually one entry).
    pub repo_scope: String,
    /// Full prior card, if any. `None` on first emit.
    pub existing_card: Option<TopicCardPayload>,
    /// Accumulated evidence items to include in the synthesis.
    pub all_evidence: Vec<EvidenceRef>,
    /// Rendered text block for new evidence items (shown in the prompt).
    pub new_evidence_text: String,
    /// Rendered text block for superseded evidence (shown in the prompt).
    pub superseded_evidence_text: String,
    /// Pass `true` to use Opus regardless of contradiction count.
    pub force_deep: bool,
}

/// Outcome of a successful [`produce`] call.
#[derive(Debug, Clone)]
pub struct ProducedTopicCard {
    /// The fully-shaped payload.
    pub payload: TopicCardPayload,
    /// Realised cost in USD cents.
    pub cost_cents: u32,
}

/// Errors the producer can surface.
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    /// Synthesiser layer failed.
    #[error("synthesiser: {0}")]
    Synthesiser(#[from] SynthesiserError),
    /// Model output did not satisfy the synthesis byte-range contract.
    #[error("synthesis below floor ({len} bytes, min {min})")]
    SynthesisBelowFloor {
        /// Actual byte length.
        len: usize,
        /// Minimum required.
        min: usize,
    },
    /// Model output exceeded the synthesis ceiling.
    #[error("synthesis above ceiling ({len} bytes, max {max})")]
    SynthesisAboveCeiling {
        /// Actual byte length.
        len: usize,
        /// Maximum allowed.
        max: usize,
    },
    /// Model emitted a required field with an invalid shape.
    #[error("invalid field: {0}")]
    InvalidField(String),
}

/// Run the full rewrite pipeline and return a [`ProducedTopicCard`].
///
/// Pipeline:
/// 1. Build [`RewriteSlots`] from `input`.
/// 2. Call `synthesiser.rewrite(slots)` → [`RewriteOutput`].
/// 3. Validate synthesis byte range and `open_questions` cap.
/// 4. Enrich raw contradictions with `surfaced_at_rev` + `status = Open`.
/// 5. Build [`TopicCardPayload`] with `revision = prior.revision + 1`
///    (or `1` on first emit), `events_since_last_rev = 0`,
///    `last_rev_at = now()`.
pub async fn produce(
    input: &ProduceInput,
    synthesiser: &TopicCardSynthesiser,
) -> Result<ProducedTopicCard, ProducerError> {
    let prior_revision = input
        .existing_card
        .as_ref()
        .map(|c| c.revision)
        .unwrap_or(0);
    let next_revision = prior_revision + 1;

    let existing_synthesis = input
        .existing_card
        .as_ref()
        .map(|c| c.synthesis_markdown.as_str())
        .unwrap_or("(no prior synthesis)");
    let existing_revision_str = prior_revision.to_string();

    let slots = RewriteSlots {
        topic_slug: &input.topic_slug,
        repo_scope: &input.repo_scope,
        existing_synthesis,
        existing_revision: &existing_revision_str,
        new_evidence_block: &input.new_evidence_text,
        superseded_evidence_block: &input.superseded_evidence_text,
    };

    let output: RewriteOutput = synthesiser.rewrite(slots).await?;

    validate_output(&output)?;

    let contradictions = enrich_contradictions(&output, next_revision);

    let topic_card_id =
        cortex_core::events::derive_topic_card_id(&input.topic_slug, &input.repo_scope);

    let synthesis_model = synthesiser.summariser_kind().model_id().to_string();

    let payload = TopicCardPayload {
        topic_card_id,
        topic_slug: input.topic_slug.clone(),
        repos: vec![input.repo_scope.clone()],
        revision: next_revision,
        synthesis_markdown: output.synthesis_markdown,
        evidence: input.all_evidence.clone(),
        contradictions,
        open_questions: output.open_questions,
        related_topic_ids: input
            .existing_card
            .as_ref()
            .map(|c| c.related_topic_ids.clone())
            .unwrap_or_default(),
        confidence: output.confidence,
        last_rev_at: Utc::now().to_rfc3339(),
        events_since_last_rev: 0,
        synthesis_model,
        synthesis_cost_cents: output.cost_cents,
    };

    Ok(ProducedTopicCard {
        cost_cents: output.cost_cents,
        payload,
    })
}

fn validate_output(output: &RewriteOutput) -> Result<(), ProducerError> {
    let len = output.synthesis_markdown.len();
    if len < TOPIC_CARD_SYNTHESIS_MIN_BYTES {
        return Err(ProducerError::SynthesisBelowFloor {
            len,
            min: TOPIC_CARD_SYNTHESIS_MIN_BYTES,
        });
    }
    if len > TOPIC_CARD_SYNTHESIS_MAX_BYTES {
        return Err(ProducerError::SynthesisAboveCeiling {
            len,
            max: TOPIC_CARD_SYNTHESIS_MAX_BYTES,
        });
    }
    if output.open_questions.len() > TOPIC_CARD_OPEN_QUESTIONS_MAX {
        return Err(ProducerError::InvalidField(format!(
            "open_questions len ({}) exceeds max ({})",
            output.open_questions.len(),
            TOPIC_CARD_OPEN_QUESTIONS_MAX
        )));
    }
    if !(0.0..=1.0).contains(&output.confidence) {
        return Err(ProducerError::InvalidField(format!(
            "confidence ({}) outside [0.0, 1.0]",
            output.confidence
        )));
    }
    Ok(())
}

fn enrich_contradictions(output: &RewriteOutput, at_rev: u32) -> Vec<Contradiction> {
    output
        .contradictions
        .iter()
        .map(|raw| Contradiction {
            kind: raw.kind,
            evidence_a: raw.evidence_a.clone(),
            evidence_b: raw.evidence_b.clone(),
            surfaced_at_rev: at_rev,
            status: ContradictionStatus::Open,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::summariser::{
        Summariser, SummariserError as SE, SummariserKind as SK, SummariserRequest,
        SummariserResult,
    };
    use cortex_core::events::EvidenceKind;
    use std::sync::Arc;

    struct CannedSummariser {
        text: String,
        cost: u32,
    }

    #[async_trait::async_trait]
    impl Summariser for CannedSummariser {
        fn kind(&self) -> SK {
            SK::Haiku45
        }
        async fn summarise(&self, _r: SummariserRequest) -> Result<SummariserResult, SE> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: self.cost,
                kind: SK::Haiku45,
                input_tokens: 100,
                output_tokens: 100,
            })
        }
    }

    fn synthesiser(text: &str, cost: u32) -> TopicCardSynthesiser {
        TopicCardSynthesiser::new(Arc::new(CannedSummariser {
            text: text.to_string(),
            cost,
        }))
    }

    fn valid_json(synthesis: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "synthesis_markdown": synthesis,
            "contradictions": [],
            "open_questions": ["anything?"],
            "confidence": 0.85
        }))
        .unwrap()
    }

    fn base_input() -> ProduceInput {
        ProduceInput {
            topic_slug: "auth-rewrite".into(),
            repo_scope: "cortex".into(),
            existing_card: None,
            all_evidence: vec![EvidenceRef {
                kind: EvidenceKind::Decision,
                id: "DEC-001".into(),
                weight: None,
                cited_at_rev: 1,
            }],
            new_evidence_text: "- DEC-001: adopt JWT".into(),
            superseded_evidence_text: "(none)".into(),
            force_deep: false,
        }
    }

    #[tokio::test]
    async fn first_emit_sets_revision_to_one() {
        let synth = synthesiser(&valid_json(&"x".repeat(250)), 10);
        let result = produce(&base_input(), &synth).await.expect("produce");
        assert_eq!(result.payload.revision, 1);
        assert_eq!(result.payload.events_since_last_rev, 0);
    }

    #[tokio::test]
    async fn rewrite_increments_revision_monotonically() {
        use cortex_core::events::derive_topic_card_id;
        let existing = TopicCardPayload {
            topic_card_id: derive_topic_card_id("auth-rewrite", "cortex"),
            topic_slug: "auth-rewrite".into(),
            repos: vec!["cortex".into()],
            revision: 3,
            synthesis_markdown: "x".repeat(250),
            evidence: vec![],
            contradictions: vec![],
            open_questions: vec![],
            related_topic_ids: vec![],
            confidence: 0.9,
            last_rev_at: "2026-01-01T00:00:00Z".into(),
            events_since_last_rev: 5,
            synthesis_model: "claude-haiku-4-5".into(),
            synthesis_cost_cents: 5,
        };
        let mut input = base_input();
        input.existing_card = Some(existing);
        let synth = synthesiser(&valid_json(&"y".repeat(250)), 12);
        let result = produce(&input, &synth).await.expect("produce");
        assert_eq!(
            result.payload.revision, 4,
            "must increment from prior revision"
        );
        assert_eq!(result.payload.events_since_last_rev, 0, "reset on rewrite");
    }

    #[tokio::test]
    async fn evidence_accumulates_from_input() {
        let evidence = vec![
            EvidenceRef {
                kind: EvidenceKind::Decision,
                id: "DEC-001".into(),
                weight: None,
                cited_at_rev: 1,
            },
            EvidenceRef {
                kind: EvidenceKind::Consolidation,
                id: "CONS-001".into(),
                weight: None,
                cited_at_rev: 1,
            },
        ];
        let mut input = base_input();
        input.all_evidence = evidence.clone();
        let synth = synthesiser(&valid_json(&"z".repeat(250)), 5);
        let result = produce(&input, &synth).await.expect("produce");
        assert_eq!(result.payload.evidence.len(), 2);
        assert_eq!(result.payload.evidence[0].id, "DEC-001");
        assert_eq!(result.payload.evidence[1].id, "CONS-001");
    }

    #[tokio::test]
    async fn synthesis_below_floor_is_rejected() {
        let too_short = "x".repeat(50); // below 200
        let synth = synthesiser(&valid_json(&too_short), 5);
        let err = produce(&base_input(), &synth)
            .await
            .expect_err("below floor");
        assert!(matches!(err, ProducerError::SynthesisBelowFloor { .. }));
    }

    #[tokio::test]
    async fn idempotent_topic_card_id_across_runs() {
        let synth1 = synthesiser(&valid_json(&"a".repeat(250)), 5);
        let synth2 = synthesiser(&valid_json(&"b".repeat(250)), 7);
        let r1 = produce(&base_input(), &synth1).await.expect("run1");
        let r2 = produce(&base_input(), &synth2).await.expect("run2");
        assert_eq!(
            r1.payload.topic_card_id, r2.payload.topic_card_id,
            "id must be deterministic across runs for same slug+repo"
        );
    }

    #[tokio::test]
    async fn cost_passthrough() {
        let synth = synthesiser(&valid_json(&"x".repeat(250)), 42);
        let result = produce(&base_input(), &synth).await.expect("produce");
        assert_eq!(result.cost_cents, 42);
        assert_eq!(result.payload.synthesis_cost_cents, 42);
    }

    #[tokio::test]
    async fn contradictions_are_enriched_with_open_status_and_rev() {
        let json_with_contradiction = serde_json::to_string(&serde_json::json!({
            "synthesis_markdown": "x".repeat(250),
            "contradictions": [{
                "kind": "decision_supersession",
                "evidence_a": "DEC-001",
                "evidence_b": "DEC-002"
            }],
            "open_questions": [],
            "confidence": 0.8
        }))
        .unwrap();
        let synth = synthesiser(&json_with_contradiction, 10);
        let result = produce(&base_input(), &synth).await.expect("produce");
        assert_eq!(result.payload.contradictions.len(), 1);
        assert_eq!(
            result.payload.contradictions[0].status,
            ContradictionStatus::Open
        );
        assert_eq!(result.payload.contradictions[0].surfaced_at_rev, 1);
    }

    #[tokio::test]
    async fn missing_required_field_returns_parse_error() {
        // confidence missing → serde error → SynthesiserError::Parse → ProducerError::Synthesiser
        let bad_json = r#"{"synthesis_markdown": "valid text that is long enough to pass the floor check and actually exceeds two hundred bytes to be safe and provide a proper test case here", "contradictions": [], "open_questions": []}"#;
        let synth = synthesiser(bad_json, 5);
        let err = produce(&base_input(), &synth)
            .await
            .expect_err("missing field");
        assert!(
            matches!(err, ProducerError::Synthesiser(_)),
            "expected Synthesiser error, got {err:?}"
        );
    }
}
