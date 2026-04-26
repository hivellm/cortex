//! Dashboard backend (spec 16, MVP slice).
//!
//! Three read endpoints under `/v1/dashboard/*`. The Electron GUI in
//! `gui/` is the consumer — `cortex-api` does not serve any HTML or
//! JS itself; it only answers JSON. Production targets (SSE, OIDC,
//! the rest of the spec-16 surface) live under §1–§9 of
//! `phase2_dashboard/tasks.md`.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lanes::MemoryKeywordLane;
use crate::types::{IncludeField, Intent, QueryRequest, Scope};

/// Shared state for dashboard routes — the same `MemoryKeywordLane`
/// the archive loader seeds. Source of truth for `/v1/dashboard/*`
/// until live spec-06 / spec-07 / spec-08 indexers ship.
#[derive(Clone)]
pub struct DashboardState {
    /// Keyword lane the archive_loader populates at boot + on
    /// refresh.
    pub lane: Arc<MemoryKeywordLane>,
}

/// Build the dashboard sub-router carrying the `/v1/dashboard/*` JSON
/// endpoints the GUI consumes. Endpoints whose upstream subsystem is
/// not built yet (laws / decisions / analyses — specs 13/14/15) still
/// answer with an honest empty list rather than mocked rows.
pub fn build_dashboard_router(state: DashboardState) -> Router {
    Router::new()
        .route("/v1/dashboard/overview", get(overview))
        .route("/v1/dashboard/timeline/recent", get(timeline_recent))
        .route("/v1/dashboard/memory", get(memory))
        .route("/v1/dashboard/decisions", get(decisions))
        .route("/v1/dashboard/laws", get(laws))
        .route("/v1/dashboard/violations", get(violations))
        .route("/v1/dashboard/analyses", get(analyses))
        .route("/v1/dashboard/tools/stats", get(tools_stats))
        .route("/v1/dashboard/graph", get(graph))
        .route("/v1/dashboard/sessions", get(sessions))
        .with_state(state)
}

/// Pull the `session_id` extras a hit was stamped with by the
/// archive loader. Helper kept inline so all sites share the same
/// fall-through logic.
fn session_id_of(hit: &crate::lanes::LaneHit) -> Option<&str> {
    hit.extras
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------
// /v1/dashboard/overview
// ---------------------------------------------------------------------

/// Counters + per-kind breakdown derived from the seeded lane. Shape
/// matches what the prototype's overview surfaces consume.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewBody {
    /// Total events visible to the dashboard (sum across canonical
    /// indexes — `cortex-code` is the seeded one today).
    pub events_total: u64,
    /// Distinct repos seen in the lane.
    pub repos_indexed: u64,
    /// Per-kind event counts (`turn`, `tool_call`, `agent_call`).
    pub kind_breakdown: Vec<KindCount>,
    /// Top repos by event count, descending. Capped at 8.
    pub recent_repos: Vec<RepoCount>,
}

/// One row of the per-kind breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct KindCount {
    /// Canonical kind label (`turn` / `tool_call` / `agent_call`).
    pub kind: String,
    /// Number of events with that kind.
    pub count: u64,
}

/// One row of the per-repo breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct RepoCount {
    /// Repo name (best-effort from `context.repo`).
    pub repo: String,
    /// Event count.
    pub count: u64,
}

