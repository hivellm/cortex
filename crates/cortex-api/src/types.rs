//! Wire types for the query API. Mirrors `docs/specs/11-query-api.md`
//! §Inputs/Outputs verbatim — every field name and shape lines up
//! with the spec so MCP and HTTP callers see the same JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Intent the orchestrator dispatches against — drives lane selection
/// and the overlay set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Pre-thinking context bundle for the Claude Code adapter
    /// (spec 12 owns the adapter-side wrapper).
    PreChangeContext,
    /// Decision lookup with supersession chain.
    DecisionLookup,
    /// Similar-problems search across past turns + analyses.
    SimilarProblems,
    /// Active laws + recent violations within scope.
    LawCheck,
    /// Free-form keyword + vector blend with no overlays.
    FreeSearch,
    /// Phase6d — navigational / explanatory prompts ("how does X
    /// work", "what is X", "where is X defined"). Vector +
    /// keyword fan-out on `code` + `docs` topics; **no decision /
    /// law / similar-turn overlays** because the user is asking
    /// to read code, not to consult policy. Closes F-006.
    Explain,
}

impl Intent {
    /// Snake-case label for telemetry + audit.
    pub fn label(self) -> &'static str {
        match self {
            Intent::PreChangeContext => "pre_change_context",
            Intent::DecisionLookup => "decision_lookup",
            Intent::SimilarProblems => "similar_problems",
            Intent::LawCheck => "law_check",
            Intent::FreeSearch => "free_search",
            Intent::Explain => "explain",
        }
    }
}

/// Result fields the caller wants surfaced. Matches the spec-11
/// `include` array verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncludeField {
    /// Code / doc snippets (vector + keyword fusion).
    Snippets,
    /// Decision overlays.
    Decisions,
    /// Law-violation overlays.
    Violations,
    /// Graph-neighbor expansion.
    GraphNeighbors,
    /// Similar-turn KNN derivation.
    SimilarTurns,
}

/// Canonicalised scope filter — populated from request and validated
/// against ACLs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Scope {
    /// Repo identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// File-glob filters (Meilisearch + Vectorizer compatible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Topic filters (controlled vocab from the classifier).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    /// ISO-8601 lower bound on `ts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Phase11i §3.1 — caller-supplied recency-decay λ in
    /// `1 / day`. When `None`, the orchestrator falls back to
    /// the per-intent default in
    /// [`crate::fusion::FusionConfig::default_recency_lambda_for_intent`].
    /// Set explicitly to `Some(0.0)` to opt out of decay for one
    /// query without disabling the global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency_decay: Option<f32>,
    /// Phase11i §3.2 — caller-supplied cross-repo boost in
    /// `[0.0, 1.0]`. When `None`, the orchestrator uses
    /// [`crate::fusion::DEFAULT_CROSS_REPO_BOOST`] (0.0 — same
    /// outcome as the existing per-repo lane filter). When
    /// `Some(b)` with `b > 0`, foreign hits are admitted with
    /// their fused score multiplied by `b`. Pairs with `repo` —
    /// hits whose `repo` field differs from `scope.repo` are
    /// the ones that get the multiplier applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_repo_boost: Option<f32>,
    /// Phase11i §3.3 — author / model allow-list. When non-empty,
    /// the keyword + vector lanes filter to envelopes whose
    /// `model` field matches one of these values. Empty list
    /// (the default) means "no model filter". Pair with
    /// `settings.v1.json` filterableAttributes that ships
    /// `model` as a top-level filterable field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Phase11i §3.3 — originating-tool allow-list (`claude-code`,
    /// `openai-codex`, etc.). Same semantics as `models`. The
    /// classifier worker stamps a `tool:<name>` topic; this
    /// filter additionally lets callers narrow on the typed
    /// envelope field rather than the topic string.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Phase11i §3.4 — caller's current session id. When set,
    /// the orchestrator boosts hits sharing this session so
    /// burst context surfaces tightly. Pairs with
    /// `Scope.session_cohort` for related-session promotion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Phase11i §3.4 — additional session ids whose hits should
    /// receive the cohort boost (lighter than `session_id`'s
    /// active-session boost).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_cohort: Vec<String>,
    /// Phase11i §3.5 — outcome allow-list. When non-empty, only
    /// hits whose `outcome` field matches one of these values
    /// are admitted. Common values: `success`, `error`,
    /// `partial`, `blocked_by_law`. Empty list = no filter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<String>,
    /// Phase11i §3.5 — outcome deny-list. Hits whose `outcome`
    /// matches any of these are filtered out. Useful for
    /// "exclude error/partial" patterns without enumerating
    /// every accepted outcome.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_outcomes: Vec<String>,
}

