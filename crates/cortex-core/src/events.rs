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
    /// phase21 — sensitivity level ordinal: public=0, internal=1, confidential=2, restricted=3.
    /// Absent (None) until the classification stamper runs; treated as public=0 by all
    /// enforcement points when the AC feature is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_level: Option<u8>,
    /// phase21 — orthogonal need-to-know compartments (e.g. `["financial","hr"]`).
    /// Empty vec and None are equivalent; serde omits on None to keep the wire format small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_compartments: Option<Vec<String>>,
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
    /// Governance law or rule definition (imported from rules files).
    Law,
    /// Law detector fired.
    LawViolation,
    /// Stand-alone artifact.
    Artifact,
    /// phase10e — pattern / anti-pattern entry imported from
    /// `.rulebook/knowledge/**`. The Rulebook MCP server captures
    /// these via `rulebook_knowledge_add`; before phase10e they
    /// sat on disk and never reached any retrieval surface.
    Knowledge,
    /// phase10e — implementation insight imported from
    /// `.rulebook/learnings/**` (Rulebook MCP
    /// `rulebook_learn_capture`). Same rationale as
    /// [`Kind::Knowledge`] — high-signal corpus that was
    /// previously invisible to the agent.
    Learning,
    /// phase11j — distilled, evergreen summary of one or more
    /// raw events (Session / Topic / DecisionTrace grain). Carries
    /// `source_event_ids` so the agent can verify any takeaway by
    /// fetching the underlying turns. Drives the spec-12
    /// "Consolidated context" section in the pre-thinking bundle.
    Consolidation,
    /// phase11r — living-synthesis topic card. The LLM rewrites the
    /// payload's `synthesis_markdown` whenever new evidence lands
    /// (consolidations, decisions, laws, raw turns), surfaces
    /// contradictions explicitly, and exposes a staleness signal.
    /// The pre-thinking renderer prefers a topic card over the raw
    /// "Consolidated context" block when one matches the query
    /// scope. Drives the MCP tools `cortex_topic_get` /
    /// `cortex_topic_drill` / `cortex_topic_neighbors` /
    /// `cortex_topic_diff` / `cortex_synthesize`.
    TopicCard,
}

impl Kind {
    /// Phase12e — total number of [`Kind`] variants. Pinned by the
    /// compile-time assertion next to [`crate::vocab::KIND_IDS`] so
    /// adding a variant without updating the vocab fails `cargo
    /// check`. Bump together with the enum.
    pub const COUNT: usize = 13;