async fn overview(State(state): State<DashboardState>) -> Response {
    let snapshot = collect_lane_hits(&state.lane);

    let mut by_kind: std::collections::BTreeMap<&'static str, u64> =
        std::collections::BTreeMap::new();
    let mut by_repo: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut events_total: u64 = 0;
    for hit in &snapshot {
        events_total += 1;
        let kind = symbol_to_kind(hit.symbol.as_deref());
        *by_kind.entry(kind).or_insert(0) += 1;
        if let Some(repo) = hit.repo.as_deref() {
            *by_repo.entry(repo.to_string()).or_insert(0) += 1;
        }
    }

    let mut repos_sorted: Vec<RepoCount> = by_repo
        .into_iter()
        .map(|(repo, count)| RepoCount { repo, count })
        .collect();
    repos_sorted.sort_by(|a, b| b.count.cmp(&a.count));
    repos_sorted.truncate(8);

    let body = OverviewBody {
        events_total,
        repos_indexed: repos_sorted.len() as u64,
        kind_breakdown: by_kind
            .into_iter()
            .map(|(kind, count)| KindCount {
                kind: kind.to_string(),
                count,
            })
            .collect(),
        recent_repos: repos_sorted,
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn symbol_to_kind(symbol: Option<&str>) -> &'static str {
    match symbol {
        Some(s) if s.starts_with("tool_call") => "tool_call",
        Some(s) if s.starts_with("agent_call") => "agent_call",
        Some("turn") | None => "turn",
        Some(_) => "turn",
    }
}

// ---------------------------------------------------------------------
// /v1/dashboard/timeline/recent
// ---------------------------------------------------------------------

/// Query params for `/v1/dashboard/timeline/recent`.
#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    /// Cap the result count. Defaults to 50, max 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to a single session. Pass the `session_id` from
    /// `/v1/dashboard/sessions` (canonical 26-char ULID).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Restrict to a single repo (matches `context.repo`).
    #[serde(default)]
    pub repo: Option<String>,
    /// Restrict to a single canonical kind (`turn` / `tool_call` /
    /// `agent_call`). Maps onto the symbol prefix the lane stamps.
    #[serde(default)]
    pub kind: Option<String>,
}

/// One timeline row — shape matches the prototype's `MOCK.events`.
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEvent {
    /// Hit doc_id (`archive|<event_id>`).
    pub id: String,
    /// Wall-clock label (HH:MM:SS) for the prototype's left column.
    /// Derived from the hit's `ts` (RFC-3339 → ms epoch → local
    /// time). Empty when no timestamp was preserved.
    pub t: String,
    /// Canonical kind (`turn` / `tool_call` / `agent_call`).
    pub kind: String,
    /// Short title — for `turn`, the user message's first 80 chars;
    /// for `tool_call`, `[<tool_name>]`; for `agent_call`, the agent
    /// type with `Task:` prefix.
    pub title: String,
    /// Body — full text of the hit, clipped at ~280 chars so the
    /// prototype's row layout stays compact.
    pub detail: String,
    /// Repo identifier (best-effort from `context.repo`).
    pub repo: Option<String>,
    /// Source session id (the canonical 26-char ULID). Surfaced so
    /// the GUI can render a "this row is from session X" pill and
    /// link back to the session detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Adapter / tool — always `claude-code` today, surfaced for
    /// future multi-adapter contexts.
    pub model: String,
}

async fn timeline_recent(
    State(state): State<DashboardState>,
    Query(params): Query<TimelineQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let mut hits = collect_lane_hits(&state.lane);
    if let Some(sid) = params.session_id.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| session_id_of(h) == Some(sid));
    }
    if let Some(repo) = params.repo.as_deref().filter(|r| !r.is_empty()) {
        hits.retain(|h| h.repo.as_deref() == Some(repo));
    }
    if let Some(kind) = params.kind.as_deref().filter(|k| !k.is_empty()) {
        hits.retain(|h| symbol_to_kind(h.symbol.as_deref()) == kind);
    }
    // Newest first by `ts`.
    hits.sort_by(|a, b| b.ts.cmp(&a.ts));
    hits.truncate(limit);

    let events: Vec<TimelineEvent> = hits
        .into_iter()
        .map(|h| TimelineEvent {
            id: h.doc_id.clone(),
            t: ts_to_clock_string(h.ts),
            kind: symbol_to_kind(h.symbol.as_deref()).to_string(),
            title: title_from_hit(&h),
            detail: clip(&h.text, 280),
            repo: h.repo.clone(),
            session_id: session_id_of(&h).map(String::from),
            model: "claude-code".to_string(),
        })
        .collect();
    (StatusCode::OK, Json(events)).into_response()
}

