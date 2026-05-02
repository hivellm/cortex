//! Phase11j §2.4 — Session producer.
//!
//! Input: every envelope sharing a `session_id` (Turn / ToolCall /
//! AgentCall in occurred_at order). Output: one
//! `Kind::Consolidation` payload with `grain = Session`. The producer
//! renders the session template, runs it through a
//! [`crate::summariser::Summariser`], parses the JSON response, and
//! returns a fully-shaped + validated [`super::ProducedConsolidation`].

use std::collections::BTreeMap;

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, Envelope,
    Kind, TimeSpan, CONSOLIDATION_SOURCE_IDS_INLINE_CAP,
};
use serde::Deserialize;

use crate::summariser::{Summariser, SummariserRequest};
use crate::templates::Template;

use super::{validate_produced, ProducedConsolidation, ProducerError};

/// Input the session producer reads. The orchestrator hydrates this
/// from the archive_loader + Synap stream replay.
#[derive(Debug, Clone)]
pub struct SessionInput {
    /// Originating session id. Drives `scope = SessionId(_)`.
    pub session_id: String,
    /// Repo slug the session ran against (for `payload.repos`).
    pub repo: Option<String>,
    /// Envelopes ordered by `occurred_at` — Turn / ToolCall /
    /// AgentCall variants only.
    pub envelopes: Vec<Envelope>,
}

impl SessionInput {
    /// Quick sanity check the orchestrator runs before invoking the
    /// summariser. Producers reject empty inputs cleanly so the
    /// nightly back-fill never emits an empty payload.
    pub fn ensure_non_empty(&self) -> Result<(), ProducerError> {
        if self.envelopes.is_empty() {
            return Err(ProducerError::EmptyInput(format!(
                "session {} has zero envelopes",
                self.session_id
            )));
        }
        Ok(())
    }

    /// Earliest / latest `occurred_at` across the envelope set, in
    /// epoch ms. Drives `temporal_span` on the produced payload.
    pub fn temporal_bounds_ms(&self) -> Option<(i64, i64)> {
        let mut iter = self.envelopes.iter().filter_map(|e| {
            chrono::DateTime::parse_from_rfc3339(&e.occurred_at)
                .ok()
                .map(|d| d.timestamp_millis())
        });
        let first = iter.next()?;
        Some(iter.fold((first, first), |(lo, hi), ts| (lo.min(ts), hi.max(ts))))
    }