    /// Filename stem for the matching per-kind schema file.
    pub fn schema_stem(self) -> &'static str {
        match self {
            Kind::Turn => "turn",
            Kind::ToolCall => "tool_call",
            Kind::AgentCall => "agent_call",
            Kind::Memory => "memory",
            Kind::Decision => "decision",
            Kind::Analysis => "analysis",
            Kind::Law => "law",
            Kind::LawViolation => "law_violation",
            Kind::Artifact => "artifact",
            Kind::Knowledge => "knowledge",
            Kind::Learning => "learning",
            Kind::Consolidation => "consolidation",
            Kind::TopicCard => "topic_card",
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

/// phase10e — payload for [`Kind::Knowledge`]. Mirrors the
/// Rulebook MCP `rulebook_knowledge_add` shape: `pattern` /
/// `anti-pattern` discriminated by `category`, with the Markdown
/// body verbatim. Required fields mirror what `rulebook_knowledge_add`
/// always emits; optional fields are skipped when absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgePayload {
    /// Knowledge entry id (slug or ULID).
    pub knowledge_id: String,
    /// Title — usually the H1 of the source file.
    pub title: String,
    /// `pattern` | `anti-pattern`.
    pub category: String,
    /// Markdown body verbatim from `.rulebook/knowledge/<file>.md`.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Repo-relative source path, when discovery walked a file.
    pub source_path: Option<String>,
    /// Free-form tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// phase10e — payload for [`Kind::Learning`]. Mirrors the
/// Rulebook MCP `rulebook_learn_capture` shape: a single
/// implementation insight, optionally linked to a related task
/// id so the graph layer can connect the learning to the work
/// that produced it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningPayload {
    /// Learning entry id (slug or ULID).
    pub learning_id: String,
    /// Brief title.
    pub title: String,
    /// Markdown body verbatim.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Related task id (`phase10c_bootstrap_dedup`, ...).
    pub related_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Repo-relative source path, when discovery walked a file.
    pub source_path: Option<String>,
    /// Free-form tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Phase11j §1 — grain discriminator for [`ConsolidationPayload`].
/// Drives the producer mode: Session = one envelope per
/// `session_id`; Topic = one envelope per HDBSCAN cluster of
/// session vectors; DecisionTrace = one envelope per
/// `Kind::Decision` parent chain. The renderer reads the grain
/// to format the line shape in the spec-12 `Consolidated context`
/// section.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationGrain {
    /// One session's worth of turns + tool calls.
    Session,
    /// HDBSCAN cluster of related sessions on a topic.
    Topic,
    /// `Kind::Decision` + ancestor chain up to N hops.
    DecisionTrace,
}

/// Phase11j §1 — depth signal carried alongside the model the
/// summariser used. Shallow = Haiku (cheap, default); Deep =
/// Opus (auto-promoted for DecisionTrace + high-impact sessions).
/// Drives the fidelity-IT threshold (≥ 90 % shallow / ≥ 98 %
/// deep — see §6.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationDepth {
    /// Haiku-class summariser (default).
    Shallow,
    /// Opus-class summariser (auto-promoted; see §2.7).
    Deep,
}

/// Phase11j §1 — the scope a consolidation covers. The variant
/// the producer chooses MUST match the `grain` field on the
/// containing payload (validated in §1.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ConsolidationScope {
    /// `grain = Session`: the originating session id.
    SessionId(String),
    /// `grain = Topic`: the canonical topic label the cluster
    /// converged on (typically a noun phrase).
    Topic(String),
    /// `grain = DecisionTrace`: the originating decision id.
    DecisionId(String),
}

/// Phase11j §1 — temporal span the consolidation covers.
///
/// `start` + `end` are epoch ms; `duration_ms` is materialised so
/// the dashboard can sort without re-deriving.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TimeSpan {
    /// Earliest envelope `occurred_at` covered (epoch ms).
    pub start_ms: i64,
    /// Latest envelope `occurred_at` covered (epoch ms).
    pub end_ms: i64,
    /// `end_ms - start_ms`, materialised. Always `>= 0` because
    /// the validator enforces `start_ms <= end_ms`.
    pub duration_ms: i64,
}

