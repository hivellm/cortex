//! Phase11j §2.6 — Decision-trace producer.
//!
//! Walks `parent_event_id` from a `Kind::Decision` envelope up to
//! [`MAX_HOPS`] ancestors. Output: one consolidation with
//! `grain = DecisionTrace` covering the chain root → decision.
//! Auto-promoted to Opus by the orchestrator (deeper grain → higher
//! fidelity threshold).

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, Envelope,
    Kind, TimeSpan, CONSOLIDATION_SOURCE_IDS_INLINE_CAP,
};

use crate::summariser::{Summariser, SummariserRequest};
use crate::templates::Template;

use super::{validate_produced, ProducedConsolidation, ProducerError};

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
    /// Repo slug the chain lives in.
    pub repo: Option<String>,
}

impl DecisionTraceInput {
    /// Validate the input shape before invoking the summariser.
    pub fn ensure_well_formed(&self) -> Result<(), ProducerError> {
        if self.chain.len() > MAX_HOPS {
            return Err(ProducerError::EmptyInput(format!(
                "chain {} exceeds MAX_HOPS = {}",
                self.chain.len(),
                MAX_HOPS
            )));
        }
        if self.decision.kind != Kind::Decision {
            return Err(ProducerError::InvalidResponse(format!(
                "trigger envelope is not Kind::Decision (got {:?})",
                self.decision.kind
            )));
        }
        Ok(())
    }

    /// Pull the decision id from the trigger envelope. Falls back
    /// to the envelope id when the payload does not carry an
    /// explicit `decision_id` field.
    pub fn decision_id(&self) -> &str {
        self.decision
            .payload
            .get("decision_id")
            .and_then(|v| v.as_str())
            .unwrap_or(self.decision.event_id.as_str())
    }