fn title_from_hit(h: &crate::lanes::LaneHit) -> String {
    match h.symbol.as_deref() {
        Some("turn") | None => clip(h.text.lines().next().unwrap_or(""), 80).to_string(),
        Some(s) if s.starts_with("tool_call:") => {
            let tool = s.trim_start_matches("tool_call:");
            format!("[{tool}]")
        }
        Some(s) if s.starts_with("agent_call:") => {
            let agent = s.trim_start_matches("agent_call:");
            format!("Task: {agent}")
        }
        Some(other) => other.to_string(),
    }
}

fn ts_to_clock_string(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        s[..end].to_string()
    }
}

// ---------------------------------------------------------------------
// /v1/dashboard/memory
// ---------------------------------------------------------------------

/// Query params for `/v1/dashboard/memory`.
#[derive(Debug, Deserialize)]
pub struct MemoryQuery {
    /// Free-text query. Empty / missing means "list everything".
    #[serde(default)]
    pub q: Option<String>,
    /// Cap the result count. Defaults to 50, max 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to one session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Restrict to one repo.
    #[serde(default)]
    pub repo: Option<String>,
    /// Restrict to one canonical kind.
    #[serde(default)]
    pub kind: Option<String>,
}

/// One memory row — shape matches the prototype's `MOCK.memories`.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryEntry {
    /// Memory title — derived from the hit's first line.
    pub title: String,
    /// Body excerpt clipped at ~320 chars.
    pub excerpt: String,
    /// Canonical kind.
    pub kind: String,
    /// Repo identifier.
    pub repo: Option<String>,
    /// Topics — empty for now (the lane doesn't carry topic
    /// metadata yet; the classifier worker fills this when spec 05
    /// lands).
    pub topics: Vec<String>,
    /// Relative wall-clock label (`"2 minutes ago"` / `"3 days ago"`)
    /// — empty when no timestamp was preserved.
    pub updated: String,
}

async fn memory(
    State(state): State<DashboardState>,
    Query(params): Query<MemoryQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let q = params.q.unwrap_or_default();

    let mut hits = collect_lane_hits(&state.lane);
    if let Some(sid) = params.session_id.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| session_id_of(h) == Some(sid));
    }
    if let Some(repo) = params.repo.as_deref().filter(|r| !r.is_empty()) {
        hits.retain(|h| h.repo.as_deref() == Some(repo));
    }
    if let Some(kind) = params.kind.as_deref().filter(|k| !k.is_empty()) {
        hits.retain(|h| symbol_to_kind(h.symbol.as_deref()) == kind);
    }
    if !q.trim().is_empty() {
        let needle = q.to_ascii_lowercase();
        hits.retain(|h| h.text.to_ascii_lowercase().contains(&needle));
    }
    hits.sort_by(|a, b| b.ts.cmp(&a.ts));
    hits.truncate(limit);

    let entries: Vec<MemoryEntry> = hits
        .into_iter()
        .map(|h| MemoryEntry {
            title: title_from_hit(&h),
            excerpt: clip(&h.text, 320),
            kind: symbol_to_kind(h.symbol.as_deref()).to_string(),
            repo: h.repo.clone(),
            topics: Vec::new(),
            updated: ts_to_relative(h.ts),
        })
        .collect();
    (StatusCode::OK, Json(entries)).into_response()
}