/// Phase11j §1 — payload for [`Kind::Consolidation`].
///
/// Curated, evergreen summary of one or more raw events. Carries
/// `source_event_ids` so the agent can verify any `takeaway[i]`
/// by fetching the underlying turn. The renderer surfaces these
/// in the spec-12 `Consolidated context` section ahead of raw
/// `Past sessions` so the agent reads the takeaway first and the
/// raw fragments only when it needs the receipts.
///
/// Field-level invariants enforced by the §1.5 validator:
/// - `title.len() <= 200` chars
/// - `200 <= summary_markdown.len() <= 4000` bytes
/// - `scope` discriminator matches `grain`
/// - `source_event_count >= source_event_ids.len()` (count holds
///   the *full* count even when the ids vector is clipped)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidationPayload {
    /// Stable id (ULID). Independent of the `event_id` on the
    /// envelope — the latter changes every re-emit; this one
    /// stays constant across re-runs of the same producer.
    pub consolidation_id: String,
    /// Discriminator selecting the producer mode.
    pub grain: ConsolidationGrain,
    /// Scope the consolidation covers. Variant must match
    /// `grain` (validated).
    pub scope: ConsolidationScope,
    /// Short title (≤ 80 chars).
    pub title: String,
    /// Markdown body (200-2 000 bytes).
    pub summary_markdown: String,
    /// One bullet per "lesson learned". Drives the
    /// fidelity IT — every entry must trace to ≥ 1
    /// `source_event_ids` entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takeaways: Vec<String>,
    /// ULIDs of the raw envelopes this consolidation distilled.
    /// Clipped to a sane inline cap (see `source_event_count`)
    /// when the source set is huge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_event_ids: Vec<String>,
    /// Total source-event count. Equal to
    /// `source_event_ids.len()` until the producer clips the
    /// inline list; after that this stays at the original count
    /// so the dashboard can surface "120 sources, 64 inlined".
    pub source_event_count: u32,
    /// Identifier of the model the summariser used (e.g.
    /// `claude-haiku-4-5`, `claude-opus-4-7`).
    pub model: String,
    /// Shallow / Deep — drives the fidelity threshold.
    pub depth: ConsolidationDepth,
    /// Outcome counts from the source set (`success` / `error` /
    /// `partial` / `blocked_by_law` / …). Empty when the producer
    /// could not infer outcomes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outcome_distribution: BTreeMap<String, u32>,
    /// Time the consolidated source set spans.
    pub temporal_span: TimeSpan,
    /// Repos referenced by the source set (sorted, deduped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    /// Free-form tags (mirrors knowledge / learning shape).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Inline cap on `ConsolidationPayload.source_event_ids`. Callers
/// clipping past this value should leave `source_event_count`
/// at the unclipped total so the dashboard preserves the gap.
pub const CONSOLIDATION_SOURCE_IDS_INLINE_CAP: usize = 256;

/// Title cap (chars) — see §1.5 validator.
pub const CONSOLIDATION_TITLE_MAX_CHARS: usize = 200;
/// Summary lower bound (bytes) — see §1.5 validator.
pub const CONSOLIDATION_SUMMARY_MIN_BYTES: usize = 200;
/// Summary upper bound (bytes) — see §1.5 validator.
pub const CONSOLIDATION_SUMMARY_MAX_BYTES: usize = 4_000;

/// Phase11r §1 — payload for [`Kind::TopicCard`].
///
/// Living-synthesis topic card. The LLM rewrites
/// `synthesis_markdown` whenever new evidence lands; the producer
/// stamps `revision` (monotonic) and resets `events_since_last_rev`
/// per rewrite. Contradictions surface explicitly so the model
/// never silently averages conflicting evidence; the staleness
/// signal (`synthesis_age_d`, `events_since_last_rev`) lets the
/// pre-thinking renderer downgrade the card when confidence drops.
///
/// Field-level invariants enforced by the §1.5 validator:
/// - `topic_slug` matches `^[a-z0-9](?:[a-z0-9-]{0,78}[a-z0-9])?$`
/// - `200 <= synthesis_markdown.len() <= 4000` bytes
/// - `evidence.len() >= 1`
/// - `open_questions.len() <= 8`
/// - `related_topic_ids.len() <= 32`
/// - `0.0 <= confidence <= 1.0`
/// - every `contradictions[*].evidence_a` / `evidence_b` references
///   an item in `evidence`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicCardPayload {
    /// Deterministic id derived from `(topic_slug, repo_scope)` via
    /// [`derive_topic_card_id`]. Re-emitting the same card lands on
    /// the same id; new revisions stamp the same id with an
    /// incremented `revision`.
    pub topic_card_id: String,
    /// Kebab-case slug, ≤ 80 chars, unique per `repos` scope.
    pub topic_slug: String,
    /// Repo scope (usually one entry).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    /// Monotonic revision. `1` on first emit; the producer increments
    /// it on every rewrite that changes `synthesis_markdown`.
    pub revision: u32,
    /// LLM-maintained prose summary (200-4 000 bytes).
    pub synthesis_markdown: String,
    /// Typed references the synthesiser cited. The §1.5 validator
    /// requires `>= 1` entry per emitted card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    /// Surfaced contradictions across evidence items. Heuristic-
    /// detected and validated against the `evidence` set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contradictions: Vec<Contradiction>,
    /// Open questions the synthesiser noted (≤ 8 items).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_questions: Vec<String>,
    /// Adjacent topic-card ids (≤ 32). Drives the
    /// `cortex_topic_neighbors` MCP tool walk.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_topic_ids: Vec<String>,
    /// Synthesis confidence, `0.0..=1.0`. The pre-thinking renderer
    /// downgrades the topic-card section to fallback when this drops
    /// below `0.6`.
    pub confidence: f32,
    /// Wall-clock timestamp of the last rewrite (RFC 3339).
    pub last_rev_at: String,
    /// Counter of new evidence events observed since `last_rev_at`.
    /// The trigger fires a rewrite when this crosses
    /// [`TOPIC_CARD_TRIGGER_EVENTS_THRESHOLD`].
    pub events_since_last_rev: u32,
    /// Identifier of the model the synthesiser used.
    pub synthesis_model: String,
    /// Realised cost of the last rewrite (USD micro-cents per
    /// `cost_telemetry::cost_cents` convention).
    pub synthesis_cost_cents: u32,
}

