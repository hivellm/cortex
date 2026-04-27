//! Dashboard backend (spec 16, MVP slice).
//!
//! Three read endpoints under `/v1/dashboard/*`. The Electron GUI in
//! `gui/` is the consumer — `cortex-api` does not serve any HTML or
//! JS itself; it only answers JSON. Production targets (SSE, OIDC,
//! the rest of the spec-16 surface) live under §1–§9 of
//! `phase2_dashboard/tasks.md`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum_extra::extract::Query;
use chrono::{Datelike, Timelike};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use nexus_sdk::{NexusClient, Value as NexusValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lanes::MemoryKeywordLane;
use crate::types::{IncludeField, Intent, QueryRequest, Scope};

/// Shared state for dashboard routes — the seeded keyword lane plus
/// an optional Nexus client used by the graph endpoint to run real
/// Cypher queries against the captured graph instead of falling back
/// to a synthetic Session→Turn→ToolCall layout.
#[derive(Clone)]
pub struct DashboardState {
    /// Keyword lane the archive_loader populates at boot + on
    /// refresh. Source of truth for the timeline / memory / decisions
    /// / analyses / violations / tools endpoints until the live
    /// spec-06 / spec-07 / spec-08 indexers ship.
    pub lane: Arc<MemoryKeywordLane>,
    /// Optional Nexus client. When `Some`, the `/v1/dashboard/graph`
    /// handler runs a Cypher MATCH against it; when `None`, the
    /// handler falls back to deriving a synthetic graph from the
    /// keyword lane so dev iterations without a live Nexus stay
    /// usable.
    pub nexus: Option<Arc<NexusClient>>,
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
        .route("/v1/dashboard/trust", get(trust))
        .route("/v1/dashboard/decisions/{id}", get(decision_detail))
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
    /// Time-bucketed series the GUI's stats grid renders. Only
    /// quantities derivable from the captured lane are emitted —
    /// see [`SeriesBlock`].
    pub series: SeriesBlock,
}

/// Time-bucketed series block. Each array carries a fixed number of
/// buckets, oldest-first, with no nulls — empty buckets are zero so
/// the front-end Sparkline renders a gap-free line. Only series that
/// can be computed from the captured envelopes are emitted; latency
/// and cost series will land when the spec-12 derivation pipeline
/// and the spec-05 classifier worker start stamping the
/// corresponding fields.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesBlock {
    /// Total events per minute over the last 20 minutes.
    pub events_per_min: Vec<u64>,
    /// Daily count of `kind=law_violation` envelopes over the last
    /// 7 days, oldest-first.
    pub violations_7d_daily: Vec<u64>,
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

    let now_ms = chrono::Utc::now().timestamp_millis();
    let series = SeriesBlock {
        events_per_min: bucket_per_minute(&snapshot, now_ms, 20),
        violations_7d_daily: bucket_violations_per_day(&snapshot, now_ms, 7),
    };

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
        series,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Bucket `hits` into `buckets` minute-wide slots ending at `now_ms`.
/// Slot 0 is the oldest (`now_ms - buckets * 60_000`), slot
/// `buckets - 1` is the most recent. Hits older than the window are
/// ignored; hits in the future (clock-skew) collapse into the last
/// slot.
fn bucket_per_minute(
    hits: &[crate::lanes::LaneHit],
    now_ms: i64,
    buckets: usize,
) -> Vec<u64> {
    let mut out = vec![0u64; buckets];
    let span_ms = (buckets as i64) * 60_000;
    let start_ms = now_ms - span_ms;
    for h in hits {
        if h.ts <= 0 {
            continue;
        }
        if h.ts < start_ms {
            continue;
        }
        let off = (h.ts - start_ms) / 60_000;
        let idx = if off < 0 {
            0
        } else if off >= buckets as i64 {
            buckets - 1
        } else {
            off as usize
        };
        out[idx] += 1;
    }
    out
}

/// Bucket `kind=law_violation` envelopes into `days` day-wide slots
/// ending at `now_ms`. Slot 0 is the oldest, slot `days - 1` is
/// today.
fn bucket_violations_per_day(
    hits: &[crate::lanes::LaneHit],
    now_ms: i64,
    days: usize,
) -> Vec<u64> {
    let mut out = vec![0u64; days];
    let day_ms: i64 = 86_400_000;
    let span_ms = (days as i64) * day_ms;
    let start_ms = now_ms - span_ms;
    for h in hits {
        if h.symbol.as_deref() != Some("law_violation") {
            continue;
        }
        if h.ts <= 0 || h.ts < start_ms {
            continue;
        }
        let off = (h.ts - start_ms) / day_ms;
        let idx = if off < 0 {
            0
        } else if off >= days as i64 {
            days - 1
        } else {
            off as usize
        };
        out[idx] += 1;
    }
    out
}

fn symbol_to_kind(symbol: Option<&str>) -> &'static str {
    match symbol {
        Some(s) if s.starts_with("tool_call") => "tool_call",
        Some(s) if s.starts_with("agent_call") => "agent_call",
        Some("decision") => "decision",
        Some("analysis") => "analysis",
        Some("law_violation") => "law_violation",
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
    /// Restrict to one or more repos. Each `repo=<name>` query param
    /// is appended; the filter passes when the hit matches ANY of
    /// the listed repos.
    #[serde(default)]
    pub repo: Vec<String>,
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
    if !params.repo.is_empty() {
        let allow: std::collections::HashSet<&str> =
            params.repo.iter().map(String::as_str).collect();
        hits.retain(|h| h.repo.as_deref().map(|r| allow.contains(r)).unwrap_or(false));
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
    /// Restrict to one or more repos. Each `repo=<name>` query param
    /// is appended; the filter passes when the hit matches ANY of
    /// the listed repos.
    #[serde(default)]
    pub repo: Vec<String>,
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
    if !params.repo.is_empty() {
        let allow: std::collections::HashSet<&str> =
            params.repo.iter().map(String::as_str).collect();
        hits.retain(|h| h.repo.as_deref().map(|r| allow.contains(r)).unwrap_or(false));
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
    /// Reverse pointer of `supersedes` — id of the decision that
    /// superseded this one. Computed by the dashboard from the
    /// captured set; not stamped on the envelope itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Linear supersession chain — oldest at index 0, current at the
    /// tail. Each node carries the decision id, its title, and its
    /// state (`current` or `old`). When the decision has no chain,
    /// this is an empty vec (length 0 or 1 means "no chain to draw"
    /// — the renderer hides the element).
    #[serde(default)]
    pub chain: Vec<DecisionChainNode>,
    /// Number of envelopes captured in the last 7 days that
    /// reference this decision id by `\bDEC-\d{4}-\d{3}\b` regex.
    pub cites_7d: u64,
    /// Relative time label.
    pub occurred_at: String,
}

/// One node of a [`DecisionRow::chain`].
#[derive(Debug, Clone, Serialize)]
pub struct DecisionChainNode {
    /// Decision id at this position.
    pub id: String,
    /// Title clipped at 80 chars.
    pub title: String,
    /// `current` (the row being rendered) or `old` (an ancestor).
    pub state: &'static str,
}

/// Optional body for the `/v1/dashboard/decisions/{id}` detail
/// endpoint — same shape as a list row plus the un-clipped Markdown
/// body.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionDetail {
    /// Spread of [`DecisionRow`].
    #[serde(flatten)]
    pub row: DecisionRow,
    /// Full envelope body.
    pub body_markdown: String,
}

/// Build the full set of [`DecisionRow`] from a snapshot of the
/// lane. Walking the supersedes pointer is local to this helper so
/// both `/decisions` and `/decisions/{id}` agree on chain layout.
fn build_decision_rows(hits: &[crate::lanes::LaneHit]) -> Vec<DecisionRow> {
    use std::collections::HashMap;

    // First pass: turn each `kind=decision` envelope into a (raw row,
    // raw body) pair so the chain pass below has a complete index
    // before walking pointers.
    let mut rows: Vec<DecisionRow> = Vec::new();
    let mut bodies: HashMap<String, String> = HashMap::new();
    for h in hits.iter().filter(|h| h.symbol.as_deref() == Some("decision")) {
        let id = h
            .doc_id
            .strip_prefix("archive|")
            .unwrap_or(&h.doc_id)
            .to_string();
        bodies.insert(id.clone(), h.text.clone());
        rows.push(DecisionRow {
            id,
            title: clip(h.text.lines().next().unwrap_or(""), 120),
            status: detect_status(&h.text),
            author: None,
            source_analysis: None,
            rationale: Some(clip(&h.text, 600)),
            tags: Vec::new(),
            cites: Vec::new(),
            supersedes: detect_supersedes(&h.text),
            superseded_by: None,
            chain: Vec::new(),
            cites_7d: 0,
            occurred_at: ts_to_relative(h.ts),
        });
    }

    // Reverse-pointer pass: for every row whose `supersedes` is set,
    // stamp the matching ancestor's `superseded_by`. Two rows that
    // both supersede the same id is a writer bug — last write wins.
    let mut superseded_by_index: HashMap<String, String> = HashMap::new();
    for row in &rows {
        if let Some(prev) = row.supersedes.as_ref() {
            superseded_by_index.insert(prev.clone(), row.id.clone());
        }
    }
    for row in rows.iter_mut() {
        if let Some(next) = superseded_by_index.get(&row.id) {
            row.superseded_by = Some(next.clone());
        }
    }

    // Status reflects the reverse-pointer index: a row with a
    // `superseded_by` is `superseded`, regardless of what the
    // payload claimed.
    for row in rows.iter_mut() {
        if row.superseded_by.is_some() && row.status != "superseded" {
            row.status = "superseded".to_string();
        }
    }

    // Chain pass: walk `supersedes` backwards from the head toward
    // the oldest decision in the chain, then reverse so the oldest
    // comes first. A chain with a single element is reported as an
    // empty vec so the renderer hides the supersede element. Indexes
    // are owned String→String so the chain build doesn't borrow from
    // `rows` while we mutate it.
    let title_index: HashMap<String, String> = rows
        .iter()
        .map(|r| (r.id.clone(), r.title.clone()))
        .collect();
    let supersedes_index: HashMap<String, String> = rows
        .iter()
        .filter_map(|r| r.supersedes.clone().map(|prev| (r.id.clone(), prev)))
        .collect();
    for i in 0..rows.len() {
        let head_id = rows[i].id.clone();
        let mut nodes: Vec<DecisionChainNode> = Vec::new();
        // Tail (current).
        nodes.push(DecisionChainNode {
            id: head_id.clone(),
            title: clip(
                title_index.get(&head_id).map(String::as_str).unwrap_or(""),
                80,
            ),
            state: "current",
        });
        // Walk backward through `supersedes` until either no parent
        // is set or a cycle is hit. The `seen` guard caps the walk.
        let mut cursor = head_id.clone();
        let mut seen: std::collections::HashSet<String> = Default::default();
        seen.insert(cursor.clone());
        while let Some(prev) = supersedes_index.get(&cursor).cloned() {
            if !seen.insert(prev.clone()) {
                break;
            }
            nodes.push(DecisionChainNode {
                id: prev.clone(),
                title: clip(
                    title_index.get(&prev).map(String::as_str).unwrap_or(""),
                    80,
                ),
                state: "old",
            });
            cursor = prev;
            if seen.len() > 32 {
                break;
            }
        }
        if nodes.len() <= 1 {
            // Single-element chains are the "no chain" case.
            continue;
        }
        nodes.reverse();
        rows[i].chain = nodes;
    }

    // Cites pass: count distinct lane hits within the last 7 days
    // whose body contains the decision id, excluding the decision's
    // own envelope. A regex match on the id is the cheapest signal
    // we have — the spec-12 derivation pipeline will refine this.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff_ms = now_ms - 7 * 86_400_000;
    for row in rows.iter_mut() {
        let needle = row.id.as_str();
        let mut count: u64 = 0;
        for h in hits {
            if h.ts <= 0 || h.ts < cutoff_ms {
                continue;
            }
            // Skip the decision's own envelope (the doc_id matches
            // `archive|<id>`).
            if h.doc_id
                .strip_prefix("archive|")
                .map(|s| s == needle)
                .unwrap_or(false)
            {
                continue;
            }
            if h.text.contains(needle) {
                count += 1;
            }
        }
        row.cites_7d = count;
    }

    let _ = bodies; // keep `bodies` available for the detail handler
    rows
}

/// Detect a `status:` line in a decision payload's first 32 lines.
/// Falls back to `active` when no marker is found.
fn detect_status(text: &str) -> String {
    for line in text.lines().take(32) {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("status:") {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    "active".to_string()
}

/// Detect a `supersedes: DEC-...` line. Returns the matched id.
fn detect_supersedes(text: &str) -> Option<String> {
    let re_target = "supersedes:";
    for line in text.lines().take(64) {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix(re_target) {
            let token = rest.trim().split_whitespace().next().unwrap_or("");
            if !token.is_empty() {
                // The lower-cased prefix gave us the offset; re-derive
                // the cased token from the original line so the id
                // keeps its `DEC-` casing.
                let cased = line
                    .trim()
                    .strip_prefix(|c: char| c.is_alphabetic() || c == ':')
                    .unwrap_or(line.trim())
                    .trim_start_matches(':')
                    .trim();
                let cased_token = cased.split_whitespace().next().unwrap_or(token);
                return Some(cased_token.to_string());
            }
        }
    }
    None
}

async fn decisions(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows = build_decision_rows(&hits);
    (StatusCode::OK, Json(rows)).into_response()
}

async fn decision_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows = build_decision_rows(&hits);
    let row = match rows.into_iter().find(|r| r.id == id) {
        Some(r) => r,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "reason": "decision_not_found" })),
            )
                .into_response();
        }
    };
    let body_markdown = hits
        .iter()
        .find(|h| {
            h.symbol.as_deref() == Some("decision")
                && h.doc_id.strip_prefix("archive|").unwrap_or(&h.doc_id) == row.id
        })
        .map(|h| h.text.clone())
        .unwrap_or_default();
    let detail = DecisionDetail { row, body_markdown };
    (StatusCode::OK, Json(detail)).into_response()
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