fn ts_to_relative(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let diff_ms = (now_ms - ts_ms).max(0);
    let secs = diff_ms / 1000;
    if secs < 60 {
        return format!("{secs}s ago");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn collect_lane_hits(lane: &MemoryKeywordLane) -> Vec<crate::lanes::LaneHit> {
    let g = match lane.hits.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    // The archive_loader seeds the same hits under three index aliases
    // (cortex-code / cortex-docs / cortex-decisions). Pick one — the
    // canonical one — to avoid triple-counting.
    g.get("cortex-code").cloned().unwrap_or_default()
}

/// Build a `QueryRequest` shaped for the dashboard memory endpoint.
/// Currently unused at the route level — the MVP filters in-process
/// against the seeded lane — but kept here so the §1 production
/// version can route through `QueryService::handle` without
/// re-deriving the request shape.
#[allow(dead_code)]
pub(crate) fn dashboard_memory_query(q: &str, limit: usize) -> QueryRequest {
    QueryRequest {
        intent: Intent::FreeSearch,
        scope: Scope::default(),
        query: q.to_string(),
        limit: limit.max(1),
        k: limit.max(1) * 2,
        include: vec![IncludeField::Snippets],
        budget_ms: 200,
    }
}

#[allow(dead_code)]
pub(crate) fn raw_value(v: &Value) -> Value {
    v.clone()
}

// ---------------------------------------------------------------------
// /v1/dashboard/decisions
// ---------------------------------------------------------------------

/// One decision row — shape matches the prototype's `MOCK.decisions`
/// minimum surface so the GUI can render the list / detail view.
/// Most fields are `None` until spec-15 (deep analysis) starts
/// producing structured `decision` envelopes.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionRow {
    /// Decision id (`DEC-NNNN` or ULID).
    pub id: String,
    /// Title — free text from the payload.
    pub title: String,
    /// `proposed` | `active` | `superseded` | `deprecated`.
    pub status: String,
    /// Author identifier (free text).
    pub author: Option<String>,
    /// Originating analysis id when known.
    pub source_analysis: Option<String>,
    /// Free-text rationale.
    pub rationale: Option<String>,
    /// Topic tags.
    pub tags: Vec<String>,
    /// Cross-references (specs / analyses / prior decisions).
    pub cites: Vec<String>,
    /// Id of the decision this one supersedes, if any.
    pub supersedes: Option<String>,
    /// Relative time label.
    pub occurred_at: String,
}

async fn decisions(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows: Vec<DecisionRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("decision"))
        .map(|h| DecisionRow {
            id: h
                .doc_id
                .strip_prefix("archive|")
                .unwrap_or(&h.doc_id)
                .to_string(),
            title: clip(h.text.lines().next().unwrap_or(""), 120),
            status: "active".to_string(),
            author: None,
            source_analysis: None,
            rationale: Some(clip(&h.text, 600)),
            tags: Vec::new(),
            cites: Vec::new(),
            supersedes: None,
            occurred_at: ts_to_relative(h.ts),
        })
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/laws
// ---------------------------------------------------------------------

/// One law row — shape mirrors the prototype's `MOCK.laws`. Real
/// data lands when spec-13 ships the law catalogue. Until then this
/// list is empty by design.
#[derive(Debug, Clone, Serialize)]
pub struct LawRow {
    /// `LAW-NNN`.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// `info` | `notable` | `critical`.
    pub severity: String,
    /// Whether the law actively blocks tool calls.
    pub blocked: bool,
    /// Free-text scope tags.
    pub scope: String,
    /// Number of contexts where the law applies.
    pub applies: u64,
    /// Violations recorded in the last 7 days.
    pub violations_7d: u64,
    /// Per-1000 violations rate.
    pub rate: f64,
    /// Detector identifier.
    pub detector: String,
    /// Suggested remediation.
    pub remediation: String,
}

async fn laws(State(_state): State<DashboardState>) -> Response {
    // Spec-13 owns the law catalogue. No envelope kind in spec-04
    // carries a law definition itself (only law_violation does), so
    // until spec-13 lands the answer is honest empty.
    let rows: Vec<LawRow> = Vec::new();
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/violations
// ---------------------------------------------------------------------

/// One law violation — shape mirrors `MOCK.violations`. Sourced from
/// `kind=law_violation` envelopes in the lane.
#[derive(Debug, Clone, Serialize)]
pub struct ViolationRow {
    /// Violation id.
    pub id: String,
    /// Law id the violation is tagged against.
    pub law_id: Option<String>,
    /// Relative time label.
    pub at: String,
    /// Repo where the violation was observed.
    pub repo: Option<String>,
    /// `blocked` | `annotated` | other.
    pub action: String,
    /// Evidence excerpt.
    pub evidence: String,
    /// Free-text remediation note.
    pub remediation: Option<String>,
}

async fn violations(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows: Vec<ViolationRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("law_violation"))
        .map(|h| ViolationRow {
            id: h
                .doc_id
                .strip_prefix("archive|")
                .unwrap_or(&h.doc_id)
                .to_string(),
            law_id: None,
            at: ts_to_relative(h.ts),
            repo: h.repo.clone(),
            action: "annotated".to_string(),
            evidence: clip(&h.text, 240),
            remediation: None,
        })
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/analyses
// ---------------------------------------------------------------------

/// One analysis row — shape mirrors `MOCK.analyses`. Sourced from
/// `kind=analysis` envelopes; populated when spec-15 (Deep Analysis
/// workflow) starts producing them.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisRow {
    /// Analysis id.
    pub id: String,
    /// Free-text title.
    pub title: String,
    /// `running` | `concluded` | `cancelled`.
    pub status: String,
    /// Panelist identifiers (model / agent names).
    pub panel: Vec<String>,
    /// Identifier of the judging model / human.
    pub judge: String,
    /// Number of debate rounds.
    pub rounds: u32,
    /// Total wall-clock duration in seconds.
    pub duration_s: u32,
    /// Final verdict body (Markdown).
    pub verdict: String,
    /// Decision id this analysis was promoted to (when applicable).
    pub decision_id: Option<String>,
    /// Relative time label.
    pub occurred_at: String,
}

async fn analyses(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows: Vec<AnalysisRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("analysis"))
        .map(|h| AnalysisRow {
            id: h
                .doc_id
                .strip_prefix("archive|")
                .unwrap_or(&h.doc_id)
                .to_string(),
            title: clip(h.text.lines().next().unwrap_or(""), 120),
            status: "concluded".to_string(),
            panel: Vec::new(),
            judge: String::new(),
            rounds: 0,
            duration_s: 0,
            verdict: clip(&h.text, 800),
            decision_id: None,
            occurred_at: ts_to_relative(h.ts),
        })
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/tools/stats
// ---------------------------------------------------------------------

/// Per-tool usage row — aggregated from `kind=tool_call` events
/// captured in the archive lane. Today populated by every PostToolUse
/// the spec-18 plugin emits.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStat {
    /// Tool name (`Edit`, `Read`, `Bash`, …).
    pub tool: String,
    /// Call count in the seeded window.
    pub calls: u64,
    /// Average duration in ms — placed at 0 until duration_ms is
    /// preserved through the lane (spec-12 derivation pipeline).
    pub avg_ms: u64,
    /// Error rate (0..1).
    pub err_rate: f64,
    /// Share of total calls (0..1).
    pub share: f64,
}