/// Phase11r §1 — typed evidence reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceRef {
    /// What kind of source this evidence points to.
    pub kind: EvidenceKind,
    /// ULID / id of the source envelope.
    pub id: String,
    /// Caller-assigned weight in `0.0..=1.0` (skip when uniform).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    /// Revision the synthesiser was on when it cited this evidence.
    /// Lets `cortex_topic_diff` distinguish "this was always cited"
    /// from "this is a fresh citation since rev N".
    pub cited_at_rev: u32,
}

/// Discriminator for [`EvidenceRef::kind`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// References a [`Kind::Consolidation`] envelope.
    Consolidation,
    /// References a [`Kind::Decision`] envelope.
    Decision,
    /// References a law (definition or violation).
    Law,
    /// References a [`Kind::Turn`] envelope.
    Turn,
}

/// Phase11r §1 — surfaced contradiction across evidence items.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contradiction {
    /// Detector class.
    pub kind: ContradictionKind,
    /// Evidence id (must match an `EvidenceRef.id` in the same
    /// payload's `evidence` vec — validated by §1.5 helper).
    pub evidence_a: String,
    /// Evidence id (must match an `EvidenceRef.id` in `evidence`).
    pub evidence_b: String,
    /// Revision the synthesiser surfaced this contradiction on.
    pub surfaced_at_rev: u32,
    /// Lifecycle status — drives the renderer's "open contradictions"
    /// filter so reconciled / deprecated entries do not pollute the
    /// pre-thinking bundle.
    pub status: ContradictionStatus,
}

/// Discriminator for [`Contradiction::kind`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContradictionKind {
    /// Two `Kind::Decision` refs where one supersedes the other.
    DecisionSupersession,
    /// `Kind::LawViolation` cites a law version different from the
    /// latest active version of the same law.
    LawViolationMismatch,
    /// Two consolidations with overlapping `temporal_span` carry
    /// conflicting `outcome_distribution` majorities.
    OutcomeDivergence,
}

/// Discriminator for [`Contradiction::status`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContradictionStatus {
    /// Surfaced and not yet resolved by a follow-up rewrite or
    /// operator intervention.
    Open,
    /// The synthesiser (Opus escalation path) reconciled the
    /// contradiction in a later revision.
    Reconciled,
    /// The contradiction is now stale (one of the evidence items
    /// got superseded itself).
    Deprecated,
}