    /// Outcome distribution over the source set. Reads
    /// `payload.outcome` when present; envelopes that lack the tag
    /// don't contribute.
    pub fn outcome_distribution(&self) -> BTreeMap<String, u32> {
        let mut out: BTreeMap<String, u32> = BTreeMap::new();
        for env in &self.envelopes {
            if let Some(s) = env.payload.get("outcome").and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    *out.entry(s.to_string()).or_insert(0) += 1;
                }
            }
        }
        out
    }

    /// Build the rendered prompt the summariser will receive. Pure
    /// function so unit tests can pin the wire shape.
    pub fn render_prompt(&self) -> String {
        let tpl = Template::for_grain(ConsolidationGrain::Session);
        let (start_ms, end_ms) = self.temporal_bounds_ms().unwrap_or((0, 0));
        let started = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(start_ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "—".into());
        let ended = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(end_ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "—".into());
        let outcome_summary = self
            .outcome_distribution()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let turn_count = self
            .envelopes
            .iter()
            .filter(|e| e.kind == Kind::Turn)
            .count()
            .to_string();
        let source_turns = self
            .envelopes
            .iter()
            .map(|e| {
                format!(
                    "- {} | {:?} | {}",
                    e.event_id,
                    e.kind,
                    e.payload
                        .get("user_message")
                        .and_then(|v| v.as_str())
                        .or_else(|| e.payload.get("assistant_message").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .chars()
                        .take(120)
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        tpl.render([
            ("session_id", self.session_id.as_str()),
            ("repo", self.repo.as_deref().unwrap_or("—")),
            ("started_at", started.as_str()),
            ("ended_at", ended.as_str()),
            ("turn_count", turn_count.as_str()),
            ("outcome_summary", outcome_summary.as_str()),
            ("source_turns", source_turns.as_str()),
        ])
    }
}

/// JSON shape the summariser is contracted to emit (one block per
/// template). Captured strictly so a hallucinated extra field
/// surfaces as a parse error rather than silently dropping.
#[derive(Debug, Deserialize)]
struct ModelResponse {
    title: String,
    summary_markdown: String,
    #[serde(default)]
    takeaways: Vec<String>,
}

/// Strip the ```json``` fence the model often wraps responses in.
/// Returns the original string when no fence is found.
fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_start();
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

fn parse_model_response(raw: &str) -> Result<ModelResponse, ProducerError> {
    let body = strip_code_fence(raw);
    serde_json::from_str::<ModelResponse>(body).map_err(|err| {
        ProducerError::InvalidResponse(format!(
            "model response did not match {{title, summary_markdown, takeaways}}: {err}"
        ))
    })
}

/// Phase11j §2.4 — full producer pipeline.
///
/// 1. Validate the input shape.
/// 2. Render the session template.
/// 3. Run it through the summariser.
/// 4. Parse the JSON response.
/// 5. Build + validate a [`ConsolidationPayload`].
pub async fn produce(
    input: &SessionInput,
    summariser: &dyn Summariser,
) -> Result<ProducedConsolidation, ProducerError> {
    input.ensure_non_empty()?;
    let prompt = input.render_prompt();
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let parsed = parse_model_response(&result.text)?;
    let (start_ms, end_ms) = input.temporal_bounds_ms().unwrap_or((0, 0));

    let mut source_event_ids: Vec<String> =
        input.envelopes.iter().map(|e| e.event_id.clone()).collect();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }

    let repos: Vec<String> = input.repo.clone().into_iter().collect();
    let payload = ConsolidationPayload {
        consolidation_id: ulid::Ulid::new().to_string(),
        grain: ConsolidationGrain::Session,
        scope: ConsolidationScope::SessionId(input.session_id.clone()),
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
        outcome_distribution: input.outcome_distribution(),
        temporal_span: TimeSpan {
            start_ms,
            end_ms,
            duration_ms: (end_ms - start_ms).max(0),
        },
        repos,
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
    use serde_json::Value;

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

    fn turn_envelope(idx: u8, ts: &str, outcome: Option<&str>) -> Envelope {
        let mut payload = serde_json::json!({
            "user_message": format!("user msg {idx}"),
            "assistant_message": format!("reply {idx}"),
        });
        if let Some(o) = outcome {
            payload
                .as_object_mut()
                .unwrap()
                .insert("outcome".into(), Value::String(o.into()));
        }
        Envelope {
            event_id: format!("01HXEVT{idx:019}"),
            schema_version: "1".into(),
            occurred_at: ts.into(),
            ingested_at: None,
            session_id: "01HXSESS00000000000000000A".into(),
            stream: Stream::Live,
            tool: "claude-code".into(),
            model: Some("claude-haiku-4-5".into()),
            kind: Kind::Turn,
            context: ctx(),
            payload,
            redactions: vec![],
            content_hash: "sha256:00".to_string()
                + "00000000000000000000000000000000000000000000000000000000000000",
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
                cost_cents: 1,
                kind: self.kind,
                input_tokens: 10,
                output_tokens: 200,
            })
        }
    }

    #[test]
    fn ensure_non_empty_rejects_zero_envelope_input() {
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![],
        };
        let err = input.ensure_non_empty().expect_err("empty input");
        assert!(format!("{err}").contains("zero envelopes"));
    }

    #[test]
    fn temporal_bounds_pick_min_and_max_across_envelopes() {
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z", None),
                turn_envelope(2, "2026-04-20T11:00:00Z", None),
                turn_envelope(3, "2026-04-20T09:00:00Z", None),
            ],
        };
        let (lo, hi) = input.temporal_bounds_ms().expect("non-empty");
        assert!(hi - lo > 0);
        assert_eq!(
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(lo)
                .unwrap()
                .to_rfc3339(),
            "2026-04-20T09:00:00+00:00"
        );
    }

    #[test]
    fn outcome_distribution_counts_payload_outcome_field() {
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z", Some("success")),
                turn_envelope(2, "2026-04-20T10:01:00Z", Some("success")),
                turn_envelope(3, "2026-04-20T10:02:00Z", Some("error")),
                turn_envelope(4, "2026-04-20T10:03:00Z", None),
            ],
        };
        let dist = input.outcome_distribution();
        assert_eq!(dist.get("success").copied(), Some(2));
        assert_eq!(dist.get("error").copied(), Some(1));
        assert!(!dist.contains_key(""));
    }

    #[test]
    fn render_prompt_substitutes_session_id_and_lists_source_turns() {
        let input = SessionInput {
            session_id: "01HXSESS01".into(),
            repo: Some("cortex".into()),
            envelopes: vec![turn_envelope(1, "2026-04-20T10:00:00Z", Some("success"))],
        };
        let prompt = input.render_prompt();
        assert!(prompt.contains("01HXSESS01"));
        assert!(prompt.contains("cortex"));
        assert!(prompt.contains("user msg 1"));
        assert!(prompt.contains("success=1"));
    }

    #[test]
    fn strip_code_fence_handles_json_fence_and_plain_text() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("```\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("{\"a\":1}"), "{\"a\":1}");
    }

    #[test]
    fn parse_model_response_rejects_missing_required_fields() {
        let err = parse_model_response("{\"title\":\"x\"}").expect_err("missing summary");
        assert!(format!("{err}").contains("summary_markdown"));
    }

    #[tokio::test]
    async fn produce_pipes_stub_response_into_validated_payload() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "tune ef_search",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["raise ef_search to 128", "watch recall@10 for drift"]
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let input = SessionInput {
            session_id: "01HXSESS01".into(),
            repo: Some("cortex".into()),
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z", Some("success")),
                turn_envelope(2, "2026-04-20T11:00:00Z", Some("success")),
            ],
        };
        let produced = produce(&input, &summariser).await.expect("produce");
        assert_eq!(produced.payload.grain, ConsolidationGrain::Session);
        assert_eq!(produced.payload.title, "tune ef_search");
        assert_eq!(produced.payload.takeaways.len(), 2);
        assert_eq!(produced.payload.source_event_ids.len(), 2);
        assert_eq!(produced.payload.source_event_count, 2);
        assert_eq!(produced.payload.model, "claude-haiku-4-5");
        assert_eq!(produced.payload.depth, ConsolidationDepth::Shallow);
        assert!(produced
            .payload
            .outcome_distribution
            .contains_key("success"));
        assert_eq!(produced.cost_cents, 1);
        // Temporal span is materialised.
        assert!(produced.payload.temporal_span.duration_ms > 0);
        assert_eq!(
            produced.payload.temporal_span.duration_ms,
            produced.payload.temporal_span.end_ms - produced.payload.temporal_span.start_ms
        );
    }

    #[tokio::test]
    async fn produce_clips_source_event_ids_at_inline_cap_but_keeps_count() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "huge session",
            "summary_markdown": "x".repeat(400),
            "takeaways": []
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let envelopes: Vec<Envelope> = (0u8..=255)
            .chain(0u8..=64)
            .enumerate()
            .map(|(idx, _)| {
                turn_envelope(
                    (idx % 250) as u8,
                    &format!("2026-04-20T10:{:02}:00Z", idx % 60),
                    None,
                )
            })
            .collect();
        let total = envelopes.len() as u32;
        let input = SessionInput {
            session_id: "01HXSESS02".into(),
            repo: None,
            envelopes,
        };
        let produced = produce(&input, &summariser).await.expect("produce");
        assert_eq!(
            produced.payload.source_event_ids.len(),
            CONSOLIDATION_SOURCE_IDS_INLINE_CAP
        );
        assert_eq!(produced.payload.source_event_count, total);
    }

    #[tokio::test]
    async fn produce_rejects_summary_below_floor() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "ok",
            "summary_markdown": "too short",
            "takeaways": []
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let input = SessionInput {
            session_id: "01HXSESS03".into(),
            repo: None,
            envelopes: vec![turn_envelope(1, "2026-04-20T10:00:00Z", None)],
        };
        let err = produce(&input, &summariser).await.expect_err("undersize");
        assert!(format!("{err}").contains("summary_markdown"));
    }
}