/// Request body for `POST /v1/query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    /// Intent — required.
    pub intent: Intent,
    /// Optional scope filter.
    #[serde(default)]
    pub scope: Scope,
    /// Free-text query.
    pub query: String,
    /// Per-field result cap.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// KNN top-k cap for the vector lane.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Result fields the caller wants populated.
    #[serde(default = "default_include")]
    pub include: Vec<IncludeField>,
    /// Total budget the orchestrator must stay under.
    #[serde(default = "default_budget_ms")]
    pub budget_ms: u64,
    /// Phase11c — byte-cap for the serialised response. Caller-side
    /// override of the default `32 KiB` cap. Optional so existing
    /// callers continue to get the default; explicit `Some(N)`
    /// tightens or loosens the clipper's budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_bytes: Option<usize>,
    /// Phase18 §4.4/§4.5 — bitemporal `as_of` anchor (RFC-3339 or
    /// `YYYY-MM-DD` per ADR-018). Missing defaults to wall-clock
    /// "now" so existing callers stay unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    /// Phase18 §4.4/§4.5 — branch composite id (`<project>:<branch>`)
    /// per ADR-019. Missing defaults to `<scope.repo>:main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Phase18 §4.4/§4.5 — cross-project axis activation list per
    /// ADR-020. Default empty (cross-project off); explicit
    /// `Some(["a", "b"])` unions facts from those projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    /// Phase18 §4.4/§4.5 — opt-in to drop the classifier's default
    /// `Drop` action for SUPERSEDED / EXPIRED hits (they get
    /// demoted instead). Off by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_history: Option<bool>,
    /// Phase18 §4.4/§4.5 — opt-in to keep NOT_YET_VALID hits
    /// (planning queries). Off by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_future: Option<bool>,
    /// Phase18 §4.4/§4.5 — opt-in to keep ABANDONED branch hits.
    /// Off by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_branches: Option<bool>,
}

fn default_limit() -> usize {
    20
}
fn default_k() -> usize {
    50
}
fn default_budget_ms() -> u64 {
    500
}
fn default_include() -> Vec<IncludeField> {
    vec![
        IncludeField::Snippets,
        IncludeField::Decisions,
        IncludeField::Violations,
        IncludeField::GraphNeighbors,
        IncludeField::SimilarTurns,
    ]
}

/// One snippet returned by the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    /// 1-based fused rank.
    pub rank: usize,
    /// `vector` | `keyword` | `graph`.
    pub source: String,
    /// Vectorizer collection / Meili index name (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Repo identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Repo-relative path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Symbol (Tree-sitter for code, H1 for doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// `sha256:...` over the chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Snippet text (already redacted by upstream, re-redacted by api).
    pub text: String,
    /// phase10b §2.2 — `true` when the keyword lane could not
    /// project a body for this hit (the upstream document was
    /// indexed without inline body — typical for large / binary
    /// artifacts). The bundle renderer uses this to render the
    /// header alone (no body block) and stamp an ellipsis cue, so
    /// agents do NOT see the path masquerading as the file
    /// contents. Defaults to `false` and skip-serialises so the
    /// wire shape stays backwards-compatible for existing
    /// consumers.
    #[serde(default, skip_serializing_if = "is_false")]
    pub body_truncated: bool,
    /// Fused score.
    pub score: f64,
    /// Why-this-result blurb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// One decision overlay entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRef {
    /// 1-based rank within the overlay.
    pub rank: usize,
    /// Decision id (`DEC-NNNN` or ULID).
    pub id: String,
    /// Title.
    pub title: String,
    /// phase10a §1.2 — first 1 KiB of the rationale body, when the
    /// upstream document carries one. Lets `decision_lookup`
    /// callers quote the ADR body without re-hydrating the full
    /// snippet payload. Optional + skip-serialising so the wire
    /// shape remains backwards-compatible for callers that only
    /// rendered title / status before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale_excerpt: Option<String>,
    /// Status (`proposed` / `accepted` / `superseded` / `deprecated`).
    pub status: String,
    /// Decision timestamp in ms epoch.
    pub ts: i64,
    /// Score from the underlying lane.
    pub score: f64,
    /// Optional links to source files / urls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