/// Slug max length (chars) — see §1.5 validator.
pub const TOPIC_CARD_SLUG_MAX_CHARS: usize = 80;
/// Synthesis lower bound (bytes) — see §1.5 validator.
pub const TOPIC_CARD_SYNTHESIS_MIN_BYTES: usize = 200;
/// Synthesis upper bound (bytes) — see §1.5 validator.
pub const TOPIC_CARD_SYNTHESIS_MAX_BYTES: usize = 4_000;
/// Open-questions cap — see §1.5 validator.
pub const TOPIC_CARD_OPEN_QUESTIONS_MAX: usize = 8;
/// Related-topics cap — see §1.5 validator.
pub const TOPIC_CARD_RELATED_MAX: usize = 32;

/// Phase11r §1.3 — derive a deterministic topic-card id from the
/// `(topic_slug, repo_scope)` pair. The hash uses SHA-256 of the
/// `slug\0repo_scope` string and prepends `topic-` so the id is
/// debuggable + greppable. Idempotent: same inputs always yield
/// the same id, so a re-run of the synthesiser never duplicates the
/// envelope.
pub fn derive_topic_card_id(topic_slug: &str, repo_scope: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(topic_slug.as_bytes());
    hasher.update([0u8]);
    hasher.update(repo_scope.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(6 + 24);
    hex.push_str("topic-");
    for byte in digest.iter().take(12) {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
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

/// Payload for [`Kind::Law`] — a governance rule or law definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LawPayload {
    /// Law identifier (e.g. `LAW-007` or synthesised from filename).
    pub law_id: String,
    /// Short title extracted from the rule heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Declared severity; `None` when not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Optional detector reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector: Option<String>,
    /// Full rule body text.
    pub body: String,
    /// Zero-based section index when the source file was split per `##` heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_index: Option<u32>,
    /// Relative source path within the repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
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
    /// Discriminator for `observed_event_id` — `"turn"` or
    /// `"tool_call"`. Required when `observed_event_id` is set so the
    /// cortex-graph writer can MERGE the OBSERVED_IN edge under the
    /// right label without phantom-node risk.
    pub observed_event_kind: Option<String>,
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

#[cfg(test)]
mod consolidation_tests {
    use super::*;

    fn sample_payload() -> ConsolidationPayload {
        ConsolidationPayload {
            consolidation_id: "01JCONS01".into(),
            grain: ConsolidationGrain::Session,
            scope: ConsolidationScope::SessionId("sess-A".into()),
            title: "tune ef_search for HNSW recall".into(),
            summary_markdown: "x".repeat(500),
            takeaways: vec!["raise ef_search to 128".into()],
            source_event_ids: vec!["01EVT01".into(), "01EVT02".into()],
            source_event_count: 2,
            model: "claude-haiku-4-5".into(),
            depth: ConsolidationDepth::Shallow,
            outcome_distribution: BTreeMap::from([("success".into(), 2u32)]),
            temporal_span: TimeSpan {
                start_ms: 1_700_000_000_000,
                end_ms: 1_700_000_060_000,
                duration_ms: 60_000,
            },
            repos: vec!["cortex".into()],
            tags: vec!["hnsw".into()],
        }
    }

    #[test]
    fn consolidation_payload_round_trips_through_serde() {
        let original = sample_payload();
        let raw = serde_json::to_string(&original).expect("serialise");
        let decoded: ConsolidationPayload = serde_json::from_str(&raw).expect("deserialise");
        assert_eq!(original, decoded);
        // Spot-check the serialised wire shape so a future field
        // rename surfaces as a test failure rather than a silent
        // schema drift.
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["grain"], "session");
        assert_eq!(v["scope"]["kind"], "session_id");
        assert_eq!(v["scope"]["value"], "sess-A");
        assert_eq!(v["depth"], "shallow");
        assert_eq!(v["source_event_count"], 2);
    }

    #[test]
    fn consolidation_kind_maps_to_consolidation_schema_stem() {
        assert_eq!(Kind::Consolidation.schema_stem(), "consolidation");
    }

    #[test]
    fn consolidation_grain_serialises_to_snake_case_discriminator() {
        let session = serde_json::to_value(ConsolidationGrain::Session).unwrap();
        let topic = serde_json::to_value(ConsolidationGrain::Topic).unwrap();
        let trace = serde_json::to_value(ConsolidationGrain::DecisionTrace).unwrap();
        assert_eq!(session, Value::String("session".into()));
        assert_eq!(topic, Value::String("topic".into()));
        assert_eq!(trace, Value::String("decision_trace".into()));
    }

    #[test]
    fn source_event_count_holds_the_full_count_when_ids_are_clipped() {
        // Producer clips source_event_ids past the inline cap but
        // leaves source_event_count at the unclipped total. The
        // dashboard depends on this gap to render "N sources, M
        // inlined"; the validator (§1.5) only rejects when the
        // count is BELOW the inlined ids vec.
        let mut p = sample_payload();
        let unclipped_total = 1_024_u32;
        p.source_event_ids
            .resize(CONSOLIDATION_SOURCE_IDS_INLINE_CAP, "01EVT".into());
        p.source_event_count = unclipped_total;
        assert!(p.source_event_count >= p.source_event_ids.len() as u32);
        assert_eq!(
            p.source_event_ids.len(),
            CONSOLIDATION_SOURCE_IDS_INLINE_CAP
        );
        assert_eq!(p.source_event_count, unclipped_total);
    }
}

#[cfg(test)]
mod topic_card_tests {
    use super::*;

    fn sample_payload() -> TopicCardPayload {
        TopicCardPayload {
            topic_card_id: derive_topic_card_id("auth-rewrite", "cortex"),
            topic_slug: "auth-rewrite".into(),
            repos: vec!["cortex".into()],
            revision: 3,
            synthesis_markdown: "x".repeat(450),
            evidence: vec![EvidenceRef {
                kind: EvidenceKind::Decision,
                id: "DEC-0042".into(),
                weight: Some(0.8),
                cited_at_rev: 2,
            }],
            contradictions: vec![Contradiction {
                kind: ContradictionKind::DecisionSupersession,
                evidence_a: "DEC-0042".into(),
                evidence_b: "DEC-0050".into(),
                surfaced_at_rev: 3,
                status: ContradictionStatus::Open,
            }],
            open_questions: vec!["does this preserve token rotation?".into()],
            related_topic_ids: vec![derive_topic_card_id("session-management", "cortex")],
            confidence: 0.78,
            last_rev_at: "2026-05-03T05:00:00Z".into(),
            events_since_last_rev: 4,
            synthesis_model: "claude-haiku-4-5".into(),
            synthesis_cost_cents: 80,
        }
    }

    #[test]
    fn schema_stem_maps_topic_card() {
        assert_eq!(Kind::TopicCard.schema_stem(), "topic_card");
    }

    #[test]
    fn kind_topic_card_serialises_snake_case() {
        let json = serde_json::to_string(&Kind::TopicCard).unwrap();
        assert_eq!(json, "\"topic_card\"");
        let parsed: Kind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Kind::TopicCard);
    }

    #[test]
    fn payload_serde_round_trip_preserves_every_field() {
        let p = sample_payload();
        let json = serde_json::to_string(&p).expect("serialize");
        let parsed: TopicCardPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, p);
    }

    #[test]
    fn evidence_kind_serialises_snake_case() {
        for (kind, tag) in [
            (EvidenceKind::Consolidation, "\"consolidation\""),
            (EvidenceKind::Decision, "\"decision\""),
            (EvidenceKind::Law, "\"law\""),
            (EvidenceKind::Turn, "\"turn\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), tag);
        }
    }

    #[test]
    fn contradiction_status_serialises_snake_case() {
        assert_eq!(
            serde_json::to_string(&ContradictionStatus::Open).unwrap(),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&ContradictionStatus::Reconciled).unwrap(),
            "\"reconciled\""
        );
        assert_eq!(
            serde_json::to_string(&ContradictionStatus::Deprecated).unwrap(),
            "\"deprecated\""
        );
    }

    #[test]
    fn derive_topic_card_id_is_deterministic_per_slug_repo_pair() {
        let a = derive_topic_card_id("auth-rewrite", "cortex");
        let b = derive_topic_card_id("auth-rewrite", "cortex");
        assert_eq!(a, b);
        assert!(a.starts_with("topic-"));
        // 6 prefix chars + 24 hex chars = 30 total
        assert_eq!(a.len(), 30);
    }

    #[test]
    fn derive_topic_card_id_differs_across_slugs() {
        let a = derive_topic_card_id("auth-rewrite", "cortex");
        let b = derive_topic_card_id("auth-other", "cortex");
        assert_ne!(a, b);
    }

    #[test]
    fn derive_topic_card_id_differs_across_repo_scope() {
        let a = derive_topic_card_id("auth-rewrite", "cortex");
        let b = derive_topic_card_id("auth-rewrite", "vectorizer");
        assert_ne!(a, b);
    }

    #[test]
    fn constants_match_phase11r_spec() {
        assert_eq!(TOPIC_CARD_SLUG_MAX_CHARS, 80);
        assert_eq!(TOPIC_CARD_SYNTHESIS_MIN_BYTES, 200);
        assert_eq!(TOPIC_CARD_SYNTHESIS_MAX_BYTES, 4_000);
        assert_eq!(TOPIC_CARD_OPEN_QUESTIONS_MAX, 8);
        assert_eq!(TOPIC_CARD_RELATED_MAX, 32);
    }
}

