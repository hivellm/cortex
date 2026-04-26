//! Rust types mirroring the JSON Schemas under `schemas/`.
//!
//! The schemas are the wire contract. These types are exercised against
//! every fixture under `tests/fixtures/` so neither side can drift (see
//! `tests/schema_alignment.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Top-level event envelope. Every event on the bus matches this shape plus
/// a per-[`Kind`] payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    /// ULID; 26 chars; client-generated.
    pub event_id: String,
    /// Fixed at `"1"` for this schema major version.
    pub schema_version: String,
    /// RFC 3339 timestamp from the source system.
    pub occurred_at: String,
    /// RFC 3339 timestamp set by the ingestion router.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<String>,
    /// Session ULID; adapter-owned, stable per AI session.
    pub session_id: String,
    /// `"live"` or `"bootstrap"`.
    pub stream: Stream,
    /// Adapter identifier (controlled vocab).
    pub tool: String,
    /// Model identifier; `None` for non-LLM events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Discriminator selecting the payload shape.
    pub kind: Kind,
    /// Capture-time metadata.
    pub context: Context,
    /// Kind-specific payload; validated against the matching per-kind schema.
    pub payload: Value,
    /// Opaque redaction tokens; absent when nothing was stripped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<String>,
    /// SHA-256 over canonical-JSON of `payload` pre-redaction.
    pub content_hash: String,
    /// Parent event id for nested/derived events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
}

/// Router destination for an event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// Live-capture path.
    Live,
    /// Bootstrap / backfill path.
    Bootstrap,
}

/// Discriminator selecting the payload schema.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// One user <-> assistant exchange.
    Turn,
    /// Invocation of a tool.
    ToolCall,
    /// Invocation of a sub-agent.
    AgentCall,
    /// Persisted memory op.
    Memory,
    /// Formalized decision record.
    Decision,
    /// Deep-analysis report.
    Analysis,
    /// Law detector fired.
    LawViolation,
    /// Stand-alone artifact.
    Artifact,
}

impl Kind {
    /// Filename stem for the matching per-kind schema file.
    pub fn schema_stem(self) -> &'static str {
        match self {
            Kind::Turn => "turn",
            Kind::ToolCall => "tool_call",
            Kind::AgentCall => "agent_call",
            Kind::Memory => "memory",
            Kind::Decision => "decision",
            Kind::Analysis => "analysis",
            Kind::LawViolation => "law_violation",
            Kind::Artifact => "artifact",
        }
    }
}

/// Capture-time context common to every event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Repo root (absolute, forward slashes); `None` when not in a repo.
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Git branch; `None` when not in a git checkout.
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Commit SHA (7–40 hex chars); `None` when not in a git checkout.
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Working directory when the event occurred.
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// `user@org` identifier.
    pub user: Option<String>,
    /// OS platform (`win32` / `darwin` / `linux`).
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// IDE / host identifier.
    pub ide: Option<String>,
    /// Adapter-specific bag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: BTreeMap<String, Value>,
}

// ---------- per-kind payloads ----------

/// Payload for [`Kind::Turn`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    /// The user prompt / message. Required even if empty-string is disallowed by the schema (`minLength` not set: the schema permits empty strings).
    pub user_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Assistant response; `None` for an in-progress turn emitted early.
    pub assistant_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Token usage stats.
    pub tokens: Option<TurnTokens>,
    /// Back-references to child `tool_call` events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_call_event_ids: Vec<String>,
}

/// Token usage carried on a [`Turn`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnTokens {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    /// Input tokens.
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Output tokens.
    pub out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Prompt-cache read tokens.
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Prompt-cache write tokens.
    pub cache_write: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Count of streamed chunks (analytics).
    pub streamed_chunks: Option<u64>,
}

/// Payload for [`Kind::ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Tool identifier (controlled per adapter).
    pub tool_name: String,
    /// Input arguments as seen by the tool.
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Output; `None` when the call was blocked before execution.
    pub output: Option<ToolCallOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Wall-clock duration in ms.
    pub duration_ms: Option<u64>,
    /// Touched artifacts resolved by the adapter post-call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<TouchedArtifact>,
    /// `"success"`, `"error"`, or `"blocked_by_law:LAW-NNN"`.
    pub outcome: String,
}