/// Top-level body of `/v1/dashboard/tools/stats`. Wraps the per-tool
/// rows the GUI table consumes plus the day×hour heatmap matrix the
/// design's Tool analytics view renders.
#[derive(Debug, Clone, Serialize)]
pub struct ToolsStatsBody {
    /// Per-tool aggregates, descending by call count.
    pub tools: Vec<ToolStat>,
    /// Tool-call density per (weekday, hour-of-day) over the last
    /// 7 days, UTC. `cells[d][h]` is the count for weekday `d` at
    /// hour `h`. Days follow ISO numbering (Monday = 0).
    pub heatmap: HeatmapBlock,
}

/// 7×24 heatmap of tool-call counts.
#[derive(Debug, Clone, Serialize)]
pub struct HeatmapBlock {
    /// Always `"UTC"` — buckets read off `chrono::DateTime<Utc>`.
    pub tz: &'static str,
    /// Day labels in display order, matching the row dimension of
    /// `cells`.
    pub days: [&'static str; 7],
    /// `[7][24]` tool-call counts. Outer index is weekday (0 = Mon),
    /// inner index is hour of day. Buckets with no calls are zero.
    pub cells: Vec<Vec<u64>>,
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

    let heatmap = build_tool_heatmap(&hits);
    let body = ToolsStatsBody {
        tools: rows,
        heatmap,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Bucket every `tool_call:*` envelope captured in the last 7 days
/// into a 7×24 grid of `(weekday, hour)` UTC counts. Hits with no
/// timestamp are dropped — they cannot be placed honestly.
fn build_tool_heatmap(hits: &[crate::lanes::LaneHit]) -> HeatmapBlock {
    let now = chrono::Utc::now();
    let cutoff_ms = now.timestamp_millis() - 7 * 86_400_000;
    let mut cells = vec![vec![0u64; 24]; 7];
    for h in hits {
        let symbol = match h.symbol.as_deref() {
            Some(s) if s.starts_with("tool_call:") => s,
            _ => continue,
        };
        let _ = symbol; // explicit kept-handle — keeps the filter readable
        if h.ts <= 0 || h.ts < cutoff_ms {
            continue;
        }
        let dt = match chrono::DateTime::<chrono::Utc>::from_timestamp_millis(h.ts) {
            Some(dt) => dt,
            None => continue,
        };
        let weekday = dt.weekday().num_days_from_monday() as usize;
        let hour = dt.hour() as usize;
        if weekday < 7 && hour < 24 {
            cells[weekday][hour] += 1;
        }
    }
    HeatmapBlock {
        tz: "UTC",
        days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        cells,
    }
}

// ---------------------------------------------------------------------
// /v1/dashboard/trust
// ---------------------------------------------------------------------

/// Trust matrix payload — model × repo cells with a `[0, 1]` score.
/// Spec 14 owns the actual computation (rolling violation rate per
/// (model, repo) pair). Until that ships the lane has no per-(model,
/// repo) trust signal, so we return empty arrays / map and the GUI
/// surfaces an empty state.
#[derive(Debug, Clone, Serialize)]
pub struct TrustMatrix {
    pub models: Vec<String>,
    pub repos: Vec<String>,
    /// `scores[model][repo]` → trust score in `[0, 1]`.
    pub scores: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
}

async fn trust(State(_state): State<DashboardState>) -> Response {
    let body = TrustMatrix {
        models: Vec::new(),
        repos: Vec::new(),
        scores: std::collections::BTreeMap::new(),
    };
    (StatusCode::OK, Json(body)).into_response()
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

/// Query params for `/v1/dashboard/graph`.
#[derive(Debug, Default, Deserialize)]
pub struct GraphQuery {
    /// Restrict to one session — when set, the Cypher MATCH anchors
    /// at `Session {session_id: $sid}`. When unset, the handler
    /// returns the most-recently-active subgraph capped at `limit`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Cap the total node count. Defaults to 60, max 200.
    #[serde(default)]
    pub limit: Option<usize>,
}

async fn graph(
    State(state): State<DashboardState>,
    Query(params): Query<GraphQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(60).clamp(1, 200);

    // Live path: when a Nexus client is configured, run a real
    // Cypher MATCH and convert the returned rows into the GraphPayload
    // shape the GUI consumes. On any failure (transport, schema, empty
    // result), fall through to the synthetic-from-lane fallback so a
    // dev environment without a populated Nexus still renders
    // something useful.
    if let Some(nx) = state.nexus.as_ref() {
        match query_nexus_graph(nx.as_ref(), params.session_id.as_deref(), limit).await {
            Ok(payload) if !payload.nodes.is_empty() => {
                return (StatusCode::OK, Json(payload)).into_response();
            }
            Ok(_) => {
                tracing::debug!("nexus returned an empty graph; falling back to lane synthesis");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "nexus graph query failed; falling back to lane synthesis"
                );
            }
        }
    }

    let payload = synthesize_graph_from_lane(&state.lane, limit);
    (StatusCode::OK, Json(payload)).into_response()
}

/// Run several Cypher MATCH passes against Nexus and assemble a
/// [`GraphPayload`].
///
/// Nexus 1.15 returns nodes as flat `{_nexus_id, ...properties}`
/// objects (no nested `properties` field, no `labels` array) and
/// returns relationships as `{_nexus_id, type}` (no `start`/`end`).
/// We therefore pull each label and edge type with explicit
/// `RETURN`-projected columns rather than parsing whole-node objects,
/// which means this code already works for the seven node labels and
/// three edge types the writer produces today (`Session`, `Turn`,
/// `Artifact`, `Repo`, `Memory`, `LawViolation`, `Decision` /
/// `IN_REPO`, `HAS_TURN`, `REMEMBERS`). New labels just need a fresh
/// pass below; new edges only need a fresh `MATCH ... RETURN`.
async fn query_nexus_graph(
    client: &NexusClient,
    session_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<GraphPayload> {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut seen_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_edges: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Per-label node pulls. Each query returns rows of two cells:
    // `[<key>, <display_label>]` — the rest of the property bag stays
    // server-side. Limit-per-label keeps the canvas legible while
    // still giving every kind a chance to appear.
    let per_label_limit = (limit / 2).max(20).min(limit);
    // Order: small / interesting kinds first so a tight `limit` still
    // surfaces every label rather than getting drowned by Artifacts.
    let label_specs: &[(&str, &str, &str, &str)] = &[
        // (cypher_label, return_id_prop, return_label_prop, gui_kind)
        ("Repo", "name", "name", "repo"),
        ("Decision", "id", "title", "decision"),
        ("Law", "id", "title", "law"),
        ("Analysis", "id", "title", "analysis"),
        ("LawViolation", "id", "law_id", "violation"),
        ("Memory", "id", "id", "memory"),
        ("Session", "id", "id", "session"),
        ("Turn", "id", "id", "turn"),
        ("ToolCall", "id", "tool_name", "tool_call"),
        ("AgentCall", "id", "agent_type", "agent_call"),
        ("Artifact", "natural_key", "path", "artifact"),
    ];
    for (cypher_label, id_prop, label_prop, gui_kind) in label_specs {
        // Anchor at a session when one is supplied — restricts to the
        // session's first-degree neighbourhood so the canvas stays
        // focused. Otherwise pull a recent slice of the label.
        let cy = if session_id.is_some() && *cypher_label != "Session" {
            format!(
                "MATCH (s:Session {{ session_id: $sid }})-[]->(n:{cypher_label}) \
                 RETURN n.{id_prop} AS id, n.{label_prop} AS label \
                 LIMIT $lim"
            )
        } else if session_id.is_some() {
            format!(
                "MATCH (n:{cypher_label} {{ session_id: $sid }}) \
                 RETURN n.{id_prop} AS id, n.{label_prop} AS label \
                 LIMIT $lim"
            )
        } else {
            format!(
                "MATCH (n:{cypher_label}) \
                 RETURN n.{id_prop} AS id, n.{label_prop} AS label \
                 LIMIT $lim"
            )
        };
        let mut params: std::collections::HashMap<String, NexusValue> =
            std::collections::HashMap::new();
        params.insert(
            "lim".to_string(),
            NexusValue::Int(per_label_limit as i64),
        );
        if let Some(sid) = session_id {
            params.insert("sid".to_string(), NexusValue::String(sid.to_string()));
        }
        let res = match client.execute_cypher(&cy, Some(params)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    cypher_label,
                    error = %e,
                    "nexus label pull failed; skipping this label"
                );
                continue;
            }
        };
        for row in &res.rows {
            let cells = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            let id = cells
                .first()
                .and_then(|v| v.as_str().map(String::from).or_else(|| {
                    if v.is_null() {
                        None
                    } else {
                        Some(v.to_string())
                    }
                }))
                .unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let label = cells
                .get(1)
                .and_then(|v| v.as_str().map(String::from).or_else(|| {
                    if v.is_null() {
                        None
                    } else {
                        Some(v.to_string())
                    }
                }))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone());
            if seen_nodes.insert(id.clone()) {
                nodes.push(GraphNode {
                    id,
                    label: clip(&label, 64),
                    x: 0,
                    y: 0,
                    kind: gui_kind.to_string(),
                });
            }
            if nodes.len() >= limit {
                break;
            }
        }
        if nodes.len() >= limit {
            break;
        }
    }

    // Edge pulls — one MATCH per relationship type. Each query
    // returns the ID of each endpoint via the same property the node
    // pull above used as `id`, so dedup by that string lines up.
    let edge_specs: &[(&str, &str, &str, &str, &str)] = &[
        ("Session", "id", "HAS_TURN", "Turn", "id"),
        ("Session", "id", "REMEMBERS", "Memory", "id"),
        ("Artifact", "natural_key", "IN_REPO", "Repo", "name"),
        ("Turn", "id", "HAS_TOOL_CALL", "ToolCall", "id"),
        ("Turn", "id", "HAS_AGENT_CALL", "AgentCall", "id"),
        ("ToolCall", "id", "TOUCHED", "Artifact", "natural_key"),
        ("LawViolation", "id", "OBSERVED_IN", "Turn", "id"),
        ("LawViolation", "id", "OF", "Law", "id"),
        ("Decision", "id", "SUPERSEDES", "Decision", "id"),
    ];
    for (from_label, from_prop, rel, to_label, to_prop) in edge_specs {
        let cy = format!(
            "MATCH (a:{from_label})-[r:{rel}]->(b:{to_label}) \
             RETURN a.{from_prop} AS from_id, b.{to_prop} AS to_id \
             LIMIT $lim"
        );
        let mut params: std::collections::HashMap<String, NexusValue> =
            std::collections::HashMap::new();
        params.insert("lim".to_string(), NexusValue::Int(limit as i64));
        let res = match client.execute_cypher(&cy, Some(params)).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        for row in &res.rows {
            let cells = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            let from = cells.first().and_then(|v| v.as_str()).unwrap_or_default();
            let to = cells.get(1).and_then(|v| v.as_str()).unwrap_or_default();
            if from.is_empty() || to.is_empty() {
                continue;
            }
            // Skip edges to nodes the canvas isn't rendering.
            if !seen_nodes.contains(from) || !seen_nodes.contains(to) {
                continue;
            }
            let key = format!("{from}|{rel}|{to}");
            if seen_edges.insert(key) {
                edges.push(GraphEdge {
                    from: from.to_string(),
                    to: to.to_string(),
                    label: rel.to_string(),
                });
            }
        }
    }

    Ok(GraphPayload { nodes, edges })
}

/// Accumulator that dedups Nexus nodes/edges by id while turning
/// untyped `serde_json::Value` cells into typed [`GraphNode`] /
/// [`GraphEdge`]. Nexus returns nodes with shape
/// `{ "labels": [..], "properties": {..}, "_id": .. }` and edges
/// with shape `{ "type": "...", "start": .., "end": .., ... }` —
/// both surface here as `Value::Object`.
#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    seen_nodes: std::collections::HashSet<String>,
    seen_edges: std::collections::HashSet<String>,
}