/// One law-violation overlay entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViolationRef {
    /// Violation id (ULID).
    pub id: String,
    /// Law id (`LAW-NNN`).
    pub law_id: String,
    /// `info` / `notable` / `critical`.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Anchor — `turn:<id>` / `tool_call:<id>`.
    pub observed_in: String,
}

/// One graph-neighbor entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNeighbor {
    /// Source node identifier.
    pub from: String,
    /// Edge type.
    pub relation: String,
    /// Target node identifier.
    pub to: String,
    /// Hop distance from the seed (1 or 2).
    pub hops: u8,
}

/// Phase11i §4.1 — one past-session entry surfaced under
/// `results.past_sessions`. Sourced from the Claude-archive
/// indexer and ranked by centroid similarity to the current
/// query (top-3 by default). The spec-12 renderer formats one
/// line per session: `id, date, first user prompt (clipped 80
/// chars), turn count` so the agent can recognise prior sessions
/// that touched the same problem space without reading every
/// turn back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PastSession {
    /// Session ULID (matches the `session_id` carried on every
    /// envelope emitted by `cortex-claude-archive`).
    pub session_id: String,
    /// Session start timestamp — typically the earliest turn's
    /// `ts`. Epoch ms; renderers convert to `YYYY-MM-DD`.
    pub ts: i64,
    /// First user prompt of the session. Renderers clip to 80
    /// chars on a UTF-8 boundary; this field carries the raw
    /// prompt so callers wanting the full text can read it.
    pub first_prompt: String,
    /// Number of turns observed in the session.
    pub turn_count: u32,
    /// Centroid similarity score that placed this session in
    /// the top-N. Identical semantics to
    /// [`SimilarTurn::score`].
    pub score: f64,
}

/// One similar-turn entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimilarTurn {
    /// Turn ULID.
    pub turn_id: String,
    /// Turn timestamp.
    pub ts: i64,
    /// Model identifier.
    pub model: String,
    /// Classifier-supplied summary.
    pub summary: String,
    /// KNN score.
    pub score: f64,
    /// Phase11i §4.2 — outcome label as classified upstream
    /// (`success` / `error` / `partial` / `blocked_by_law` …).
    /// Drives the renderer's outcome glyph (`✓` / `✗` / `⚠`).
    /// Empty / `None` falls back to the neutral glyph so a
    /// missing-tag regression still renders consistently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Active law surfaced as an overlay (separate from violations).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LawRef {
    /// Law id.
    pub id: String,
    /// Severity.
    pub severity: String,
    /// Title.
    pub title: String,
}

/// Per-lane / overall budget breakdown surfaced in the response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BudgetReport {
    /// Total wall-clock used (ms).
    pub used_ms: u64,
    /// Cap from the request.
    pub cap_ms: u64,
    /// `hit` | `miss`.
    pub cache: String,
}

/// Lane-level latency / partial flags surfaced under `debug.lanes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaneTimings {
    /// Wall-clock taken by the vector lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_ms: Option<u64>,
    /// Wall-clock taken by the keyword lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_ms: Option<u64>,
    /// Wall-clock taken by the graph lane (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_ms: Option<u64>,
}

/// Debug bag attached to every response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebugInfo {
    /// Lane wall-clock breakdown.
    pub lanes: LaneTimings,
    /// Per-lane errors observed during fan-out.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<String, String>,
    /// `true` when the orchestrator returned partial results because
    /// the total budget elapsed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// Phase11e §3 — structured per-lane diagnostic notes the
    /// orchestrator collected during fan-out. Distinct from
    /// `errors` because notes are NOT failures: a missing
    /// collection still lets the lane return an empty hit set
    /// fail-open. Surfaced only when
    /// `CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=1` (default 0
    /// keeps the response shape backwards-compatible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<DebugNote>,
}

/// Phase11e §3 — one structured diagnostic entry on `DebugInfo.notes`.
/// Today only `collection_missing` is emitted; the shape leaves
/// room for richer kinds (e.g. `cache_skipped`, `acl_denied`)
/// without a wire change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DebugNote {
    /// Lane that emitted the note (`vector` / `keyword` / `graph`).
    pub lane: String,
    /// Note kind. Stable string label so dashboards can group by it.
    pub kind: String,
    /// Collection / index name the note refers to (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// Human-readable message.
    pub message: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Diagnostic surfaced when the orchestrator can answer the request