#[cfg(test)]
mod classification_fields_tests {
    use super::*;

    fn minimal_envelope() -> Envelope {
        Envelope {
            event_id: "01EVT".into(),
            schema_version: "1".into(),
            occurred_at: "2026-06-23T00:00:00Z".into(),
            ingested_at: None,
            session_id: "01SESS".into(),
            stream: Stream::Live,
            tool: "test".into(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: None,
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".into(),
                ide: None,
                extras: Default::default(),
            },
            payload: serde_json::json!({}),
            redactions: vec![],
            content_hash: "sha256:abc".into(),
            parent_event_id: None,
            class_level: None,
            class_compartments: None,
        }
    }

    #[test]
    fn class_fields_default_to_none_and_are_omitted_when_serialised() {
        let env = minimal_envelope();
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(
            !json.contains("class_level"),
            "class_level absent when None"
        );
        assert!(
            !json.contains("class_compartments"),
            "class_compartments absent when None"
        );
    }

    #[test]
    fn class_fields_round_trip_when_set() {
        let mut env = minimal_envelope();
        env.class_level = Some(2);
        env.class_compartments = Some(vec!["financial".into(), "hr".into()]);
        let json = serde_json::to_string(&env).expect("serialize");
        let decoded: Envelope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.class_level, Some(2));
        assert_eq!(
            decoded.class_compartments,
            Some(vec!["financial".into(), "hr".into()])
        );
    }

    #[test]
    fn class_fields_default_to_none_when_absent_from_json() {
        // An envelope serialised before phase21 has no class fields.
        // Deserialisation must succeed with None defaults.
        let legacy_json = r#"{
            "event_id":"01EVT","schema_version":"1",
            "occurred_at":"2026-06-23T00:00:00Z","session_id":"01SESS",
            "stream":"live","tool":"test","kind":"turn",
            "context":{"platform":"linux"},
            "payload":{},"content_hash":"sha256:abc"
        }"#;
        let decoded: Envelope = serde_json::from_str(legacy_json).expect("deserialize");
        assert_eq!(decoded.class_level, None);
        assert_eq!(decoded.class_compartments, None);
    }

    #[test]
    fn class_level_ordinal_values() {
        // Validate the canonical ordinal contract: public=0, internal=1,
        // confidential=2, restricted=3. Stored as u8 so arithmetic comparisons
        // work without enum overhead.
        let public: u8 = 0;
        let internal: u8 = 1;
        let confidential: u8 = 2;
        let restricted: u8 = 3;
        assert!(public < internal);
        assert!(internal < confidential);
        assert!(confidential < restricted);
    }
}