impl GraphBuilder {
    fn add_node(&mut self, cell: Option<&Value>, default_kind: &str) {
        let obj = match cell.and_then(|v| v.as_object()) {
            Some(o) => o,
            None => return,
        };
        let props = obj
            .get("properties")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let id = match props
            .get("session_id")
            .or_else(|| props.get("turn_id"))
            .or_else(|| props.get("tool_call_id"))
            .or_else(|| props.get("event_id"))
            .or_else(|| obj.get("_id"))
            .and_then(|v| v.as_str().map(String::from).or_else(|| Some(v.to_string())))
        {
            Some(s) if !s.is_empty() => s,
            _ => return,
        };
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let kind = obj
            .get("labels")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(label_to_kind)
            .unwrap_or_else(|| default_kind.to_string());
        let label = node_label(&kind, &props, &id);
        // x/y are unused by the Cytoscape renderer (cose layout owns
        // positioning) but kept in the shape so older clients still
        // round-trip the response.
        self.nodes.push(GraphNode {
            id,
            label,
            x: 0,
            y: 0,
            kind,
        });
    }

    fn add_edge(&mut self, cell: Option<&Value>) {
        let obj = match cell.and_then(|v| v.as_object()) {
            Some(o) => o,
            None => return,
        };
        let from = obj.get("start").and_then(|v| v.as_str()).map(String::from);
        let to = obj.get("end").and_then(|v| v.as_str()).map(String::from);
        let rel = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("REL")
            .to_string();
        let (from, to) = match (from, to) {
            (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => (a, b),
            _ => return,
        };
        let key = format!("{from}|{rel}|{to}");
        if !self.seen_edges.insert(key) {
            return;
        }
        self.edges.push(GraphEdge {
            from,
            to,
            label: rel,
        });
    }

    fn into_payload(mut self, limit: usize) -> GraphPayload {
        if self.nodes.len() > limit {
            self.nodes.truncate(limit);
            let kept: std::collections::HashSet<&String> =
                self.nodes.iter().map(|n| &n.id).collect();
            self.edges
                .retain(|e| kept.contains(&e.from) && kept.contains(&e.to));
        }
        GraphPayload {
            nodes: self.nodes,
            edges: self.edges,
        }
    }
}

/// Map a Nexus node label (`Session`, `Turn`, `ToolCall`, …) to the
/// kebab kind the GUI consumes (`session`, `turn`, `tool_call`, …).
fn label_to_kind(label: &str) -> String {
    match label {
        "Session" => "session".to_string(),
        "Turn" => "turn".to_string(),
        "ToolCall" => "tool_call".to_string(),
        "AgentCall" => "agent_call".to_string(),
        "Decision" => "decision".to_string(),
        "Law" => "law".to_string(),
        "LawViolation" => "violation".to_string(),
        "Analysis" => "analysis".to_string(),
        "Artifact" => "artifact".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

/// Pick a human-readable label for a node based on the kind + props.
fn node_label(
    kind: &str,
    props: &serde_json::Map<String, Value>,
    fallback_id: &str,
) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| props.get(*k).and_then(|v| v.as_str()))
            .map(|s| clip(s, 48))
    };
    // The graph writer (cortex-graph mapper) now stamps a `name`
    // prop on every node — it's the single human-readable label
    // every kind ends up carrying. Prefer it; fall back to the
    // payload-specific fields only when `name` is absent (older
    // nodes written before the labelling change).
    if let Some(name) = props.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            return clip(name, 96);
        }
    }
    match kind {
        "session" => pick(&["title", "session_id"]).unwrap_or_else(|| "Session".to_string()),
        "turn" => pick(&["user_message", "text"]).unwrap_or_else(|| "Turn".to_string()),
        "tool_call" => pick(&["tool_name"])
            .map(|s| format!("[{s}]"))
            .unwrap_or_else(|| "ToolCall".to_string()),
        "agent_call" => pick(&["agent_type"])
            .map(|s| format!("Task: {s}"))
            .unwrap_or_else(|| "AgentCall".to_string()),
        "decision" => pick(&["title", "decision_id"]).unwrap_or_else(|| "Decision".to_string()),
        "analysis" => pick(&["title", "analysis_id"]).unwrap_or_else(|| "Analysis".to_string()),
        "law" => pick(&["title", "law_id"]).unwrap_or_else(|| "Law".to_string()),
        "violation" => pick(&["message", "law_id"]).unwrap_or_else(|| "Violation".to_string()),
        "artifact" => pick(&["path"]).unwrap_or_else(|| fallback_id.to_string()),
        _ => fallback_id.to_string(),
    }
}

