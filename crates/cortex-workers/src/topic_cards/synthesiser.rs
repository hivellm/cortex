//! Phase11r §2.3 — topic-card synthesiser.
//!
//! Wraps a [`crate::consolidator::summariser::Summariser`] (composition over
//! duplication — no new abstract trait) and orchestrates the rewrite loop:
//! render prompt → call summariser → strip JSON fences → deserialise.

use serde::Deserialize;

use crate::consolidator::summariser::{
    Summariser, SummariserError, SummariserKind, SummariserRequest,
};

use super::templates::{render_rewrite_prompt, RewriteSlots};

/// Deserialised model output for a topic-card rewrite.
#[derive(Debug, Clone, Deserialize)]
pub struct RewriteOutput {
    /// 200-4 000 byte Markdown prose summary.
    pub synthesis_markdown: String,
    /// Raw contradictions from the model (before the producer stamps
    /// `surfaced_at_rev` and `status`).
    #[serde(default)]
    pub contradictions: Vec<RawContradiction>,
    /// Open questions surfaced by the model (≤ 8 items).
    #[serde(default)]
    pub open_questions: Vec<String>,
    /// Model confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Realised cost in USD cents, propagated from the summariser result.
    #[serde(skip)]
    pub cost_cents: u32,
}

/// Raw contradiction as the model emits it, before the producer enriches it
/// with `surfaced_at_rev` and `status`.
#[derive(Debug, Clone, Deserialize)]
pub struct RawContradiction {
    /// Detector class (must deserialise as the snake_case tags used in
    /// [`cortex_core::events::ContradictionKind`]).
    pub kind: cortex_core::events::ContradictionKind,
    /// Evidence id A.
    pub evidence_a: String,
    /// Evidence id B.
    pub evidence_b: String,
}

/// Errors surfaced by the synthesiser layer.
#[derive(Debug, thiserror::Error)]
pub enum SynthesiserError {
    /// The underlying summariser returned an error.
    #[error("summariser: {0}")]
    Summariser(#[from] SummariserError),
    /// The model output could not be deserialised into [`RewriteOutput`].
    #[error("parse error: {0}")]
    Parse(String),
}

/// Topic-card synthesiser — wraps a [`Summariser`] impl.
pub struct TopicCardSynthesiser {
    summariser: std::sync::Arc<dyn Summariser>,
}

impl TopicCardSynthesiser {
    /// Build a synthesiser backed by the given summariser.
    pub fn new(summariser: std::sync::Arc<dyn Summariser>) -> Self {
        Self { summariser }
    }

    /// Which model backs this synthesiser.
    pub fn summariser_kind(&self) -> SummariserKind {
        self.summariser.kind()
    }