async fn tools_stats(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let mut by_tool: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    for h in &hits {
        if let Some(s) = h.symbol.as_deref() {
            if let Some(tool) = s.strip_prefix("tool_call:") {
                *by_tool.entry(tool.to_string()).or_insert(0) += 1;
            }
        }
    }
    let total: u64 = by_tool.values().sum();
    let total_f = if total == 0 { 1.0 } else { total as f64 };
    let mut rows: Vec<ToolStat> = by_tool
        .into_iter()
        .map(|(tool, calls)| ToolStat {
            tool,
            calls,
            avg_ms: 0,
            err_rate: 0.0,
            share: calls as f64 / total_f,
        })
        .collect();
    rows.sort_by(|a, b| b.calls.cmp(&a.calls));
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/graph
// ---------------------------------------------------------------------

/// Graph payload mirroring `MOCK.graph` — explicit `nodes` + `edges`
/// arrays the SPA's inline SVG renderer consumes directly.
#[derive(Debug, Clone, Serialize)]
pub struct GraphPayload {
    /// Nodes laid out in canvas-space.
    pub nodes: Vec<GraphNode>,
    /// Directed edges between node ids.
    pub edges: Vec<GraphEdge>,
}

/// One graph node.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    /// Node id (`session-...`, `turn-...`, `tool_call-...`, etc.).
    pub id: String,
    /// Display label.
    pub label: String,
    /// X coordinate in the SVG viewBox (0..820).
    pub x: i32,
    /// Y coordinate in the SVG viewBox (0..400).
    pub y: i32,
    /// `session` | `turn` | `tool_call` | `decision` | `law` | `violation` | `analysis` | `artifact`.
    pub kind: String,
}