/// Original lane-only graph synthesis — used as the fallback when no
/// Nexus client is configured or the live query fails.
fn synthesize_graph_from_lane(
    lane: &MemoryKeywordLane,
    limit: usize,
) -> GraphPayload {
    let hits = collect_lane_hits(lane);

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
    let mut decisions_seen = 0i32;
    let mut analyses_seen = 0i32;
    let mut violations_seen = 0i32;
    let cap = limit.saturating_sub(1).min(60);

    for (idx, h) in hits.iter().enumerate().take(cap) {
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
            "decision" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("decision-{raw_id}");
                let y = 80 + decisions_seen * 60;
                decisions_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 720,
                    y,
                    kind: kind.to_string(),
                });
                // Anchor the decision under the most recent turn —
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
                    label: "REFERENCES".to_string(),
                });
            }
            "analysis" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("analysis-{raw_id}");
                let y = 220 + analyses_seen * 60;
                analyses_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 32),
                    x: 720,
                    y,
                    kind: kind.to_string(),
                });
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: parent,
                    to: id,
                    label: "PRODUCED".to_string(),
                });
            }
            "law_violation" => {
                let raw_id = h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string();
                let id = format!("violation-{raw_id}");
                let y = 320 + violations_seen * 50;
                violations_seen += 1;
                nodes.push(GraphNode {
                    id: id.clone(),
                    label: clip(&label, 28),
                    x: 540,
                    y,
                    kind: "violation".to_string(),
                });
                let parent = turns_seen
                    .keys()
                    .next_back()
                    .cloned()
                    .unwrap_or_else(|| session_id.to_string());
                edges.push(GraphEdge {
                    from: id.clone(),
                    to: parent,
                    label: "OBSERVED_IN".to_string(),
                });
            }
            _ => {}
        }
    }

    GraphPayload { nodes, edges }
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
        let state = DashboardState { lane, nexus: None };
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
        let state = DashboardState { lane, nexus: None };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(2),
                session_id: None,
                repo: Vec::new(),
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
        let state = DashboardState { lane, nexus: None };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: Some("hnsw".to_string()),
                limit: None,
                session_id: None,
                repo: Vec::new(),
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
        let state = DashboardState { lane, nexus: None };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: Vec::new(),
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
        let state = DashboardState { lane, nexus: None };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(99999),
                session_id: None,
                repo: Vec::new(),
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
        let state = DashboardState { lane, nexus: None };
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
        let state = DashboardState { lane, nexus: None };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: None,
                session_id: Some("01SESSIONB0000000000000002".to_string()),
                repo: Vec::new(),
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
        let state = DashboardState { lane, nexus: None };
        let resp = memory(
            State(state),
            Query(MemoryQuery {
                q: None,
                limit: None,
                session_id: None,
                repo: vec!["Cortex".to_string()],
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