/// but wants to flag a structural condition the caller would
/// otherwise miss — e.g. the resolved scope points at a repo the
/// daemon has never indexed. Optional + skip-serialising so the wire
/// shape is fully backwards-compatible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Notice {
    /// Stable discriminant the caller branches on.
    /// `repo_not_indexed` — the resolved scope's repo is unknown to
    /// the keyword-lane snapshot used by `/v1/status`.
    pub code: String,
    /// Human-readable single-line explanation.
    pub message: String,
    /// Suggested next step the caller can echo back to the user.
    /// Always present; carries the bootstrap CLI hint for the only
    /// code we emit today (`repo_not_indexed`).
    pub hint: String,
}

/// Top-level response envelope. Matches spec 11 §Response schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryResponse {
    /// Echoed intent label.
    pub intent: String,
    /// Per-request ULID (echoed in audit + dashboard).
    pub query_id: String,
    /// Canonicalised scope.
    pub scope_resolved: Scope,
    /// Per-field result groups.
    pub results: ResultsBag,
    /// Active laws overlay (top-level for cheap rendering).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub laws_active: Vec<LawRef>,
    /// Budget breakdown.
    pub budget: BudgetReport,
    /// Debug bag.
    pub debug: DebugInfo,
    /// Optional structural diagnostic — see [`Notice`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
    /// Phase11c — populated when the byte-budget clipper trimmed the
    /// response. Lets callers see what was dropped + the final size,
    /// and lets MCP adapters detect "more results available" without
    /// re-running the query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipped: Option<ClipReport>,
}

/// Phase11c — per-call byte-budget clip summary attached to
/// [`QueryResponse`] when the clipper actually removed something.
/// Emitted in addition to `Notice` so a structured caller can branch
/// on `removed_*` counts instead of parsing a free-form message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClipReport {
    /// Snippets removed from the tail.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed_snippets: usize,
    /// Decisions removed from the tail.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed_decisions: usize,
    /// Violations removed from the tail.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed_violations: usize,
    /// Similar-turn entries removed from the tail.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed_similar_turns: usize,
    /// Graph-neighbor entries removed from the tail.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub removed_graph_neighbors: usize,
    /// Snippets whose `text` field was clipped to the per-snippet cap
    /// (does NOT count snippets removed entirely above).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub snippets_text_clipped: usize,
    /// Final serialised JSON size after clipping (bytes).
    pub final_bytes: usize,
    /// Budget the clipper targeted (bytes). Echoes
    /// `QueryRequest::budget_bytes` (or its default).
    pub budget_bytes: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Result groups bag carried under `results`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResultsBag {
    /// Snippets (vector + keyword + graph fused).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
    /// Decision overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionRef>,
    /// Violation overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<ViolationRef>,
    /// Graph-neighbor overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_neighbors: Vec<GraphNeighbor>,
    /// Similar-turn KNN.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub similar_turns: Vec<SimilarTurn>,
    /// Phase11i §4.1 — past-session overlay surfaced by the
    /// pre-thinking renderer ("Past sessions" section). Top-3 by
    /// centroid similarity to the current query when the
    /// upstream classifier + claude-archive indexer have populated
    /// the field; empty otherwise so the section gracefully
    /// degrades on cold caches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub past_sessions: Vec<PastSession>,
    /// Phase11j §4.2 — consolidated-context overlay. When ≥ 1
    /// consolidation matches the query, the renderer's
    /// "Consolidated context" section replaces "Past sessions"
    /// and lists the top-3 by similarity. Falls back to
    /// `past_sessions` when this is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consolidations: Vec<ConsolidationRef>,
    /// Phase11r §5.1 — topic-card overlay. When ≥ 1 topic card
    /// matches the query above the staleness threshold, the
    /// renderer's "Topic card" section takes priority over
    /// `consolidations` (per §5.4 reorder); the section is
    /// downgraded when staleness fires. Empty falls back through
    /// `consolidations` → `past_sessions` → `snippets`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_cards: Vec<TopicCardRef>,
}