    /// Run a rewrite: render the template, call the summariser, parse the output.
    pub async fn rewrite(&self, slots: RewriteSlots<'_>) -> Result<RewriteOutput, SynthesiserError> {
        let prompt = render_rewrite_prompt(&slots);
        let request = SummariserRequest {
            prompt,
            max_output_tokens: Some(2_048),
        };
        let result = self.summariser.summarise(request).await?;
        let cost_cents = result.cost_cents;
        let text = strip_json_fences(&result.text);
        let mut output: RewriteOutput = serde_json::from_str(text).map_err(|e| {
            SynthesiserError::Parse(format!("JSON deserialise failed: {e}; body={text:?}"))
        })?;
        output.cost_cents = cost_cents;
        Ok(output)
    }
}

/// Strip leading/trailing ` ```json ` and ` ``` ` code fences from a model
/// response. Returns a `&str` slice into `s` (or `s` itself when no fence is
/// present).
fn strip_json_fences(s: &str) -> &str {
    let trimmed = s.trim();
    // Pattern: ```json\n<body>\n```
    if let Some(after_open) = trimmed.strip_prefix("```json") {
        let body = after_open.trim_start_matches('\n').trim_start_matches(' ');
        if let Some(stripped) = body.strip_suffix("```") {
            return stripped.trim_end_matches('\n').trim_end_matches(' ');
        }
        return body;
    }
    // Pattern: ```\n<body>\n```
    if let Some(after_open) = trimmed.strip_prefix("```") {
        let body = after_open.trim_start_matches('\n').trim_start_matches(' ');
        if let Some(stripped) = body.strip_suffix("```") {
            return stripped.trim_end_matches('\n').trim_end_matches(' ');
        }
        return body;
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidator::summariser::{SummariserResult, SummariserKind as SK};
    use std::sync::Arc;

    struct CannedSummariser {
        kind: SK,
        text: String,
        cost: u32,
    }

    #[async_trait::async_trait]
    impl Summariser for CannedSummariser {
        fn kind(&self) -> SK {
            self.kind
        }
        async fn summarise(
            &self,
            _req: SummariserRequest,
        ) -> Result<SummariserResult, SummariserError> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: self.cost,
                kind: self.kind,
                input_tokens: 100,
                output_tokens: 100,
            })
        }
    }

    fn valid_json_response(synthesis: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "synthesis_markdown": synthesis,
            "contradictions": [],
            "open_questions": ["is this correct?"],
            "confidence": 0.8
        }))
        .unwrap()
    }

    fn default_slots() -> RewriteSlots<'static> {
        RewriteSlots {
            topic_slug: "auth-rewrite",
            repo_scope: "cortex",
            existing_synthesis: "(no prior synthesis)",
            existing_revision: "0",
            new_evidence_block: "- DEC-001: Adopt JWT",
            superseded_evidence_block: "(none)",
        }
    }

    #[tokio::test]
    async fn rewrite_succeeds_with_valid_json_response() {
        let synthesis = "x".repeat(250);
        let summariser = Arc::new(CannedSummariser {
            kind: SK::Haiku45,
            text: valid_json_response(&synthesis),
            cost: 10,
        });
        let tc = TopicCardSynthesiser::new(summariser);
        let output = tc.rewrite(default_slots()).await.expect("rewrite");
        assert_eq!(output.synthesis_markdown, synthesis);
        assert_eq!(output.cost_cents, 10);
        assert!((output.confidence - 0.8).abs() < 0.001);
    }

    #[tokio::test]
    async fn rewrite_strips_json_code_fences() {
        let synthesis = "y".repeat(250);
        let body = format!(
            "```json\n{}\n```",
            serde_json::to_string(&serde_json::json!({
                "synthesis_markdown": synthesis,
                "contradictions": [],
                "open_questions": [],
                "confidence": 0.9
            }))
            .unwrap()
        );
        let summariser = Arc::new(CannedSummariser {
            kind: SK::Haiku45,
            text: body,
            cost: 5,
        });
        let tc = TopicCardSynthesiser::new(summariser);
        let output = tc.rewrite(default_slots()).await.expect("fence stripped");
        assert_eq!(output.synthesis_markdown, synthesis);
    }

    #[tokio::test]
    async fn rewrite_returns_parse_error_on_missing_field() {
        let bad_json = r#"{"synthesis_markdown": "ok", "confidence": 0.5}"#; // missing contradictions ok (default), but missing confidence won't trigger — try truly broken:
        let truly_broken = r#"{"not_valid": true}"#;
        let summariser = Arc::new(CannedSummariser {
            kind: SK::Haiku45,
            text: truly_broken.into(),
            cost: 5,
        });
        let tc = TopicCardSynthesiser::new(summariser);
        let err = tc.rewrite(default_slots()).await.expect_err("parse error");
        assert!(
            matches!(err, SynthesiserError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn rewrite_propagates_cost_from_summariser() {
        let synthesis = "z".repeat(250);
        let summariser = Arc::new(CannedSummariser {
            kind: SK::Haiku45,
            text: valid_json_response(&synthesis),
            cost: 42,
        });
        let tc = TopicCardSynthesiser::new(summariser);
        let output = tc.rewrite(default_slots()).await.expect("cost propagation");
        assert_eq!(output.cost_cents, 42);
    }

    #[tokio::test]
    async fn rewrite_surfaces_summariser_error() {
        struct FailSummariser;
        #[async_trait::async_trait]
        impl Summariser for FailSummariser {
            fn kind(&self) -> SK { SK::Haiku45 }
            async fn summarise(&self, _r: SummariserRequest) -> Result<SummariserResult, SummariserError> {
                Err(SummariserError::Transport("network down".into()))
            }
        }
        let tc = TopicCardSynthesiser::new(Arc::new(FailSummariser));
        let err = tc.rewrite(default_slots()).await.expect_err("transport error");
        assert!(
            matches!(err, SynthesiserError::Summariser(SummariserError::Transport(_))),
            "expected Summariser(Transport), got {err:?}"
        );
    }

    #[test]
    fn strip_json_fences_handles_plain_json() {
        let plain = r#"{"a": 1}"#;
        assert_eq!(strip_json_fences(plain), plain);
    }

    #[test]
    fn strip_json_fences_removes_backtick_json_wrapper() {
        let fenced = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_json_fences(fenced), "{\"a\": 1}");
    }

    #[test]
    fn strip_json_fences_removes_plain_backtick_wrapper() {
        let fenced = "```\n{\"a\": 1}\n```";
        assert_eq!(strip_json_fences(fenced), "{\"a\": 1}");
    }
}
