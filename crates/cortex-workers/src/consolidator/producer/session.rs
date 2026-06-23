//! Phase11j §2.4 — Session producer.
//!
//! Input: every envelope sharing a `session_id` (Turn / ToolCall /
//! AgentCall in occurred_at order). Output: one
//! `Kind::Consolidation` payload with `grain = Session`. The producer
//! renders the session template, runs it through a
//! [`super::super::summariser::Summariser`], parses the JSON response, and
//! returns a fully-shaped + validated [`super::ProducedConsolidation`].

use std::collections::BTreeMap;

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, Envelope,
    Kind, TimeSpan, CONSOLIDATION_SOURCE_IDS_INLINE_CAP,
};
use serde::Deserialize;

use super::super::summariser::{Summariser, SummariserRequest};
use super::super::templates::Template;

use super::{validate_produced, ProducedConsolidation, ProducerError};

/// Minimum combined char count across `user_message +
/// assistant_message` for a `Kind::Turn` envelope to count as
/// usable input. Single-char pings / adapter shadows fall below.
pub const USABLE_TURN_MIN_CHARS: usize = 32;

/// Minimum number of usable Turns required to trigger an LLM
/// call. Sessions with `1..USABLE_MIN_TURNS` Turn envelopes are
/// rejected unless they also carry tool calls.
pub const USABLE_MIN_TURNS: usize = 2;

/// Alternate gate — sessions with at least this many
/// `Kind::ToolCall` envelopes are accepted regardless of Turn
/// count (auto-generated agent sessions carry mostly tool work).
pub const USABLE_MIN_TOOLCALLS: usize = 3;