/// One graph edge.
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Edge label (e.g. `INVOKED`, `WROTE`, `OBSERVED_IN`).
    pub label: String,
}

// ---------------------------------------------------------------------
// /v1/dashboard/sessions
// ---------------------------------------------------------------------

/// One session row — aggregated from the lane hits' `session_id`
/// extras stamp. The dashboard sidebar lists these so the user can
/// click to filter Timeline / Memory views.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// Canonical 26-char ULID.
    pub session_id: String,
    /// Total events captured under this session.
    pub event_count: u64,
    /// Per-kind breakdown.
    pub kind_breakdown: Vec<KindCount>,
    /// `ts` of the earliest event we have (ms epoch). 0 if missing.
    pub started_at_ms: i64,
    /// `ts` of the latest event we have (ms epoch).
    pub last_event_ms: i64,
    /// Duration in milliseconds between earliest and latest event.
    pub duration_ms: i64,
    /// Repos touched by this session.
    pub repos: Vec<String>,
    /// Title surface — first turn's user_message clipped at 80 chars.
    /// Empty when no turn was captured under this session.
    pub title: String,
}

async fn sessions(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Group by session_id.
    let mut groups: std::collections::BTreeMap<String, Vec<crate::lanes::LaneHit>> =
        std::collections::BTreeMap::new();
    for h in hits {
        if let Some(sid) = session_id_of(&h) {
            groups.entry(sid.to_string()).or_default().push(h);
        }
    }

    let mut rows: Vec<SessionRow> = groups
        .into_iter()
        .map(|(session_id, mut bucket)| {
            // Sort oldest → newest so [0] is the earliest event.
            bucket.sort_by(|a, b| a.ts.cmp(&b.ts));
            let started_at_ms = bucket.first().map(|h| h.ts).unwrap_or(0);
            let last_event_ms = bucket.last().map(|h| h.ts).unwrap_or(0);
            let duration_ms = (last_event_ms - started_at_ms).max(0);

            let mut by_kind: std::collections::BTreeMap<&'static str, u64> =
                std::collections::BTreeMap::new();
            let mut repos: std::collections::BTreeSet<String> = Default::default();
            let mut title = String::new();
            for h in &bucket {
                let kind = symbol_to_kind(h.symbol.as_deref());
                *by_kind.entry(kind).or_insert(0) += 1;
                if let Some(r) = h.repo.as_deref() {
                    repos.insert(r.to_string());
                }
                if title.is_empty() && kind == "turn" {
                    title = clip(h.text.lines().next().unwrap_or(""), 80);
                }
            }

            SessionRow {
                session_id,
                event_count: bucket.len() as u64,
                kind_breakdown: by_kind
                    .into_iter()
                    .map(|(k, count)| KindCount {
                        kind: k.to_string(),
                        count,
                    })
                    .collect(),
                started_at_ms,
                last_event_ms,
                duration_ms,
                repos: repos.into_iter().collect(),
                title,
            }
        })
        .collect();

    // Sort by most-recent-activity, descending.
    rows.sort_by(|a, b| b.last_event_ms.cmp(&a.last_event_ms));
    (StatusCode::OK, Json(rows)).into_response()
}