    /// Render the decision-trace prompt against the chain.
    pub fn render_prompt(&self) -> String {
        let tpl = Template::for_grain(ConsolidationGrain::DecisionTrace);
        let title = self
            .decision
            .payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let status = self
            .decision
            .payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("proposed");
        let chain_text = self
            .chain
            .iter()
            .chain(std::iter::once(&self.decision))
            .map(|e| {
                format!(
                    "- {} | {:?} | {}",
                    e.event_id,
                    e.kind,
                    e.payload
                        .get("title")
                        .and_then(|v| v.as_str())
                        .or_else(|| e.payload.get("user_message").and_then(|v| v.as_str()))
                        .or_else(|| e.payload.get("rationale").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let chain_hops = self.chain.len().to_string();
        tpl.render([
            ("decision_id", self.decision_id()),
            ("decision_title", title),
            ("decision_status", status),
            ("repo", self.repo.as_deref().unwrap_or("—")),
            ("decided_at", self.decision.occurred_at.as_str()),
            ("chain_hops", chain_hops.as_str()),
            ("source_chain", chain_text.as_str()),
        ])
    }

    /// Earliest / latest `occurred_at` across chain + decision.
    pub fn temporal_bounds_ms(&self) -> Option<(i64, i64)> {
        let mut iter = self
            .chain
            .iter()
            .chain(std::iter::once(&self.decision))
            .filter_map(|e| {
                chrono::DateTime::parse_from_rfc3339(&e.occurred_at)
                    .ok()
                    .map(|d| d.timestamp_millis())
            });
        let first = iter.next()?;
        Some(iter.fold((first, first), |(lo, hi), ts| (lo.min(ts), hi.max(ts))))
    }
}

#[derive(Debug, serde::Deserialize)]
struct ModelResponse {
    title: String,
    summary_markdown: String,
    #[serde(default)]
    takeaways: Vec<String>,
}

fn strip_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_start();
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

fn parse_response(raw: &str) -> Result<ModelResponse, ProducerError> {
    serde_json::from_str(strip_fence(raw)).map_err(|err| {
        ProducerError::InvalidResponse(format!(
            "decision-trace response did not match {{title, summary_markdown, takeaways}}: {err}"
        ))
    })
}

/// Phase11j §2.6 — full pipeline.
pub async fn produce(
    input: &DecisionTraceInput,
    summariser: &dyn Summariser,
) -> Result<ProducedConsolidation, ProducerError> {
    input.ensure_well_formed()?;
    let prompt = input.render_prompt();
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let parsed = parse_response(&result.text)?;
    let (start_ms, end_ms) = input.temporal_bounds_ms().unwrap_or((0, 0));

    let mut source_event_ids: Vec<String> = input
        .chain
        .iter()
        .chain(std::iter::once(&input.decision))
        .map(|e| e.event_id.clone())
        .collect();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }

    let scope = ConsolidationScope::DecisionId(input.decision_id().to_string());
    let payload = ConsolidationPayload {
        consolidation_id: super::derive_consolidation_id(ConsolidationGrain::DecisionTrace, &scope),
        grain: ConsolidationGrain::DecisionTrace,
        scope,
        title: clip_title(&parsed.title),
        summary_markdown: parsed.summary_markdown,
        takeaways: parsed.takeaways,
        source_event_ids,
        source_event_count,
        model: result.kind.model_id().to_string(),
        depth: match result.kind {
            crate::summariser::SummariserKind::Haiku45 => ConsolidationDepth::Shallow,
            crate::summariser::SummariserKind::Opus47 => ConsolidationDepth::Deep,
        },
        outcome_distribution: Default::default(),
        temporal_span: TimeSpan {
            start_ms,
            end_ms,
            duration_ms: (end_ms - start_ms).max(0),
        },
        repos: input.repo.clone().into_iter().collect(),
        tags: Vec::new(),
    };
    validate_produced(&payload)?;
    Ok(ProducedConsolidation {
        payload,
        cost_cents: result.cost_cents,
    })
}

fn clip_title(raw: &str) -> String {
    raw.chars()
        .take(cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summariser::{
        SummariserError, SummariserKind, SummariserRequest as Req, SummariserResult,
    };
    use cortex_core::events::{Context, Stream};

    fn ctx() -> Context {
        Context {
            repo: Some("cortex".into()),
            branch: None,
            commit: None,
            cwd: None,
            user: None,
            platform: "linux".into(),
            ide: None,
            extras: Default::default(),
        }
    }

    fn envelope(idx: u8, kind: Kind, ts: &str, payload: serde_json::Value) -> Envelope {
        Envelope {
            event_id: format!("01HXEVT{idx:019}"),
            schema_version: "1".into(),
            occurred_at: ts.into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: Stream::Live,
            tool: "cortex-cli".into(),
            model: None,
            kind,
            context: ctx(),
            payload,
            redactions: vec![],
            content_hash: "sha256:".to_string()
                + "0000000000000000000000000000000000000000000000000000000000000000",
            parent_event_id: None,
        }
    }

    struct StubSummariser {
        text: String,
        kind: SummariserKind,
    }
    #[async_trait::async_trait]
    impl Summariser for StubSummariser {
        fn kind(&self) -> SummariserKind {
            self.kind
        }
        async fn summarise(&self, _req: Req) -> Result<SummariserResult, SummariserError> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: 5_000,
                kind: self.kind,
                input_tokens: 100,
                output_tokens: 400,
            })
        }
    }

    #[test]
    fn ensure_well_formed_rejects_chain_over_max_hops() {
        let input = DecisionTraceInput {
            decision: envelope(
                0,
                Kind::Decision,
                "2026-04-20T10:00:00Z",
                serde_json::json!({}),
            ),
            chain: (0..(MAX_HOPS as u8 + 1))
                .map(|i| {
                    envelope(
                        i + 1,
                        Kind::Turn,
                        "2026-04-20T09:00:00Z",
                        serde_json::json!({}),
                    )
                })
                .collect(),
            repo: None,
        };
        let err = input.ensure_well_formed().expect_err("over cap");
        assert!(format!("{err}").contains("MAX_HOPS"));
    }

    #[test]
    fn ensure_well_formed_rejects_non_decision_trigger() {
        let input = DecisionTraceInput {
            decision: envelope(0, Kind::Turn, "2026-04-20T10:00:00Z", serde_json::json!({})),
            chain: vec![],
            repo: None,
        };
        let err = input.ensure_well_formed().expect_err("not a decision");
        assert!(format!("{err}").contains("Kind::Decision"));
    }

    #[test]
    fn decision_id_falls_back_to_event_id_when_payload_field_missing() {
        let input = DecisionTraceInput {
            decision: envelope(
                7,
                Kind::Decision,
                "2026-04-20T10:00:00Z",
                serde_json::json!({}),
            ),
            chain: vec![],
            repo: None,
        };
        assert_eq!(input.decision_id(), "01HXEVT0000000000000000007");
    }

    #[tokio::test]
    async fn produce_emits_decision_trace_grain_with_opus_depth() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "Adopt Meilisearch over Lexum",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["meili exposes filter grammar", "lexum was non-standard SQL"]
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Opus47,
        };
        let input = DecisionTraceInput {
            decision: envelope(
                9,
                Kind::Decision,
                "2026-04-20T11:00:00Z",
                serde_json::json!({"decision_id": "DEC-0042", "title": "Adopt Meilisearch", "status": "accepted"}),
            ),
            chain: vec![
                envelope(
                    1,
                    Kind::Turn,
                    "2026-04-20T09:00:00Z",
                    serde_json::json!({"user_message": "evaluate keyword backends"}),
                ),
                envelope(
                    2,
                    Kind::Turn,
                    "2026-04-20T09:30:00Z",
                    serde_json::json!({"user_message": "lexum filter grammar review"}),
                ),
            ],
            repo: Some("cortex".into()),
        };
        let produced = produce(&input, &summariser).await.expect("produce");
        assert_eq!(produced.payload.grain, ConsolidationGrain::DecisionTrace);
        assert_eq!(
            produced.payload.scope,
            ConsolidationScope::DecisionId("DEC-0042".into())
        );
        assert_eq!(produced.payload.depth, ConsolidationDepth::Deep);
        assert_eq!(produced.payload.source_event_count, 3);
        assert!(produced.payload.repos.contains(&"cortex".to_string()));
        assert_eq!(produced.cost_cents, 5_000);
    }
}