/// Lowercased substrings the model emits when it concludes the
/// session was empty. Producer rejects responses matching any of
/// these so the consolidations index never carries "Empty
/// session" / "No-op session" / "Zero-turn" garbage.
const GARBAGE_TITLE_MARKERS: &[&str] = &[
    "empty session",
    "no-op session",
    "no op session",
    "zero-turn",
    "zero turn",
    "no turns",
    "no substantive",
    "no substantial",
    "no work recorded",
    "no work performed",
    "no work was performed",
    "no activity recorded",
    "session incomplete",
    "session initialization only",
    "session data incomplete",
    "unable to summarize",
    "unable to summarise",
    "cannot be meaningfully summarized",
    "cannot be meaningfully summarised",
    "no actionable output",
    "no actionable content",
    "session created but never",
];

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
    /// Quick sanity check the orchestrator runs before invoking
    /// the summariser. Rejects sessions whose envelope set carries
    /// no consolidatable conversational content so the nightly
    /// back-fill stops emitting "no substantive output" replies.
    ///
    /// 2026-05-19 — sessions whose envelopes are all
    /// classifier-prompt / memory / system events (no
    /// `Kind::Turn` carrying a non-empty user_message /
    /// assistant_message, no `Kind::ToolCall` with a tool name,
    /// no `Kind::AgentCall` with an agent_type) poisoned the
    /// consolidations index with hundreds of "Session incomplete"
    /// model replies. The filter below short-circuits them
    /// before any prompt rendering.
    pub fn ensure_non_empty(&self) -> Result<(), ProducerError> {
        if self.envelopes.is_empty() {
            return Err(ProducerError::EmptyInput(format!(
                "session {} has zero envelopes",
                self.session_id
            )));
        }
        if self.usable_envelopes().next().is_none() {
            return Err(ProducerError::EmptyInput(format!(
                "session {} has no Turn/ToolCall/AgentCall envelopes with real content",
                self.session_id
            )));
        }
        if !self.has_substantive_content() {
            return Err(ProducerError::EmptyInput(format!(
                "session {} below substance floor (need >= {} Turns or >= {} ToolCalls)",
                self.session_id, USABLE_MIN_TURNS, USABLE_MIN_TOOLCALLS
            )));
        }
        Ok(())
    }

    /// Iterator over envelopes that actually carry consolidatable
    /// work: a `Kind::Turn` whose combined `user_message +
    /// assistant_message` is at least
    /// [`USABLE_TURN_MIN_CHARS`] characters of real text, a
    /// `Kind::ToolCall` with a tool name, or a `Kind::AgentCall`
    /// with an `agent_type`. Other kinds (Memory, classifier-prompt
    /// shadows, Decision echoes, etc.) carry no value the session
    /// producer can fold into a summary and are dropped.
    ///
    /// 2026-05-19 — single-char Turns (`user_message = "x"`,
    /// adapter ping fragments) passed the previous "non-empty"
    /// gate but produced no useful prompt content; the model
    /// dutifully replied "Empty session — no work recorded". The
    /// minimum-char floor stops that class at the source.
    pub fn usable_envelopes(&self) -> impl Iterator<Item = &Envelope> {
        self.envelopes.iter().filter(|e| match e.kind {
            Kind::Turn => {
                let user = e
                    .payload
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let asst = e
                    .payload
                    .get("assistant_message")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let combined = user.chars().count() + asst.chars().count();
                combined >= USABLE_TURN_MIN_CHARS
            }
            Kind::ToolCall => e
                .payload
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            Kind::AgentCall => e
                .payload
                .get("agent_type")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            _ => false,
        })
    }

    /// True when the source set has enough substance for the LLM
    /// to produce a non-trivial summary: at least
    /// [`USABLE_MIN_TURNS`] usable Turns OR at least
    /// [`USABLE_MIN_TOOLCALLS`] tool calls. Sessions below both
    /// thresholds are rejected before any prompt is rendered.
    pub fn has_substantive_content(&self) -> bool {
        let mut turn_count = 0usize;
        let mut toolcall_count = 0usize;
        for e in self.usable_envelopes() {
            match e.kind {
                Kind::Turn => turn_count += 1,
                Kind::ToolCall => toolcall_count += 1,
                _ => {}
            }
        }
        turn_count >= USABLE_MIN_TURNS || toolcall_count >= USABLE_MIN_TOOLCALLS
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

    /// 2026-05-19 — short pre-flight prompt asking the model
    /// whether the session is substantive enough to be worth
    /// consolidating. Output contract: a single JSON object
    /// `{"relevant": true|false, "reason": "<one line>"}`.
    /// Catches sessions that clear the static substance floor
    /// (`USABLE_MIN_TURNS`/`USABLE_MIN_TOOLCALLS`) but still
    /// contain marginal work (test pings, hello-world prompts,
    /// debug noise, single-question lookups) the operator does
    /// not want pinned to the consolidations index.
    pub fn render_relevance_prompt(&self) -> String {
        let turn_count = self
            .envelopes
            .iter()
            .filter(|e| e.kind == Kind::Turn)
            .count();
        let tool_count = self
            .envelopes
            .iter()
            .filter(|e| e.kind == Kind::ToolCall)
            .count();
        let preview = self
            .usable_envelopes()
            .take(6)
            .map(|e| {
                let snippet = match e.kind {
                    Kind::Turn => e
                        .payload
                        .get("user_message")
                        .and_then(|v| v.as_str())
                        .or_else(|| e.payload.get("assistant_message").and_then(|v| v.as_str()))
                        .unwrap_or(""),
                    Kind::ToolCall => e
                        .payload
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    Kind::AgentCall => e
                        .payload
                        .get("agent_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    _ => "",
                };
                format!(
                    "- {:?}: {}",
                    e.kind,
                    snippet.chars().take(200).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "You are a relevance judge for the Cortex consolidator. Given a \
session's high-level shape, decide if it is worth pinning to the \
consolidations index. \n\n\
Worth consolidating: real engineering work (debugging, design \
decisions, code changes shipped, multi-step investigations, \
architectural choices) the operator might want to recall weeks \
later.\n\
Not worth consolidating: smoke tests, hello-world pings, single \
trivial questions answered in one turn, sessions that produced \
no durable artifact, sessions where the operator was just \
poking at the harness.\n\n\
Session {session}: {turns} Turn envelopes, {tools} ToolCall envelopes.\n\
Preview of the first 6 usable envelopes:\n{preview}\n\n\
Reply with EXACTLY one JSON object on a single line:\n\
{{\"relevant\": true|false, \"reason\": \"<one short sentence>\"}}\n\
No markdown fence, no extra prose.",
            session = self.session_id,
            turns = turn_count,
            tools = tool_count,
            preview = preview
        )
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
        // 2026-05-19 — only render envelopes that carry usable
        // content. Lists Turn with non-empty user/assistant,
        // ToolCall with tool_name + input excerpt, AgentCall with
        // agent_type + description. Classifier-prompt shadows and
        // Memory aggregations are dropped so the model never sees
        // "You are an event classifier for the Cortex system…" as
        // a turn line.
        let source_turns = self
            .usable_envelopes()
            .map(|e| {
                let body = match e.kind {
                    Kind::Turn => {
                        let user = e
                            .payload
                            .get("user_message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let asst = e
                            .payload
                            .get("assistant_message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !user.is_empty() && !asst.is_empty() {
                            format!("user={user} | asst={asst}")
                        } else if !user.is_empty() {
                            format!("user={user}")
                        } else {
                            format!("asst={asst}")
                        }
                    }
                    Kind::ToolCall => {
                        let tool = e
                            .payload
                            .get("tool_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let input_excerpt = e
                            .payload
                            .get("input")
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        format!("tool={tool} input={input_excerpt}")
                    }
                    Kind::AgentCall => {
                        let agent = e
                            .payload
                            .get("agent_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let desc = e
                            .payload
                            .get("description")
                            .and_then(|v| v.as_str())
                            .or_else(|| e.payload.get("prompt").and_then(|v| v.as_str()))
                            .unwrap_or("");
                        format!("agent={agent} desc={desc}")
                    }
                    _ => String::new(),
                };
                format!(
                    "- {} | {:?} | {}",
                    e.event_id,
                    e.kind,
                    body.chars().take(240).collect::<String>()
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

/// Returns `true` when the model's title or summary contains any
/// of [`GARBAGE_TITLE_MARKERS`] (case-insensitive). Used to drop
/// "Empty session — no work recorded" replies that survived the
/// upstream filter because the input had token-thin Turn payloads.
fn is_garbage_summary(title: &str, summary: &str) -> bool {
    let title_lc = title.to_lowercase();
    let summary_lc = summary.to_lowercase();
    GARBAGE_TITLE_MARKERS
        .iter()
        .any(|marker| title_lc.contains(marker) || summary_lc.contains(marker))
}

/// 2026-05-19 — model verdict shape for the relevance gate.
#[derive(Debug, Deserialize)]
struct RelevanceVerdictRaw {
    relevant: bool,
    #[serde(default)]
    reason: String,
}

/// Parsed verdict the producer reads.
#[derive(Debug)]
pub struct RelevanceVerdict {
    /// `true` when the model judges the session worth consolidating.
    pub relevant: bool,
    /// Short rationale the model gave; surfaces in the producer's
    /// skip log so operators can audit upstream judgments.
    pub reason: String,
}

/// Parse the relevance verdict. Strips a markdown fence if the
/// model wrapped the JSON object in one. Defaults to
/// `relevant=false` when the model output cannot be parsed — fail
/// closed so a garbage parse never leaks through as a green light.
fn parse_relevance(raw: &str) -> Result<RelevanceVerdict, ProducerError> {
    let body = strip_code_fence(raw);
    let parsed: RelevanceVerdictRaw = serde_json::from_str(body).map_err(|err| {
        ProducerError::InvalidResponse(format!(
            "relevance verdict did not match {{relevant, reason}}: {err}; body={body}"
        ))
    })?;
    Ok(RelevanceVerdict {
        relevant: parsed.relevant,
        reason: parsed.reason,
    })
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
    // 2026-05-19 — LLM relevance gate. Before spending the cost of
    // the full summary, ask the model a cheap yes/no: is this
    // session substantive enough to be worth consolidating? Skip
    // the full pass when the model says NO. Stops the
    // consolidations index from filling up with marginal work the
    // substance floor can't catch (sessions whose Turns clear the
    // 32-char floor but still amount to nothing useful: hello-world
    // pings, single-line "test" prompts, debug noise).
    let relevance_prompt = input.render_relevance_prompt();
    let relevance = summariser
        .summarise(SummariserRequest {
            prompt: relevance_prompt,
            max_output_tokens: None,
        })
        .await?;
    let verdict = parse_relevance(&relevance.text)?;
    if !verdict.relevant {
        return Err(ProducerError::EmptyInput(format!(
            "session {} judged not worth consolidating by relevance gate: {}",
            input.session_id, verdict.reason
        )));
    }
    let prompt = input.render_prompt();
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let parsed = parse_model_response(&result.text)?;
    // 2026-05-19 — reject the response if the model concluded the
    // session was empty (`Empty session`, `Zero-turn`, `No-op`,
    // `Session incomplete`, etc.). Some sessions clear
    // `ensure_non_empty` because they carry token-thin Turns but
    // still resolve to garbage downstream; this gate stops the
    // garbage at the producer rather than letting it land in the
    // consolidations index.
    if is_garbage_summary(&parsed.title, &parsed.summary_markdown) {
        return Err(ProducerError::InvalidResponse(format!(
            "model concluded session {} has no substantive content (title={:?})",
            input.session_id, parsed.title
        )));
    }
    let (start_ms, end_ms) = input.temporal_bounds_ms().unwrap_or((0, 0));

    // 2026-05-19 — count only the envelopes the model actually
    // received in the prompt. Inflating source_event_count with
    // classifier-shadow / memory rows misleads the dashboard's
    // "Source events folded" panel.
    let mut source_event_ids: Vec<String> = input
        .usable_envelopes()
        .map(|e| e.event_id.clone())
        .collect();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }

    let repos: Vec<String> = input.repo.clone().into_iter().collect();
    let scope = ConsolidationScope::SessionId(input.session_id.clone());
    let payload = ConsolidationPayload {
        consolidation_id: super::derive_consolidation_id(ConsolidationGrain::Session, &scope),
        grain: ConsolidationGrain::Session,
        scope,
        title: clip_title(&parsed.title),
        summary_markdown: parsed.summary_markdown,
        takeaways: parsed.takeaways,
        source_event_ids,
        source_event_count,
        model: result.kind.model_id().to_string(),
        depth: match result.kind {
            super::super::summariser::SummariserKind::Haiku45 => ConsolidationDepth::Shallow,
            super::super::summariser::SummariserKind::Opus47 => ConsolidationDepth::Deep,
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
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
    })
}

fn clip_title(raw: &str) -> String {
    raw.chars()
        .take(cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::super::summariser::{
        SummariserError, SummariserKind, SummariserRequest as Req, SummariserResult,
    };
    use super::*;
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
        // Carry enough content per turn to clear the
        // `USABLE_TURN_MIN_CHARS` substance floor introduced
        // 2026-05-19 (≥32 combined chars on user + assistant).
        let mut payload = serde_json::json!({
            "user_message": format!(
                "user message {idx} requesting investigation of failing test"
            ),
            "assistant_message": format!(
                "reply {idx} — patched middleware and reran the suite"
            ),
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
            class_level: None,
            class_compartments: None,
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
        async fn summarise(&self, req: Req) -> Result<SummariserResult, SummariserError> {
            // 2026-05-19 — the producer now makes two LLM calls
            // per session: a relevance gate (expects
            // `{"relevant": true, "reason": "..."}`) then the
            // full summary (expects
            // `{"title", "summary_markdown", "takeaways"}`). This
            // test double sniffs the prompt for the relevance-
            // gate's anchor string and returns a permissive
            // `relevant: true` so existing tests keep exercising
            // the full summary path.
            let text = if req
                .prompt
                .contains("relevance judge for the Cortex consolidator")
            {
                "{\"relevant\": true, \"reason\": \"substantive session\"}".to_string()
            } else {
                self.text.clone()
            };
            Ok(SummariserResult {
                text,
                cost_cents: 1,
                kind: self.kind,
                input_tokens: 10,
                output_tokens: 200,
            })
        }
    }

    /// Variant — the relevance gate returns false. Used to test
    /// that produce() short-circuits without calling the summary
    /// LLM.
    struct GatedOutSummariser {
        kind: SummariserKind,
    }

    #[async_trait::async_trait]
    impl Summariser for GatedOutSummariser {
        fn kind(&self) -> SummariserKind {
            self.kind
        }
        async fn summarise(&self, req: Req) -> Result<SummariserResult, SummariserError> {
            assert!(
                req.prompt
                    .contains("relevance judge for the Cortex consolidator"),
                "summary LLM call MUST be skipped when relevance gate says no"
            );
            Ok(SummariserResult {
                text: "{\"relevant\": false, \"reason\": \"smoke test, no durable work\"}"
                    .to_string(),
                cost_cents: 1,
                kind: self.kind,
                input_tokens: 10,
                output_tokens: 50,
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

    /// 2026-05-19 regression — sessions whose envelopes are all
    /// `Kind::Turn` with empty user/assistant fields (the shape
    /// the classifier-prompt shadows produced) MUST be rejected
    /// before reaching the LLM. Otherwise the consolidations
    /// index fills up with "Session incomplete — no substantive
    /// output captured" replies.
    #[test]
    fn ensure_non_empty_rejects_turns_with_empty_user_and_assistant() {
        let mut env = turn_envelope(1, "2026-04-20T10:00:00Z", None);
        env.payload = serde_json::json!({
            "user_message": "",
            "assistant_message": null,
            "tokens": null,
            "tool_call_event_ids": []
        });
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![env],
        };
        let err = input
            .ensure_non_empty()
            .expect_err("turn with empty payload must be rejected");
        assert!(format!("{err}").contains("no Turn/ToolCall/AgentCall"));
    }

    /// 2026-05-19 regression — single-char Turn payloads
    /// (`user_message = "x"`) fell below the substance floor and
    /// MUST be rejected pre-LLM.
    #[test]
    fn ensure_non_empty_rejects_token_thin_turn_below_min_chars() {
        let mut env = turn_envelope(1, "2026-04-20T10:00:00Z", None);
        env.payload = serde_json::json!({
            "user_message": "x",
            "assistant_message": null,
            "tokens": null,
            "tool_call_event_ids": []
        });
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![env],
        };
        let err = input
            .ensure_non_empty()
            .expect_err("single-char turn must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("no Turn/ToolCall/AgentCall") || msg.contains("substance floor"),
            "got: {msg}"
        );
    }

    /// 2026-05-19 regression — one real Turn is not enough; the
    /// substance floor is 2 Turns OR 3 ToolCalls.
    #[test]
    fn ensure_non_empty_rejects_single_real_turn_without_tool_calls() {
        let mut env = turn_envelope(1, "2026-04-20T10:00:00Z", None);
        env.payload = serde_json::json!({
            "user_message": "Please fix the failing auth test in middleware module",
            "assistant_message": "Patched middleware so the JWT cache invalidates correctly.",
            "tokens": null,
            "tool_call_event_ids": []
        });
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![env],
        };
        let err = input
            .ensure_non_empty()
            .expect_err("single substantive Turn alone must still be rejected");
        assert!(format!("{err}").contains("substance floor"));
    }

    /// 2026-05-19 regression — two real Turns clear the floor.
    #[test]
    fn ensure_non_empty_accepts_two_real_turns() {
        let make_turn = |id: u8, ts: &str, user: &str, asst: &str| {
            let mut env = turn_envelope(id, ts, None);
            env.payload = serde_json::json!({
                "user_message": user,
                "assistant_message": asst,
                "tokens": null,
                "tool_call_event_ids": []
            });
            env
        };
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![
                make_turn(
                    1,
                    "2026-04-20T10:00:00Z",
                    "Investigate the failing auth tests reported by CI",
                    "Found the JWT cache returning stale entries; need invalidation.",
                ),
                make_turn(
                    2,
                    "2026-04-20T10:05:00Z",
                    "Apply the invalidation patch and rerun the suite",
                    "Patch applied, all tests passing locally.",
                ),
            ],
        };
        assert!(input.ensure_non_empty().is_ok());
    }

    /// 2026-05-19 — `is_garbage_summary` catches every shape the
    /// model emitted during the 2026-05-19 incident.
    #[test]
    fn is_garbage_summary_catches_known_shapes() {
        let cases = [
            "Empty session — no work recorded",
            "No-op session — zero turns",
            "Zero-turn session — no activity recorded",
            "Session incomplete — no substantial output captured",
            "Session data incomplete — unable to summarize",
            "Empty session — no substantive turns recorded",
            "No-op session — zero turns, no work recorded",
        ];
        for title in cases {
            assert!(is_garbage_summary(title, ""), "should flag title: {title}");
        }
        // A real summary must not trip the filter.
        assert!(!is_garbage_summary(
            "Auth refactor — JWT cache rotation",
            "Reworked the JWT middleware so token rotation completes within 250ms."
        ));
    }

    /// 2026-05-19 regression — non-conversational envelopes
    /// (Memory, Decision, etc.) MUST be dropped by
    /// `usable_envelopes`; the substantive Turns are listed in
    /// the prompt while the noise envelopes are not.
    #[test]
    fn usable_envelopes_drops_non_conversational_kinds() {
        use cortex_core::events::{Context, Stream};
        let real_turn_a = {
            let mut env = turn_envelope(1, "2026-04-20T10:00:00Z", None);
            env.payload = serde_json::json!({
                "user_message": "Investigate failing auth tests reported by CI",
                "assistant_message": "Identified the JWT cache returning stale entries.",
                "tokens": null,
                "tool_call_event_ids": []
            });
            env
        };
        let real_turn_b = {
            let mut env = turn_envelope(2, "2026-04-20T10:01:00Z", None);
            env.payload = serde_json::json!({
                "user_message": "Apply the invalidation patch and rerun the suite",
                "assistant_message": "Patch applied, all tests passing locally.",
                "tokens": null,
                "tool_call_event_ids": []
            });
            env
        };
        let memory_noise = Envelope {
            event_id: "01MEMORY".into(),
            schema_version: "1".into(),
            occurred_at: "2026-04-20T10:00:01Z".into(),
            ingested_at: None,
            session_id: "s".into(),
            stream: Stream::Live,
            tool: "claude-code".into(),
            model: None,
            kind: Kind::Memory,
            context: Context {
                repo: None,
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".into(),
                ide: None,
                extras: BTreeMap::new(),
            },
            payload: serde_json::json!({"memory_type": "preference", "body": "noise"}),
            redactions: Vec::new(),
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
            parent_event_id: None,
            class_level: None,
            class_compartments: None,
        };
        let input = SessionInput {
            session_id: "s".into(),
            repo: None,
            envelopes: vec![real_turn_a, real_turn_b, memory_noise],
        };
        let usable: Vec<_> = input.usable_envelopes().collect();
        assert_eq!(usable.len(), 2);
        assert!(usable.iter().all(|e| e.kind == Kind::Turn));
        assert!(input.ensure_non_empty().is_ok());
        let prompt = input.render_prompt();
        assert!(prompt.contains("Investigate failing auth tests"));
        assert!(prompt.contains("Apply the invalidation patch"));
        assert!(!prompt.contains("01MEMORY"));
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
        assert!(prompt.contains("user message 1"));
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
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z", None),
                turn_envelope(2, "2026-04-20T10:01:00Z", None),
            ],
        };
        let err = produce(&input, &summariser).await.expect_err("undersize");
        assert!(format!("{err}").contains("summary_markdown"));
    }

    /// 2026-05-19 — when the relevance gate returns
    /// `relevant: false`, `produce()` MUST short-circuit before
    /// rendering the full summary prompt. The test double panics
    /// if the summary call is attempted.
    #[tokio::test]
    async fn produce_short_circuits_when_relevance_gate_says_no() {
        let summariser = GatedOutSummariser {
            kind: SummariserKind::Haiku45,
        };
        let input = SessionInput {
            session_id: "01HXSESS04".into(),
            repo: Some("cortex".into()),
            envelopes: vec![
                turn_envelope(1, "2026-04-20T10:00:00Z", None),
                turn_envelope(2, "2026-04-20T10:01:00Z", None),
            ],
        };
        let err = produce(&input, &summariser).await.expect_err("gated out");
        let msg = format!("{err}");
        assert!(
            msg.contains("relevance gate") && msg.contains("smoke test"),
            "got: {msg}"
        );
    }

    /// 2026-05-19 — `parse_relevance` fails closed on garbage
    /// output. A model that ignores the JSON contract must not
    /// trigger a green light.
    #[test]
    fn parse_relevance_rejects_unparseable_output() {
        let err = parse_relevance("yes it is").expect_err("garbage must error");
        assert!(format!("{err}").contains("relevance verdict"));
    }

    /// 2026-05-19 — `parse_relevance` reads a clean true/false
    /// JSON object.
    #[test]
    fn parse_relevance_reads_well_formed_verdict() {
        let v =
            parse_relevance("{\"relevant\": true, \"reason\": \"real engineering work\"}").unwrap();
        assert!(v.relevant);
        assert_eq!(v.reason, "real engineering work");

        let v2 = parse_relevance(
            "```json\n{\"relevant\": false, \"reason\": \"hello-world ping\"}\n```",
        )
        .unwrap();
        assert!(!v2.relevant);
        assert_eq!(v2.reason, "hello-world ping");
    }
}