/// Phase11j §4.2 — one consolidation entry surfaced under
/// `results.consolidations`. Carries just the fields the
/// pre-thinking renderer needs (id, grain, date, title, outcome
/// glyph driver) — the full `ConsolidationPayload` lives in the
/// upstream consolidator's storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidationRef {
    /// Stable consolidation id (`cons-ses-...`, `cons-top-...`,
    /// `cons-dec-...`).
    pub consolidation_id: String,
    /// `session` / `topic` / `decision_trace` — drives the
    /// `grain/id` prefix the renderer prints.
    pub grain: String,
    /// Consolidation timestamp (epoch ms; renderers convert to
    /// `YYYY-MM-DD`).
    pub ts: i64,
    /// One-line title (≤ 80 chars by spec 11j §1).
    pub title: String,
    /// Dominant outcome from the source-event distribution.
    /// Drives the renderer's outcome glyph (`✓` / `✗` / `⚠`).
    /// Empty when the consolidation could not infer a dominant
    /// outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Similarity score that placed this entry in the top-N.
    pub score: f64,
}

/// Phase11r §5.1 — one topic-card entry surfaced under
/// `results.topic_cards`. Carries just the fields the pre-thinking
/// renderer needs (id, slug, synthesis preview, top-5 evidence,
/// open contradictions, confidence, age, events-since-last-rev,
/// score). The full `TopicCardPayload` lives upstream in
/// `cortex_topic_cards`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicCardRef {
    /// Deterministic topic-card id (`topic-{24-hex}`).
    pub topic_card_id: String,
    /// Human-readable slug.
    pub topic_slug: String,
    /// Monotonic revision the renderer surfaces in the section
    /// header line. Required by the §5.3 format
    /// `[slug] (rev N, confidence X%, age Yd, +Z ev)`; the §5.1
    /// brief omitted it but the §5.3 line spec needs the value.
    pub revision: u32,
    /// Clipped synthesis body. Spec §5.1: ≤ 600 bytes.
    pub synthesis_preview: String,
    /// Top-5 evidence items by `cited_at_rev` desc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_top5: Vec<cortex_core::events::EvidenceRef>,
    /// Open (`status == Open`) contradictions only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_contradictions: Vec<cortex_core::events::Contradiction>,
    /// Synthesis confidence, `0.0..=1.0`.
    pub confidence: f32,
    /// Age in days since `last_rev_at` — drives the §5.4
    /// staleness advisory.
    pub synthesis_age_d: u32,
    /// Counter of new evidence events observed since the last
    /// rewrite. Drives the staleness advisory together with
    /// `synthesis_age_d`.
    pub events_since_last_rev: u32,
    /// Similarity score that placed this card in the top-N.
    pub score: f32,
}

/// Convenience helper to build an empty response stamped with a fresh
/// `query_id` and the request's intent label.
pub fn empty_response(req: &QueryRequest) -> QueryResponse {
    QueryResponse {
        intent: req.intent.label().to_string(),
        query_id: ulid::Ulid::new().to_string(),
        scope_resolved: req.scope.clone(), // canonicalised for real responses by `QueryService::handle`
        results: ResultsBag::default(),
        laws_active: Vec::new(),
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: req.budget_ms,
            cache: "miss".to_string(),
        },
        debug: DebugInfo::default(),
        notice: None,
        clipped: None,
    }
}

/// Free-form properties bag — used by lanes to round-trip raw
/// upstream metadata into the orchestrator without exposing
/// vendor-specific types.
pub type Props = BTreeMap<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_round_trips_through_json() {
        let r: QueryRequest = serde_json::from_value(serde_json::json!({
            "intent": "pre_change_context",
            "query": "tune ef_search",
        }))
        .unwrap();
        assert_eq!(r.intent, Intent::PreChangeContext);
        assert_eq!(r.limit, default_limit());
        assert_eq!(r.k, default_k());
        assert_eq!(r.budget_ms, default_budget_ms());
    }

    #[test]
    fn include_default_covers_every_field() {
        let r: QueryRequest = serde_json::from_value(serde_json::json!({
            "intent": "free_search",
            "query": "x",
        }))
        .unwrap();
        let inc = r.include;
        assert!(inc.contains(&IncludeField::Snippets));
        assert!(inc.contains(&IncludeField::Decisions));
        assert!(inc.contains(&IncludeField::Violations));
        assert!(inc.contains(&IncludeField::GraphNeighbors));
        assert!(inc.contains(&IncludeField::SimilarTurns));
    }
}