async fn graph(State(state): State<DashboardState>) -> Response {
    // Build a small graph from the seeded lane. We can reach Turn
    // and ToolCall nodes via the captured hits + the
    // `claude_code.turn_id` extras the spec-18 envelope writer
    // stamps. Each turn anchors under a synthetic Session node so
    // the renderer always shows a connected component.
    let hits = collect_lane_hits(&state.lane);

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let session_id = "session-active";
    nodes.push(GraphNode {
        id: session_id.to_string(),
        label: "Session".to_string(),
        x: 60,
        y: 200,
        kind: "session".to_string(),
    });

    let mut turns_seen: std::collections::BTreeMap<String, (i32, i32)> =
        std::collections::BTreeMap::new();
    let mut tool_calls_seen = 0i32;

    for (idx, h) in hits.iter().enumerate().take(12) {
        let kind = symbol_to_kind(h.symbol.as_deref());
        let label = title_from_hit(h);

        match kind {
            "turn" => {
                let id = format!("turn-{idx}");
                let y = 80 + (turns_seen.len() as i32) * 60;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 220,
                    y,
                    kind: kind.to_string(),
                });
                edges.push(GraphEdge {
                    from: session_id.to_string(),
                    to: id.clone(),
                    label: "CONTAINS".to_string(),
                });
                turns_seen.insert(id, (220, y));
            }
            "tool_call" => {
                let id = format!("tool_call-{idx}");
                let y = 80 + tool_calls_seen * 50;
                tool_calls_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 24),
                    x: 420,
                    y,
                    kind: kind.to_string(),
                });
                // Anchor the tool call under the most recent turn —
                // when no turn is around, hang it directly off the
                // session so the canvas stays connected.
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: parent,
                    to: id,
                    label: "INVOKED".to_string(),
                });
            }
            _ => {}
        }
    }

    (
        StatusCode::OK,
        Json(GraphPayload { nodes, edges }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::LaneHit;
    use std::collections::BTreeMap;

    fn lane_with(hits: Vec<LaneHit>) -> Arc<MemoryKeywordLane> {
        let lane = MemoryKeywordLane::new();
        lane.seed("cortex-code", hits);
        Arc::new(lane)
    }

    fn turn_hit(text: &str, repo: &str, ts: i64) -> LaneHit {
        turn_hit_in("session-default", text, repo, ts)
    }

    fn turn_hit_in(session: &str, text: &str, repo: &str, ts: i64) -> LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert("session_id".to_string(), Value::String(session.to_string()));
        LaneHit {
            doc_id: format!("archive|{}", text),
            text: text.to_string(),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
        }
    }

    fn tool_call_hit(tool: &str, body: &str, repo: &str, ts: i64) -> LaneHit {
        LaneHit {
            doc_id: format!("archive|{tool}-{ts}"),
            text: format!("[{tool}] {body}"),
            repo: Some(repo.to_string()),
            path: None,
            symbol: Some(format!("tool_call:{tool}")),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn overview_breaks_down_by_kind_and_repo() {
        let lane = lane_with(vec![
            turn_hit("hi", "Cortex", 100),
            turn_hit("again", "Cortex", 200),
            tool_call_hit("Edit", "stuff", "Vectorizer", 150),
        ]);
        let state = DashboardState { lane };
        let resp = overview(State(state)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["events_total"], 3);
        assert_eq!(parsed["repos_indexed"], 2);
        let kinds = parsed["kind_breakdown"].as_array().unwrap();
        let turn_count = kinds
            .iter()
            .find(|k| k["kind"] == "turn")
            .and_then(|k| k["count"].as_u64());
        assert_eq!(turn_count, Some(2));
        let tool_count = kinds
            .iter()
            .find(|k| k["kind"] == "tool_call")
            .and_then(|k| k["count"].as_u64());
        assert_eq!(tool_count, Some(1));
    }

    #[tokio::test]
    async fn timeline_recent_returns_newest_first_and_clips_titles() {
        let lane = lane_with(vec![
            turn_hit("oldest prompt", "Cortex", 100),
            turn_hit("middle prompt", "Cortex", 200),
            turn_hit("newest prompt", "Cortex", 300),
        ]);
        let state = DashboardState { lane };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(2),
                session_id: None,
                repo: None,
                kind: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["title"], "newest prompt");
        assert_eq!(parsed[1]["title"], "middle prompt");
        assert_eq!(parsed[0]["kind"], "turn");
    }

    #[tokio::test]
    async fn memory_filter_matches_q_substring_case_insensitive() {
        let lane = lane_with(vec![
            turn_hit("HNSW recall floor benchmark", "Vectorizer", 100),
            turn_hit("unrelated thoughts", "Cortex", 200),
        ]);
        let state = DashboardState { lane };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: Some("hnsw".to_string()),
                limit: None,
                session_id: None,
                repo: None,
                kind: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0]["title"].as_str().unwrap().contains("HNSW"));
    }

    #[tokio::test]
    async fn memory_with_empty_query_returns_everything_newest_first() {
        let lane = lane_with(vec![
            turn_hit("a", "Cortex", 100),
            turn_hit("b", "Cortex", 200),
        ]);
        let state = DashboardState { lane };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: None,
                kind: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["title"], "b");
    }

    #[tokio::test]
    async fn timeline_clamps_limit_to_max() {
        let lane = lane_with(
            (0..600)
                .map(|i| turn_hit(&format!("p{i}"), "X", i))
                .collect(),
        );
        let state = DashboardState { lane };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(99999),
                session_id: None,
                repo: None,
                kind: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 5 * 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 500);
    }

    #[tokio::test]
    async fn sessions_groups_by_session_id_and_sorts_by_recency() {
        let lane = lane_with(vec![
            turn_hit_in("01SESSIONA0000000000000001", "first ever", "Cortex", 100),
            turn_hit_in("01SESSIONA0000000000000001", "still session A", "Cortex", 200),
            turn_hit_in("01SESSIONB0000000000000002", "session B latest", "Vectorizer", 500),
        ]);
        let state = DashboardState { lane };
        let resp = sessions(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 2);
        // Most-recent first.
        assert_eq!(rows[0]["session_id"], "01SESSIONB0000000000000002");
        assert_eq!(rows[0]["event_count"], 1);
        assert_eq!(rows[0]["title"], "session B latest");
        assert_eq!(rows[1]["session_id"], "01SESSIONA0000000000000001");
        assert_eq!(rows[1]["event_count"], 2);
        assert_eq!(rows[1]["duration_ms"], 100);
        assert_eq!(rows[1]["title"], "first ever");
    }

    #[tokio::test]
    async fn timeline_filter_by_session_id_only_returns_matching_rows() {
        let lane = lane_with(vec![
            turn_hit_in("01SESSIONA0000000000000001", "alpha", "Cortex", 100),
            turn_hit_in("01SESSIONB0000000000000002", "beta", "Cortex", 200),
        ]);
        let state = DashboardState { lane };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: None,
                session_id: Some("01SESSIONB0000000000000002".to_string()),
                repo: None,
                kind: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["title"], "beta");
        assert_eq!(parsed[0]["session_id"], "01SESSIONB0000000000000002");
    }

    #[tokio::test]
    async fn memory_filter_by_repo_and_kind_combine() {
        let lane = lane_with(vec![
            turn_hit("note A", "Cortex", 100),
            tool_call_hit("Edit", "x", "Cortex", 200),
            turn_hit("note V", "Vectorizer", 150),
        ]);
        let state = DashboardState { lane };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Some("Cortex".to_string()),
                kind: Some("turn".to_string()),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["title"], "note A");
    }

    #[test]
    fn ts_to_relative_buckets_through_units() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(ts_to_relative(now), "0s ago");
        assert_eq!(ts_to_relative(now - 30_000), "30s ago");
        assert_eq!(ts_to_relative(now - 90_000), "1m ago");
        assert_eq!(ts_to_relative(now - 3_700_000), "1h ago");
        assert_eq!(ts_to_relative(now - 100_000_000), "1d ago");
    }
}