/// Output subsection of [`ToolCall`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Standard output (possibly truncated / CAS-offloaded).
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Standard error.
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Exit code.
    pub exit_code: Option<i32>,
    /// Truncation flag.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// CAS reference when `stdout` was offloaded.
    pub cas_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Original size in bytes.
    pub size: Option<u64>,
}

/// Artifact touched by a [`ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TouchedArtifact {
    /// Operation kind.
    pub kind: String,
    /// Target path.
    pub path: String,
}

/// Payload for [`Kind::AgentCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCall {
    /// Sub-agent type (e.g., `"code-reviewer"`).
    pub agent_type: String,
    /// Short description.
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Prompt forwarded to the sub-agent.
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Model used by the sub-agent.
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Team name when dispatched inside a team.
    pub team_name: Option<String>,
    /// Back-references to child events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Structured result; `None` on failure.
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Wall-clock duration in ms.
    pub duration_ms: Option<u64>,
    /// `success` | `error` | `cancelled` | `timeout`.
    pub outcome: String,
}

/// Payload for [`Kind::Memory`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryPayload {
    /// `write` | `update` | `delete`.
    pub op: String,
    /// `user` | `feedback` | `project` | `reference`.
    pub memory_type: String,
    /// Memory title.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Body for `write`/`update`; `None` for `delete`.
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// On-disk path when applicable.
    pub memory_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Short description (memory metadata).
    pub description: Option<String>,
}

/// Payload for [`Kind::Decision`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionPayload {
    /// ADR-style identifier (e.g. `DEC-0042`) or ULID.
    pub decision_id: String,
    /// Decision title.
    pub title: String,
    /// `proposed` | `accepted` | `superseded` | `deprecated`.
    pub status: String,
    /// Markdown body.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Decision this one supersedes.
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// CAS reference when the body was offloaded.
    pub cas_ref: Option<String>,
    /// Free-form tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Payload for [`Kind::Analysis`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisPayload {
    /// Analysis ULID.
    pub analysis_id: String,
    /// The question being analyzed.
    pub question: String,
    /// Lifecycle status.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Scope of the analysis.
    pub scope: Option<AnalysisScope>,
    /// Panel of agent identifiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panel: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Planned round count.
    pub rounds_planned: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Completed round count.
    pub rounds_completed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Final Decision ID (set once resolved).
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Budget cap in USD.
    pub budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Spent USD.
    pub spent_usd: Option<f64>,
}

/// Scope block on an [`AnalysisPayload`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Target repo.
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Topics to focus on.
    pub topics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Files to focus on.
    pub files: Vec<String>,
}

/// Payload for [`Kind::LawViolation`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LawViolationPayload {
    /// Violation ULID.
    pub violation_id: String,
    /// Law identifier (e.g. `LAW-007`).
    pub law_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Law version number.
    pub law_version: Option<u32>,
    /// `info` | `notable` | `critical`.
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Enforcement tier 1–4.
    pub tier: Option<u8>,
    /// Human-readable message.
    pub message: String,
    /// Detector-supplied evidence.
    #[serde(default)]
    pub evidence: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Event where the violation was observed.
    pub observed_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Detector latency in ms.
    pub detector_latency_ms: Option<u64>,
}

/// Payload for [`Kind::Artifact`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactPayload {
    /// `file` | `diff` | `snippet` | `url` | `binary`.
    pub artifact_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Filesystem path when applicable.
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// URL when applicable.
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Language identifier (Tree-sitter name).
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Inline body (truncated to 1 MB by the envelope rule).
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// CAS reference when body offloaded.
    pub cas_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Original size in bytes.
    pub size: Option<u64>,
    /// Truncation flag.
    #[serde(default)]
    pub truncated: bool,
    /// Free-form metadata bag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

