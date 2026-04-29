//! Dashboard backend (spec 16, MVP slice).
//!
//! Three read endpoints under `/v1/dashboard/*`. The Electron GUI in
//! `gui/` is the consumer — `cortex-api` does not serve any HTML or
//! JS itself; it only answers JSON. Production targets (SSE, OIDC,
//! the rest of the spec-16 surface) live under §1–§9 of
//! `phase2_dashboard/tasks.md`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum_extra::extract::Query;
use chrono::{Datelike, Timelike};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::Stream;
use nexus_sdk::{NexusClient, Value as NexusValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lanes::MemoryKeywordLane;
use crate::tasks_loader::{ListQuery, SortField, SortOrder, TaskLoader};
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
    /// Sonnet-backed session analyzer. Shared so the in-memory
    /// summary cache survives across requests; built once at boot.
    pub analyzer: Arc<crate::analyzer::Analyzer>,
    /// Rulebook task loader. Backs the `/v1/dashboard/tasks*`
    /// endpoints. When the workspace root is unreachable the loader
    /// transparently yields empty results — cold-stack dev keeps
    /// booting.
    pub tasks: Arc<TaskLoader>,
    /// SQLite metadata store. When `Some`, the overview handler
    /// reads `classifier_spend_hourly` to feed the dashboard's
    /// `series.classifier_cost_usd_today` ribbon. `None` keeps the
    /// stubbed all-zeros behaviour. Wrapped in [`std::sync::Mutex`]
    /// because `rusqlite::Connection` is `Send` but not `Sync`, and
    /// dashboard handlers are dispatched across the tokio runtime.
    pub metadata: Option<Arc<std::sync::Mutex<cortex_storage::MetadataStore>>>,
    /// Phase8b — shared loader metrics registry. Bumped from the
    /// archive_loader + meili_loader refresh tasks; surfaced via
    /// `/healthz` extras and the new `/v1/health/freshness`
    /// endpoint so a stalled loader localises immediately.
    pub loader_metrics: Arc<crate::LoaderMetrics>,
}

/// Build the dashboard sub-router carrying the `/v1/dashboard/*` JSON
/// endpoints the GUI consumes. Endpoints whose upstream subsystem is
/// not built yet (laws / decisions / analyses — specs 13/14/15) still
/// answer with an honest empty list rather than mocked rows.
pub fn build_dashboard_router(state: DashboardState) -> Router {
    Router::new()
        .route("/v1/dashboard/overview", get(overview))
        .route("/v1/dashboard/timeline/recent", get(timeline_recent))
        .route("/v1/dashboard/timeline/stream", get(timeline_stream))
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
        .route("/v1/dashboard/conversations", get(conversations_list))
        .route("/v1/dashboard/conversations/{session_id}", get(conversation_detail))
        .route(
            "/v1/dashboard/conversations/{session_id}/summary",
            get(conversation_summary),
        )
        .route("/v1/dashboard/handoffs", get(handoffs))
        .route("/v1/dashboard/classifications", get(classifications))
        .route("/v1/dashboard/tasks", get(tasks_list))
        .route("/v1/dashboard/tasks/summary", get(tasks_summary))
        .route("/v1/dashboard/tasks/{id}", get(tasks_detail))
        .route("/v1/retention/sweeps", get(retention_sweeps))
        .route("/v1/retention/state", get(retention_state))
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
    /// Honest "we don't have this signal yet" flag for the
    /// `series.classifier_cost_usd_today` ribbon. Always `true`
    /// today; flips to `false` once the spec-05 classifier worker
    /// stamps per-event cost on the lane and the API can sum them.
    /// The wire shape stays stable across the cut-over so clients
    /// don't need a v2 endpoint.
    pub classifier_cost_unavailable_until_spec05: bool,
}

/// Time-bucketed series block. Each array carries a fixed number of
/// buckets, oldest-first. `events_per_min` and `violations_7d_daily`
/// have no nulls — empty buckets are zero so the front-end Sparkline
/// renders a gap-free line. `pre_thinking_p95_ms` carries `null`
/// for buckets where no envelope landed with a duration stamp so
/// the renderer draws a gap rather than a dishonest zero.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesBlock {
    /// Total events per minute over the last 20 minutes.
    pub events_per_min: Vec<u64>,
    /// P95 duration (ms) per minute over the last 20 minutes. The
    /// signal source is `tool_call` + `agent_call` envelopes that
    /// carry `duration_ms`; turns alone don't populate the series
    /// today (spec-12 derivation pipeline owns turn-level latency
    /// and will widen the source set when it ships).
    pub pre_thinking_p95_ms: Vec<Option<u64>>,
    /// Daily count of `kind=law_violation` envelopes over the last
    /// 7 days, oldest-first.
    pub violations_7d_daily: Vec<u64>,
    /// Hourly classifier cost (USD) over the rolling 24 hours,
    /// oldest-first. All zero today — see
    /// `classifier_cost_unavailable_until_spec05` on the parent
    /// body for context.
    pub classifier_cost_usd_today: Vec<f64>,
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

    let now = chrono::Utc::now();
    let now_ms = now.timestamp_millis();
    // Pull the rolling-24h cost ribbon from the metadata store when
    // it's wired. `is_stub = true` means "store unwired or empty
    // window" — the GUI keeps showing "—" instead of $0.00 in that
    // case. Once the classifier-worker has booked at least one
    // classification in the last 24h, the flag flips to `false` and
    // the ribbon renders real numbers.
    let (classifier_cost_series, classifier_cost_stub) = match state.metadata.as_ref() {
        Some(store) => {
            let guard = match store.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            crate::dashboard_series::read_classifier_cost_24h(&guard, now)
        }
        None => (crate::dashboard_series::classifier_cost_zeros(), true),
    };
    let series = SeriesBlock {
        events_per_min: crate::dashboard_series::bucket_per_minute(&snapshot, now_ms, 20),
        pre_thinking_p95_ms: crate::dashboard_series::bucket_p95_duration_per_minute(
            &snapshot, now_ms, 20,
        ),
        violations_7d_daily: crate::dashboard_series::bucket_violations_per_day(
            &snapshot, now_ms, 7,
        ),
        classifier_cost_usd_today: classifier_cost_series,
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
        classifier_cost_unavailable_until_spec05: classifier_cost_stub,
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn symbol_to_kind(symbol: Option<&str>) -> &'static str {
    match symbol {
        Some(s) if s.starts_with("tool_call") => "tool_call",
        Some(s) if s.starts_with("agent_call") => "agent_call",
        Some("decision") => "decision",
        Some("analysis") => "analysis",
        Some("law_violation") => "law_violation",
        // `memory` is the canonical kind the meili_loader stamps on
        // hits projected from `.rulebook/{handoff,specs,knowledge,
        // learnings}/**` — without this branch they collapsed into
        // "turn" and the Handoffs endpoint never matched them.
        Some("memory") => "memory",
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
    /// Phase3 — filter to rows whose `content_hash` matches the
    /// supplied value verbatim (full `sha256:<64hex>` form). Powers
    /// the Inspector's "show every call with this fingerprint"
    /// workflow.
    #[serde(default)]
    pub content_hash: Option<String>,
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
    /// Phase3 — sha256 fingerprint of the captured envelope
    /// (`sha256:<64hex>`). Pass-through from `LaneHit.content_hash`;
    /// dropped for redacted hits per `redaction.rs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Phase3 — un-clipped tool-call body. Capped at
    /// [`PREVIEW_BYTE_CAP`] (8 KiB) so a 200-row response stays
    /// under ~2 MiB; rows larger than that get
    /// `preview_truncated = true` and the GUI fetches the full text
    /// via the per-id timeline route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Phase3 — `true` when the original `LaneHit.text` exceeded
    /// [`PREVIEW_BYTE_CAP`] and the field was clipped on its char
    /// boundary. Dropped from the wire when `false` so non-tool_call
    /// rows stay lean.
    #[serde(default, skip_serializing_if = "is_false")]
    pub preview_truncated: bool,
}

/// Hard cap on `TimelineEvent.preview` — 8 KiB matches the
/// proposal's bandwidth budget (≈2 MiB for a 200-row response).
pub const PREVIEW_BYTE_CAP: usize = 8 * 1024;

fn is_false(b: &bool) -> bool {
    !*b
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
    if let Some(hash) = params.content_hash.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| h.content_hash.as_deref() == Some(hash));
    }
    // Newest first by `ts`.
    hits.sort_by(|a, b| b.ts.cmp(&a.ts));
    hits.truncate(limit);

    let events: Vec<TimelineEvent> = hits.iter().map(build_timeline_event).collect();
    (StatusCode::OK, Json(events)).into_response()
}

/// `GET /v1/dashboard/timeline/stream` — SSE stream of timeline
/// events. Each new envelope visible to the lane fans out as one
/// `event: timeline` frame; a periodic `event: heartbeat` is emitted
/// every 15 seconds so the client can flip a "stale" pill when the
/// server stops talking.
///
/// Reconnect contract: every event carries `id: <doc_id>`. On
/// reconnect the browser sends `Last-Event-ID`; the handler then
/// emits any envelopes newer than that id (best-effort — the lane
/// is in-memory, so an id older than the current snapshot just
/// drops back to the live tail).
///
/// Filters via `?repo`, `?session_id`, `?kind` honour the same shape
/// as `/timeline/recent`. The handler polls the lane every 500 ms
/// and diffs against the per-subscriber seen-id set so each
/// connection sees a clean per-session timeline.
async fn timeline_stream(
    State(state): State<DashboardState>,
    Query(params): Query<TimelineQuery>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let lane = state.lane.clone();
    let session_filter = params
        .session_id
        .clone()
        .filter(|s| !s.is_empty());
    let repo_filter: std::collections::HashSet<String> =
        params.repo.iter().cloned().collect();
    let kind_filter = params.kind.clone().filter(|s| !s.is_empty());
    let content_hash_filter = params.content_hash.clone().filter(|s| !s.is_empty());

    // Per-subscriber loop. Polls the in-memory lane every 500 ms and
    // emits the diff against the previously-seen ids. Heartbeat
    // every 15 s decouples liveness signal from event volume.
    let stream = async_stream::stream! {
        // Prime the seen-ids set with whatever the lane has now,
        // optionally rewinding to `Last-Event-ID` so the client gets
        // events newer than that point on reconnect. Without rewind,
        // we'd flash the entire backfill every time the user
        // reloaded the GUI.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let initial_hits = filtered_hits(&lane, session_filter.as_deref(), &repo_filter, kind_filter.as_deref(), content_hash_filter.as_deref());
        let cutoff_ts: Option<i64> = match last_event_id.as_deref() {
            Some(id) => initial_hits
                .iter()
                .find(|h| h.doc_id == id)
                .map(|h| h.ts),
            None => None,
        };
        for h in &initial_hits {
            if let Some(t) = cutoff_ts {
                if h.ts > t {
                    let event = build_timeline_event(h);
                    yield encode_sse(&event);
                }
            }
            seen.insert(h.doc_id.clone());
        }

        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        heartbeat.tick().await; // skip the immediate first tick
        let mut poll = tokio::time::interval(Duration::from_millis(500));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = poll.tick() => {
                    let snapshot = filtered_hits(&lane, session_filter.as_deref(), &repo_filter, kind_filter.as_deref(), content_hash_filter.as_deref());
                    for h in snapshot {
                        if seen.insert(h.doc_id.clone()) {
                            let event = build_timeline_event(&h);
                            yield encode_sse(&event);
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok::<SseEvent, Infallible>(
                        SseEvent::default()
                            .event("heartbeat")
                            .data(r#"{"ok":true}"#)
                    );
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Apply the same `(session_id, repo, kind)` filters the polling
/// `/timeline/recent` handler uses, in the same order, so both
/// surfaces agree on which envelopes are visible to a given query.
/// Returns the result sorted oldest-first so the SSE stream emits
/// envelopes in chronological order on first paint.
fn filtered_hits(
    lane: &crate::lanes::MemoryKeywordLane,
    session_filter: Option<&str>,
    repo_filter: &std::collections::HashSet<String>,
    kind_filter: Option<&str>,
    content_hash_filter: Option<&str>,
) -> Vec<crate::lanes::LaneHit> {
    let mut hits = collect_lane_hits(lane);
    if let Some(sid) = session_filter {
        hits.retain(|h| session_id_of(h) == Some(sid));
    }
    if !repo_filter.is_empty() {
        hits.retain(|h| h.repo.as_deref().map(|r| repo_filter.contains(r)).unwrap_or(false));
    }
    if let Some(kind) = kind_filter {
        hits.retain(|h| symbol_to_kind(h.symbol.as_deref()) == kind);
    }
    if let Some(hash) = content_hash_filter {
        hits.retain(|h| h.content_hash.as_deref() == Some(hash));
    }
    hits.sort_by(|a, b| a.ts.cmp(&b.ts));
    hits
}

fn build_timeline_event(h: &crate::lanes::LaneHit) -> TimelineEvent {
    let kind = symbol_to_kind(h.symbol.as_deref()).to_string();
    // Phase3 — `preview` is the un-clipped body so the Inspector can
    // render the full edit/diff/script. Cap at PREVIEW_BYTE_CAP and
    // flip `preview_truncated` when the source overflowed; non-tool_call
    // rows skip the field entirely so the wire stays compact.
    let (preview, preview_truncated) = if kind == "tool_call" && !h.text.is_empty() {
        if h.text.len() <= PREVIEW_BYTE_CAP {
            (Some(h.text.clone()), false)
        } else {
            (Some(clip(&h.text, PREVIEW_BYTE_CAP)), true)
        }
    } else {
        (None, false)
    };
    TimelineEvent {
        id: h.doc_id.clone(),
        t: ts_to_clock_string(h.ts),
        kind,
        title: title_from_hit(h),
        detail: clip(&h.text, 280),
        repo: h.repo.clone(),
        session_id: session_id_of(h).map(String::from),
        model: "claude-code".to_string(),
        content_hash: h.content_hash.clone(),
        preview,
        preview_truncated,
    }
}

fn encode_sse(event: &TimelineEvent) -> Result<SseEvent, Infallible> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    Ok(SseEvent::default()
        .id(event.id.clone())
        .event("timeline")
        .data(payload))
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
    // Aggregate every seeded index, deduping by `doc_id`.
    //
    // The earlier comment claimed each seed is a "complete snapshot
    // for its family" so flat-chaining was safe — that claim is
    // wrong. `archive_loader::load_into_keyword_lane` deliberately
    // fans the same hit list into three aliases (`cortex-code`,
    // `cortex-docs`, `cortex-decisions`) so spec-11 free-search /
    // pre-change-context strategies hit non-empty results regardless
    // of which alias they look up. Without dedup the dashboard
    // timeline shows every event 3x; the user spotted this on
    // 2026-04-28. Keeping the first occurrence preserves the lane's
    // existing per-alias query semantics while collapsing the
    // duplicates the dashboard never wanted.
    let mut out: Vec<crate::lanes::LaneHit> = Vec::new();
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for hits in g.values() {
        for h in hits {
            if seen.insert(h.doc_id.clone()) {
                out.push(h.clone());
            }
        }
    }
    out
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
    /// Repo this decision belongs to (the project that owns the
    /// `.rulebook/decisions/*.md` file). Multiple Hive repos ship
    /// their own ADRs — without this field the dashboard can't
    /// tell whether a decision is Cortex's, Nexus's, or another
    /// project's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
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
        // The meili_loader stamps cleaner fields on extras (parsed
        // from the JSON-encoded envelope body the fulltext worker
        // stores). Fall back to the legacy text-scraping path so
        // archive-fed envelopes — Turn / ToolCall / AgentCall paths
        // through detect_status() — still parse identically.
        let extras_title = h.extras.get("title").and_then(|v| v.as_str());
        let extras_status = h.extras.get("status").and_then(|v| v.as_str());
        let extras_supersedes = h
            .extras
            .get("supersedes")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let extras_body = h.extras.get("body_markdown").and_then(|v| v.as_str());
        let title_clean = match extras_title {
            Some(t) if !t.is_empty() => clip(t, 120),
            _ => clip(h.text.lines().next().unwrap_or(""), 120),
        };
        let rationale = match extras_body {
            Some(b) if !b.is_empty() => Some(clip(b, 600)),
            _ => Some(clip(&h.text, 600)),
        };
        let status = match extras_status {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => detect_status(&h.text),
        };
        let supersedes = extras_supersedes.or_else(|| detect_supersedes(&h.text));

        bodies.insert(id.clone(), extras_body.unwrap_or(&h.text).to_string());
        rows.push(DecisionRow {
            id,
            repo: h.repo.clone(),
            title: title_clean,
            status,
            author: None,
            source_analysis: None,
            rationale,
            tags: Vec::new(),
            cites: Vec::new(),
            supersedes,
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

/// Query params for `/v1/dashboard/decisions`. `repo` filters to a
/// single project (Cortex / Nexus / Vectorizer / …) so the GUI can
/// render a per-repo decisions tab. `repos` (repeatable) supports
/// multi-select. Empty → all repos.
#[derive(Debug, Default, Deserialize)]
pub struct DecisionsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Multi-repo filter — `?repos=Cortex&repos=Nexus`.
    #[serde(default)]
    pub repos: Vec<String>,
}

async fn decisions(
    State(state): State<DashboardState>,
    Query(params): Query<DecisionsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let mut rows = build_decision_rows(&hits);
    // Apply repo filter: union of single + multi forms. Empty union
    // means "all repos".
    let mut allow: std::collections::HashSet<String> = params.repos.into_iter().collect();
    if let Some(r) = params.repo.filter(|s| !s.is_empty()) {
        allow.insert(r);
    }
    if !allow.is_empty() {
        rows.retain(|r| {
            r.repo
                .as_deref()
                .map(|repo| allow.contains(repo))
                .unwrap_or(false)
        });
    }
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
        .map(|h| {
            // Prefer the parsed body the meili_loader stamped onto
            // extras over the lane's `text` field, since `text` may
            // be the JSON-encoded raw envelope for archive-fed
            // decisions. The meili-fed path always has the parsed
            // form available.
            h.extras
                .get("body_markdown")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| h.text.clone())
        })
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

async fn laws(State(state): State<DashboardState>) -> Response {
    // Spec-13 owns the canonical law catalogue. Until it lands, the
    // `.claude/rules/*.md` files that the bootstrap walker imports as
    // `kind=law_violation` envelopes (one per rule file) double as
    // the catalogue: each rule's `law_id` is its identifier, and the
    // body Markdown is its definition. Dedupe the envelope stream by
    // `law_id` and return one row per unique rule.
    //
    // When spec-13 ships an explicit catalogue source (envelope kind
    // or static manifest), this handler stays — it just consumes the
    // richer stream instead of fanning out from the violations
    // bucket.
    let hits = collect_lane_hits(&state.lane);
    let mut by_id: std::collections::BTreeMap<String, LawRow> =
        std::collections::BTreeMap::new();
    for h in hits.iter().filter(|h| h.symbol.as_deref() == Some("law_violation")) {
        let law_id = match h.extras.get("law_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let title = h
            .extras
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&law_id)
            .to_string();
        let severity = h
            .severity
            .clone()
            .unwrap_or_else(|| "info".to_string());
        let blocked = severity == "critical";
        let scope = h.path.clone().unwrap_or_else(|| "all".to_string());
        let row = by_id
            .entry(law_id.clone())
            .or_insert_with(|| LawRow {
                id: law_id,
                title,
                severity,
                blocked,
                scope,
                applies: 0,
                violations_7d: 0,
                rate: 0.0,
                detector: String::new(),
                remediation: String::new(),
            });
        row.violations_7d = row.violations_7d.saturating_add(1);
        row.applies = row.applies.saturating_add(1);
    }
    let rows: Vec<LawRow> = by_id.into_values().collect();
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
        .map(|h| {
            let id = h
                .doc_id
                .strip_prefix("archive|")
                .unwrap_or(&h.doc_id)
                .to_string();
            // The meili_loader stamps `law_id` on extras when the
            // envelope body carries it. Fall back to None for
            // envelopes that arrived through the (currently empty)
            // archive path.
            let law_id = h
                .extras
                .get("law_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Critical-severity violations are surfaced as `blocked`;
            // everything else stays `annotated`. Honest mapping until
            // spec-14 emits a richer `action` enum on the wire.
            let action = if h.severity.as_deref() == Some("critical") {
                "blocked".to_string()
            } else {
                "annotated".to_string()
            };
            ViolationRow {
                id,
                law_id,
                at: ts_to_relative(h.ts),
                repo: h.repo.clone(),
                action,
                evidence: clip(&h.text, 240),
                remediation: None,
            }
        })
        .collect();
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/analyses
// ---------------------------------------------------------------------

/// One analysis row — sourced from `kind=analysis` envelopes via two
/// upstream paths:
///
/// - **phase4e bootstrap-imported audits** (`docs/analysis/**/*.md`):
///   carry `{ title, status, body, source_path }`. `repo` is the owning
///   project; `source_path` deep-links the on-disk markdown. Spec-15
///   fields (panel, judge, rounds, duration) are absent and surface as
///   empty defaults.
/// - **spec-15 deep-analysis envelopes** (when the workflow ships):
///   carry the full debate metadata. Both shapes flow through this row.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisRow {
    /// Analysis id (event id for imports; `analysis_id` for spec-15).
    pub id: String,
    /// Free-text title — H1 of the imported markdown, or `question`
    /// for spec-15.
    pub title: String,
    /// `draft` | `running` | `concluded` | `cancelled`. Imports default
    /// to whatever the markdown's `Status:` line says (or `draft`).
    pub status: String,
    /// Panelist identifiers (model / agent names). Empty for imports.
    pub panel: Vec<String>,
    /// Identifier of the judging model / human. Empty for imports.
    pub judge: String,
    /// Number of debate rounds. Zero for imports.
    pub rounds: u32,
    /// Total wall-clock duration in seconds. Zero for imports.
    pub duration_s: u32,
    /// Final verdict body (Markdown).
    pub verdict: String,
    /// Decision id this analysis was promoted to (when applicable).
    pub decision_id: Option<String>,
    /// Relative time label.
    pub occurred_at: String,
    /// Owning repo — read off the envelope's `context_repo` so the GUI
    /// can group / filter analyses per project. `None` when the analysis
    /// is not anchored to a repo (rare; spec-15 cross-repo debates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Repo-rooted source path for imported audits — lets the GUI deep-
    /// link to the underlying markdown. Absent for spec-15 envelopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

async fn analyses(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);
    let rows: Vec<AnalysisRow> = hits
        .into_iter()
        .filter(|h| h.symbol.as_deref() == Some("analysis"))
        .map(|h| {
            // Prefer the parsed extras the meili_loader stamped over
            // best-effort string-munging on `h.text`. The first non-
            // empty body line is the fallback when extras are absent
            // (in-memory archive lane path).
            let title = h
                .extras
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| clip(h.text.lines().next().unwrap_or(""), 120));
            // Older bootstrap runs (pre-`derive_status` fix) wrote the
            // whole markdown sentence into `status`. Sanitize to the
            // first ASCII word here so consumers always see a clean
            // badge token regardless of what's in the index.
            let status = h
                .extras
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| {
                    let token: String = s
                        .chars()
                        .skip_while(|c| !c.is_ascii_alphabetic())
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    token.to_ascii_lowercase()
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "draft".to_string());
            let source_path = h
                .extras
                .get("source_path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| h.path.clone());
            AnalysisRow {
                id: h
                    .doc_id
                    .strip_prefix("archive|")
                    .unwrap_or(&h.doc_id)
                    .to_string(),
                title,
                status,
                panel: Vec::new(),
                judge: String::new(),
                rounds: 0,
                duration_s: 0,
                verdict: clip(&h.text, 800),
                decision_id: None,
                occurred_at: ts_to_relative(h.ts),
                repo: h.repo.clone(),
                source_path,
            }
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
    /// Model names (rows of the matrix).
    pub models: Vec<String>,
    /// Repo names (columns of the matrix).
    pub repos: Vec<String>,
    /// `scores[model][repo]` → trust score in `[0, 1]`.
    pub scores: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    /// Provenance hint for the renderer's empty-state copy. Today
    /// always `"stub_until_spec14"`; flips to `"derived"` once spec
    /// 14 starts computing rolling violation rates per
    /// `(model, repo)`.
    pub source: &'static str,
}

async fn trust(State(_state): State<DashboardState>) -> Response {
    let body = TrustMatrix {
        models: Vec::new(),
        repos: Vec::new(),
        scores: std::collections::BTreeMap::new(),
        source: "stub_until_spec14",
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

// ---------------------------------------------------------------------
// /v1/dashboard/conversations + /v1/dashboard/conversations/{session_id}
// ---------------------------------------------------------------------

/// One conversation summary — same per-session aggregation `sessions`
/// returns, but rendered with the conversation lens (turn count
/// front-and-centre, repo + title for a chat-history list). The
/// detail endpoint below returns the full transcript.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    /// Canonical 26-char ULID.
    pub session_id: String,
    /// First captured user prompt clipped at 80 chars (the conversation's
    /// "subject line"). Empty when no turn was captured.
    pub title: String,
    /// Repos the session touched (usually one).
    pub repos: Vec<String>,
    /// Number of distinct turns we paired (each turn = one user prompt
    /// + zero-or-one assistant reply).
    pub turn_count: u64,
    /// `ts` (ms epoch) of the earliest turn we have. 0 when missing.
    pub started_at_ms: i64,
    /// `ts` (ms epoch) of the latest turn we have.
    pub last_at_ms: i64,
}

/// One paired turn in a conversation transcript. The Stop hook +
/// UserPromptSubmit hook each emit a `Kind::Turn` envelope sharing
/// the same `turn_id` under `context.extras.claude_code` — the
/// detail handler folds them into this row.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationTurn {
    /// Adapter-side turn id (`cc-turn-<ulid>`) when present, else
    /// the user envelope's `event_id`.
    pub turn_id: String,
    /// The user's prompt — sourced from the UserPromptSubmit
    /// envelope. Empty when we never captured the prompt side.
    pub user_message: String,
    /// The assistant's reply — sourced from the Stop envelope's
    /// `assistant_message`. `None` when the reply hasn't been
    /// captured yet (turn still open) or pre-Stop-hook archives.
    pub assistant_message: Option<String>,
    /// `ts` (ms epoch) — wall-clock of the user prompt envelope.
    pub started_at_ms: i64,
    /// `ts` (ms epoch) of the assistant-reply envelope when present;
    /// `None` for unpaired turns.
    pub completed_at_ms: Option<i64>,
}

/// Full transcript of one session.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationDetail {
    /// Echo the session id so the GUI can correlate without
    /// re-deriving it from the route.
    pub session_id: String,
    /// Repos touched by the session (usually one).
    pub repos: Vec<String>,
    /// Turns ordered oldest → newest.
    pub turns: Vec<ConversationTurn>,
}

/// Pull the `claude_code.turn_id` extras a hit was stamped with by
/// the adapter. Used to pair UserPromptSubmit and Stop envelopes
/// for the same turn.
fn turn_id_of(hit: &crate::lanes::LaneHit) -> Option<String> {
    hit.extras
        .get("claude_code")
        .and_then(|v| v.get("turn_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// `true` when the turn LaneHit is an internal Cortex CLI invocation
/// (classifier-worker per-event Haiku call, dashboard analyzer Sonnet
/// session-summary call) rather than a real user chat. Both render
/// their full prompt template into the spawned `claude -p`'s stdin,
/// which the adapter then captures verbatim into `user_message` —
/// flooding the Conversations panel with one row per classified
/// event. The signature checks for the stable opening sentence of
/// each prompt template; using `contains` (not `starts_with`) keeps
/// the match robust against any leading whitespace, redaction
/// markers, or shell-injected preamble.
fn is_internal_cortex_turn(hit: &crate::lanes::LaneHit) -> bool {
    let user_message = hit
        .extras
        .get("user_message")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let probe = if user_message.is_empty() {
        hit.text.as_str()
    } else {
        user_message
    };
    // Classifier-worker prompt — see
    // `crates/cortex-classifier/prompts/classifier.v1.txt`. Every
    // per-event Haiku CLI call ships this template verbatim.
    if probe.contains("You are an event classifier + graph extractor for the Cortex system") {
        return true;
    }
    // Analyzer prompt — see
    // `crates/cortex-api/src/analyzer.rs::build_prompt`. The
    // "Analyze with Sonnet" button calls this on demand.
    if probe.contains("You are analyzing one session of captured Claude Code activity") {
        return true;
    }
    // Defence in depth: when only the assistant side survived the
    // capture (e.g. user_message stripped by redaction), the
    // classifier output is still a recognisable JSON shape — a
    // markdown-fenced object whose top key is "events" and whose
    // first record carries the classifier-specific
    // `kind_refinement` field. Real Claude Code chats never reply
    // with this shape.
    let assistant_message = hit
        .extras
        .get("assistant_message")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if assistant_message.contains("\"events\":[{")
        && assistant_message.contains("\"kind_refinement\"")
    {
        return true;
    }
    false
}

async fn conversations_list(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Group turn-kind hits by session. `is_internal_cortex_turn`
    // drops classifier-worker / analyzer CLI invocations so the
    // panel shows only real user chats — those tooling calls were
    // creating one session row per classified event.
    let mut by_session: std::collections::BTreeMap<String, Vec<crate::lanes::LaneHit>> =
        std::collections::BTreeMap::new();
    for h in hits.into_iter().filter(|h| {
        symbol_to_kind(h.symbol.as_deref()) == "turn" && !is_internal_cortex_turn(h)
    }) {
        if let Some(sid) = session_id_of(&h) {
            by_session.entry(sid.to_string()).or_default().push(h);
        }
    }

    let mut rows: Vec<ConversationSummary> = by_session
        .into_iter()
        .map(|(session_id, mut bucket)| {
            bucket.sort_by(|a, b| a.ts.cmp(&b.ts));
            // Distinct turn_ids — pairs (user envelope + Stop envelope)
            // sharing the same turn_id collapse to one count. Hits
            // without a turn_id (legacy archives pre-Stop-hook) each
            // count as their own turn so we never under-report.
            let mut seen_turns: std::collections::BTreeSet<String> = Default::default();
            let mut anonymous = 0u64;
            for h in &bucket {
                match turn_id_of(h) {
                    Some(tid) => {
                        seen_turns.insert(tid);
                    }
                    None => anonymous += 1,
                }
            }
            let turn_count = seen_turns.len() as u64 + anonymous;

            let mut repos: std::collections::BTreeSet<String> = Default::default();
            let mut title = String::new();
            for h in &bucket {
                if let Some(r) = h.repo.as_deref() {
                    repos.insert(r.to_string());
                }
                // First non-empty user_message wins. The Stop envelope
                // has user_message="" so we never accidentally surface
                // the assistant's reply as the conversation title.
                if title.is_empty() && !h.text.is_empty() {
                    title = clip(h.text.lines().next().unwrap_or(""), 80);
                }
            }

            ConversationSummary {
                session_id,
                title,
                repos: repos.into_iter().collect(),
                turn_count,
                started_at_ms: bucket.first().map(|h| h.ts).unwrap_or(0),
                last_at_ms: bucket.last().map(|h| h.ts).unwrap_or(0),
            }
        })
        .collect();

    rows.sort_by(|a, b| b.last_at_ms.cmp(&a.last_at_ms));
    (StatusCode::OK, Json(rows)).into_response()
}

async fn conversation_detail(
    State(state): State<DashboardState>,
    Path(session_id): Path<String>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Pair UserPromptSubmit + Stop envelopes by turn_id within this
    // session. The user envelope carries user_message + ts of the
    // prompt; the Stop envelope carries assistant_message + ts of
    // the reply. When only one half exists (still in flight, or a
    // pre-Stop-hook archive), we surface what we have.
    struct TurnSlot {
        turn_id: String,
        user_message: String,
        assistant_message: Option<String>,
        started_at_ms: i64,
        completed_at_ms: Option<i64>,
    }
    let mut slots: std::collections::BTreeMap<String, TurnSlot> = std::collections::BTreeMap::new();
    let mut anonymous: Vec<TurnSlot> = Vec::new();
    let mut repos: std::collections::BTreeSet<String> = Default::default();

    for h in hits.into_iter().filter(|h| {
        session_id_of(h) == Some(session_id.as_str())
            && symbol_to_kind(h.symbol.as_deref()) == "turn"
            && !is_internal_cortex_turn(h)
    }) {
        if let Some(r) = h.repo.as_deref() {
            repos.insert(r.to_string());
        }
        // Disambiguate user-side vs Stop-side by checking which field
        // has content. The cortex-fulltext builder concatenates
        // user_message + "\n" + assistant_message into the LaneHit's
        // `text`; Stop envelopes start with the assistant text
        // (user_message empty), UserPromptSubmit envelopes start
        // with the user prompt.
        //
        // The body extras the meili_loader stamps carry the parsed
        // payload directly — when present they're the authoritative
        // signal. Fall back to the text-shape heuristic for
        // archive_loader hits which don't have the parsed extras.
        let user_text = h
            .extras
            .get("user_message")
            .and_then(|v| v.as_str())
            .map(String::from);
        let assistant_text = h
            .extras
            .get("assistant_message")
            .and_then(|v| v.as_str())
            .map(String::from);
        let (user_msg, assistant_msg) = match (user_text, assistant_text) {
            (Some(u), Some(a)) => (u, Some(a)),
            (Some(u), None) => (u, None),
            (None, Some(a)) => (String::new(), Some(a)),
            (None, None) => (h.text.clone(), None),
        };

        match turn_id_of(&h) {
            Some(tid) => {
                let slot = slots.entry(tid.clone()).or_insert_with(|| TurnSlot {
                    turn_id: tid.clone(),
                    user_message: String::new(),
                    assistant_message: None,
                    started_at_ms: 0,
                    completed_at_ms: None,
                });
                if !user_msg.is_empty() && slot.user_message.is_empty() {
                    slot.user_message = user_msg;
                    slot.started_at_ms = h.ts;
                }
                if let Some(a) = assistant_msg {
                    if slot.assistant_message.is_none() {
                        slot.assistant_message = Some(a);
                        slot.completed_at_ms = Some(h.ts);
                    }
                }
            }
            None => {
                anonymous.push(TurnSlot {
                    turn_id: format!("anon-{}", h.ts),
                    user_message: user_msg,
                    assistant_message: assistant_msg,
                    started_at_ms: h.ts,
                    completed_at_ms: None,
                });
            }
        }
    }

    let mut turns: Vec<ConversationTurn> = slots
        .into_values()
        .chain(anonymous)
        .map(|s| ConversationTurn {
            turn_id: s.turn_id,
            user_message: s.user_message,
            assistant_message: s.assistant_message,
            started_at_ms: s.started_at_ms,
            completed_at_ms: s.completed_at_ms,
        })
        .collect();
    turns.sort_by(|a, b| a.started_at_ms.cmp(&b.started_at_ms));

    let detail = ConversationDetail {
        session_id,
        repos: repos.into_iter().collect(),
        turns,
    };
    (StatusCode::OK, Json(detail)).into_response()
}

/// Sonnet-backed session summary. Pulls every event from the named
/// session, hands them to the analyzer (which shells out to the
/// local `claude` CLI with `--model claude-sonnet-4-6`), and
/// returns a structured summary + key actions + cross-references.
/// Cached server-side keyed by `(session_id, last_event_ts)` so a
/// dashboard refresh doesn't re-burn the call.
async fn conversation_summary(
    State(state): State<DashboardState>,
    Path(session_id): Path<String>,
) -> Response {
    match state.analyzer.summarize_session(&state.lane, &session_id).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(reason) => {
            // 503 with a structured body — the GUI shows a graceful
            // "summary unavailable" instead of treating this as a
            // hard outage. Most likely cause is `claude` not on
            // PATH or the model returning malformed JSON; the
            // reason field tells the user which.
            let body = serde_json::json!({
                "error": "summary_unavailable",
                "reason": reason,
                "session_id": session_id,
            });
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
    }
}

// ---------------------------------------------------------------------
// /v1/dashboard/handoffs
// ---------------------------------------------------------------------

/// One hand-off snapshot — pulled from `kind=memory` envelopes whose
/// path lives under `.rulebook/handoff/`. The walker now promotes
/// those automatically across every repo.
#[derive(Debug, Clone, Serialize)]
pub struct HandoffRow {
    /// Repo this hand-off belongs to.
    pub repo: Option<String>,
    /// Repo-relative path of the hand-off file.
    pub path: Option<String>,
    /// Filename component for display (e.g. `_pending.md` /
    /// `2026-04-27.md`).
    pub filename: String,
    /// Excerpt of the body clipped at ~600 chars.
    pub excerpt: String,
    /// Relative wall-clock label (`"3 hours ago"` / `"yesterday"`)
    /// — empty when no timestamp was preserved.
    pub updated: String,
    /// Raw ms epoch for client-side sorting.
    pub updated_ms: i64,
}

/// Query params for `/v1/dashboard/handoffs`.
#[derive(Debug, Default, Deserialize)]
pub struct HandoffsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
}

async fn handoffs(
    State(state): State<DashboardState>,
    Query(params): Query<HandoffsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);

    let mut rows: Vec<HandoffRow> = hits
        .into_iter()
        .filter(|h| {
            symbol_to_kind(h.symbol.as_deref()) == "memory"
                && h.path
                    .as_deref()
                    .map(|p| p.contains(".rulebook/handoff/") || p.contains(".rulebook\\handoff\\"))
                    .unwrap_or(false)
        })
        .filter(|h| {
            // Optional repo filter.
            match params.repo.as_deref().filter(|s| !s.is_empty()) {
                Some(r) => h.repo.as_deref() == Some(r),
                None => true,
            }
        })
        .map(|h| {
            let filename = h
                .path
                .as_deref()
                .and_then(|p| p.rsplit(['/', '\\']).next())
                .unwrap_or("(unnamed)")
                .to_string();
            HandoffRow {
                repo: h.repo.clone(),
                path: h.path.clone(),
                filename,
                excerpt: clip(&h.text, 600),
                updated: ts_to_relative(h.ts),
                updated_ms: h.ts,
            }
        })
        .collect();

    // Most-recent first — the user is usually looking for the latest
    // hand-off when resuming a session.
    rows.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
    (StatusCode::OK, Json(rows)).into_response()
}

// ---------------------------------------------------------------------
// /v1/dashboard/classifications
// ---------------------------------------------------------------------

/// One classified-event row surfaced by the Classifications view.
/// Mirrors what the cortex-fulltext-worker stamped on the Meili doc
/// (which the meili_loader projects onto every LaneHit's extras),
/// shaped for the GUI table.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationRow {
    /// `event_id` (best-effort — falls back to the `doc_id` chunks).
    pub event_id: String,
    /// `turn` / `tool_call` / `decision` / `memory` / etc.
    pub kind: String,
    /// Repo the event was captured from.
    pub repo: Option<String>,
    /// Repo-relative path when available (artifact / handoff / spec).
    pub path: Option<String>,
    /// Topics the classifier stamped (controlled-vocab tags).
    pub topics: Vec<String>,
    /// `info` / `notable` / `critical`.
    pub severity: Option<String>,
    /// `none` / `low` / `high` (or whatever the classifier surfaced).
    pub pii_risk: Option<String>,
    /// Short summary clipped at 240 chars — same content the
    /// Sonnet classifier produces, surfaced inline so the operator
    /// can see whether the summaries are useful at scale.
    pub summary: String,
    /// Wall-clock ms epoch.
    pub ts: i64,
    /// Relative time label.
    pub at: String,
}

/// Aggregate counts the GUI renders as histograms / topic clouds
/// alongside the recent rows.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationStats {
    /// Total classified events surfaced (post-filter).
    pub total: u64,
    /// Top topics across the surfaced rows, descending by count.
    pub top_topics: Vec<TopicCount>,
    /// Per-severity counts.
    pub by_severity: Vec<KindCount>,
    /// Per-pii-risk counts.
    pub by_pii_risk: Vec<KindCount>,
    /// Per-repo counts.
    pub by_repo: Vec<RepoCount>,
}

/// One row of the topic cloud.
#[derive(Debug, Clone, Serialize)]
pub struct TopicCount {
    /// The topic tag.
    pub topic: String,
    /// How many surfaced rows carried it.
    pub count: u64,
}

/// Top-level body for `/v1/dashboard/classifications`. Splits the
/// stats from the rows so the GUI can lay them out in distinct
/// regions without re-aggregating client-side.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationsBody {
    /// Aggregate counts over the surfaced rows.
    pub stats: ClassificationStats,
    /// Recent rows, newest-first, capped by `limit`.
    pub rows: Vec<ClassificationRow>,
}

/// Query params for `/v1/dashboard/classifications`. All optional;
/// empty filters surface every classified event in the lane.
#[derive(Debug, Default, Deserialize)]
pub struct ClassificationsQuery {
    /// Single-repo filter — `?repo=Nexus`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Single-topic filter — `?topic=performance`.
    #[serde(default)]
    pub topic: Option<String>,
    /// Single-severity filter — `?severity=critical`.
    #[serde(default)]
    pub severity: Option<String>,
    /// Single-kind filter — `?kind=turn`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Cap on rows returned. Stats always cover the full filtered
    /// set regardless of this limit.
    #[serde(default)]
    pub limit: Option<usize>,
}

async fn classifications(
    State(state): State<DashboardState>,
    Query(params): Query<ClassificationsQuery>,
) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Apply optional filters first so stats reflect what the user
    // is looking at, not the whole corpus.
    let filtered: Vec<&crate::lanes::LaneHit> = hits
        .iter()
        .filter(|h| {
            if let Some(r) = params.repo.as_deref().filter(|s| !s.is_empty()) {
                if h.repo.as_deref() != Some(r) {
                    return false;
                }
            }
            if let Some(k) = params.kind.as_deref().filter(|s| !s.is_empty()) {
                if symbol_to_kind(h.symbol.as_deref()) != k {
                    return false;
                }
            }
            if let Some(sev) = params.severity.as_deref().filter(|s| !s.is_empty()) {
                if h.severity.as_deref() != Some(sev) {
                    return false;
                }
            }
            if let Some(t) = params.topic.as_deref().filter(|s| !s.is_empty()) {
                let topics = h
                    .extras
                    .get("topics")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .any(|x| x.as_str() == Some(t))
                    })
                    .unwrap_or(false);
                if !topics {
                    return false;
                }
            }
            true
        })
        .collect();

    // Aggregates over the filtered set.
    let mut topic_counts: std::collections::HashMap<String, u64> = Default::default();
    let mut sev_counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut pii_counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut repo_counts: std::collections::BTreeMap<String, u64> = Default::default();
    for h in &filtered {
        if let Some(arr) = h.extras.get("topics").and_then(|v| v.as_array()) {
            for t in arr.iter().filter_map(|v| v.as_str()) {
                *topic_counts.entry(t.to_string()).or_insert(0) += 1;
            }
        }
        if let Some(s) = h.severity.as_deref() {
            *sev_counts.entry(s.to_string()).or_insert(0) += 1;
        }
        if let Some(p) = h
            .extras
            .get("pii_risk")
            .and_then(|v| v.as_str())
        {
            *pii_counts.entry(p.to_string()).or_insert(0) += 1;
        }
        if let Some(r) = h.repo.as_deref() {
            *repo_counts.entry(r.to_string()).or_insert(0) += 1;
        }
    }

    let mut top_topics: Vec<TopicCount> = topic_counts
        .into_iter()
        .map(|(topic, count)| TopicCount { topic, count })
        .collect();
    top_topics.sort_by(|a, b| b.count.cmp(&a.count));
    top_topics.truncate(40);

    let by_severity: Vec<KindCount> = sev_counts
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let by_pii_risk: Vec<KindCount> = pii_counts
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let mut by_repo: Vec<RepoCount> = repo_counts
        .into_iter()
        .map(|(repo, count)| RepoCount { repo, count })
        .collect();
    by_repo.sort_by(|a, b| b.count.cmp(&a.count));

    let total = filtered.len() as u64;

    // Recent rows, newest-first, capped at `limit` (default 100, max 500).
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let mut sorted: Vec<&crate::lanes::LaneHit> = filtered.into_iter().collect();
    sorted.sort_by(|a, b| b.ts.cmp(&a.ts));
    let rows: Vec<ClassificationRow> = sorted
        .into_iter()
        .take(limit)
        .map(|h| {
            let topics = h
                .extras
                .get("topics")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let summary = h
                .extras
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| clip(s, 240))
                .unwrap_or_else(|| clip(&h.text, 240));
            let pii_risk = h
                .extras
                .get("pii_risk")
                .and_then(|v| v.as_str())
                .map(String::from);
            let event_id = h
                .doc_id
                .rsplit_once('|')
                .map(|(_, id)| id.to_string())
                .unwrap_or_else(|| h.doc_id.clone());
            ClassificationRow {
                event_id,
                kind: symbol_to_kind(h.symbol.as_deref()).to_string(),
                repo: h.repo.clone(),
                path: h.path.clone(),
                topics,
                severity: h.severity.clone(),
                pii_risk,
                summary,
                ts: h.ts,
                at: ts_to_relative(h.ts),
            }
        })
        .collect();

    let body = ClassificationsBody {
        stats: ClassificationStats {
            total,
            top_topics,
            by_severity,
            by_pii_risk,
            by_repo,
        },
        rows,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Query params for `/v1/dashboard/graph`.
#[derive(Debug, Default, Deserialize)]
pub struct GraphQuery {
    /// Restrict to one session — when set, the Cypher MATCH anchors
    /// at `Session {session_id: $sid}`. When unset, the handler
    /// returns the most-recently-active subgraph capped at `limit`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Restrict to one or more repos. Each `repo=<name>` query param
    /// is appended; the filter passes when the artifact's owning
    /// Repo matches ANY of the listed repos. Other kinds (Session /
    /// Decision / Memory / Law / Analysis) are kept regardless so
    /// the cross-project knowledge spine stays visible even when
    /// the user is drilling into a single repo's artifacts.
    #[serde(default)]
    pub repo: Vec<String>,
    /// Cap the total node count. Defaults to 200, max 50,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

async fn graph(
    State(state): State<DashboardState>,
    Query(params): Query<GraphQuery>,
) -> Response {
    // The cap is intentionally generous: the explorer's "show me the
    // whole panorama" mode needs to walk every Repo / Session / Turn /
    // ToolCall / Artifact at once. Sigma WebGL renders 30k+ nodes at
    // 60fps; the network round-trip at this size is ~200 KB gzipped.
    // The `unwrap_or` default stays small so unauthenticated probes
    // don't accidentally pull the full graph.
    let limit = params.limit.unwrap_or(200).clamp(1, 50_000);
    let repo_filter: Vec<String> = params
        .repo
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();

    // Live path: when a Nexus client is configured, run a real
    // Cypher MATCH and convert the returned rows into the GraphPayload
    // shape the GUI consumes. On any failure (transport, schema, empty
    // result), fall through to the synthetic-from-lane fallback so a
    // dev environment without a populated Nexus still renders
    // something useful.
    if let Some(nx) = state.nexus.as_ref() {
        match query_nexus_graph(
            nx.as_ref(),
            params.session_id.as_deref(),
            &repo_filter,
            limit,
        )
        .await
        {
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
    repo_filter: &[String],
    limit: usize,
) -> anyhow::Result<GraphPayload> {
    let mut nodes_by_id: std::collections::HashMap<String, GraphNode> =
        std::collections::HashMap::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_edges: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Build a HashSet for O(1) membership; the loop below short-
    // circuits the whole-Cypher round-trip when the filter rejects.
    let repo_set: std::collections::HashSet<&str> = repo_filter
        .iter()
        .map(String::as_str)
        .collect();

    // Edge-first sampling. The previous node-first strategy pulled
    // each label independently (10 Sessions, 18 Turns, 18 Memories…)
    // and then filtered edges to those whose endpoints happened to
    // overlap. With independent random samples the overlap was
    // near-zero — a fresh DB returned 98 nodes and 23 edges, mostly
    // because Sessions and their Turns weren't co-sampled.
    //
    // The fix: each MATCH returns BOTH endpoints' id + display label
    // in one round-trip, and we materialize the nodes inline from
    // the edge rows. Every edge we keep is guaranteed to have its
    // endpoints rendered.
    //
    // Tuple: (from_label, from_id_prop, from_label_prop, from_kind,
    //         rel, to_label, to_id_prop, to_label_prop, to_kind)
    // `*_label_prop` resolves the human-readable display name for
    // each endpoint. The mapper writes `name` on every node kind
    // (Turn = user message, Decision = title, Session/Memory = best-
    // effort short label). Defaulting to `id` made the canvas a wall
    // of ULIDs — `name` lets the explorer show "fixes" / "docs:
    // move..." instead of "01KQ8534N93Y1MA28...".
    let edge_specs: &[(&str, &str, &str, &str, &str, &str, &str, &str, &str)] = &[
        ("Session", "id", "name", "session", "HAS_TURN", "Turn", "id", "name", "turn"),
        ("Session", "id", "name", "session", "REMEMBERS", "Memory", "id", "name", "memory"),
        ("Turn", "id", "name", "turn", "HAS_TOOL_CALL", "ToolCall", "id", "name", "tool_call"),
        // Session→ToolCall is the mapper's fallback anchor for tool_call
        // events without a `parent_event_id` (bootstrap envelopes ship
        // without one — see `cortex-graph/src/mapper.rs::emit_tool_call`).
        // Without this row the dashboard misses every backfilled
        // HAS_TOOL_CALL because the canonical `(:Turn)-[:HAS_TOOL_CALL]->`
        // pattern matches zero on bootstrap data.
        ("Session", "id", "name", "session", "HAS_TOOL_CALL", "ToolCall", "id", "name", "tool_call"),
        ("Turn", "id", "name", "turn", "HAS_AGENT_CALL", "AgentCall", "id", "name", "agent_call"),
        ("Session", "id", "name", "session", "HAS_AGENT_CALL", "AgentCall", "id", "name", "agent_call"),
        ("ToolCall", "id", "name", "tool_call", "TOUCHED", "Artifact", "natural_key", "name", "artifact"),
        ("Artifact", "natural_key", "name", "artifact", "IN_REPO", "Repo", "name", "name", "repo"),
        ("LawViolation", "id", "name", "violation", "OBSERVED_IN", "Turn", "id", "name", "turn"),
        ("LawViolation", "id", "name", "violation", "OF", "Law", "id", "name", "law"),
        ("Decision", "id", "name", "decision", "SUPERSEDES", "Decision", "id", "name", "decision"),
    ];

    // Per-relation budget. Each MATCH gets the full node-count cap so
    // the rarest relationships land first and the densest one
    // (IN_REPO, ~28k rows on Cortex) is allowed to fill the rest of
    // the budget. The early-break on `nodes_by_id.len() >= limit`
    // inside the row loop bounds total memory regardless of
    // per-relation supply. The integer is inlined into the query
    // string because the Nexus Cypher dialect ignores `LIMIT $param`.
    let per_rel_limit = limit;

    for (from_label, from_id_p, from_lbl_p, from_kind, rel, to_label, to_id_p, to_lbl_p, to_kind) in
        edge_specs
    {
        if nodes_by_id.len() >= limit {
            break;
        }
        // Session-anchored mode: restrict the edge's "from" side to
        // descendants of the requested Session. Cheaper than a full
        // 3-hop path because each rel-spec already names the depth.
        let cy = match session_id {
            Some(_) if *from_label == "Session" => format!(
                "MATCH (a:Session {{ session_id: $sid }})-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "Turn" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(a:Turn)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "ToolCall" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(:Turn)-[:HAS_TOOL_CALL]->(a:ToolCall)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl \
                 LIMIT {per_rel_limit}"
            ),
            Some(_) if *from_label == "LawViolation" => format!(
                "MATCH (s:Session {{ session_id: $sid }})-[:HAS_TURN]->(t:Turn)<-[:OBSERVED_IN]-(a:LawViolation)-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl \
                 LIMIT {per_rel_limit}"
            ),
            // Decision/SUPERSEDES and Artifact/IN_REPO are not
            // session-scoped — skip when filtering by session.
            Some(_) => continue,
            None => format!(
                "MATCH (a:{from_label})-[r:{rel}]->(b:{to_label}) \
                 RETURN a.{from_id_p} AS f_id, a.{from_lbl_p} AS f_lbl, \
                        b.{to_id_p} AS t_id, b.{to_lbl_p} AS t_lbl \
                 LIMIT {per_rel_limit}"
            ),
        };

        let mut params: std::collections::HashMap<String, NexusValue> =
            std::collections::HashMap::new();
        if let Some(sid) = session_id {
            params.insert("sid".to_string(), NexusValue::String(sid.to_string()));
        }

        let res = match client.execute_cypher(&cy, Some(params)).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    rel,
                    error = %e,
                    "nexus edge pull failed; skipping this relationship"
                );
                continue;
            }
        };

        for row in &res.rows {
            let cells = match row.as_array() {
                Some(a) => a,
                None => continue,
            };
            let from_id = cell_str(cells.first()).unwrap_or_default();
            let to_id = cell_str(cells.get(2)).unwrap_or_default();
            if from_id.is_empty() || to_id.is_empty() {
                continue;
            }
            let from_lbl = cell_str(cells.get(1))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| from_id.clone());
            let to_lbl = cell_str(cells.get(3))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| to_id.clone());

            // Repo-filter slice. The filter is applied at the edge-row
            // level so the per-relationship Cypher stays a generic
            // pattern — IN_REPO edges drop unless the destination
            // Repo is in the allow-list, and TOUCHED edges drop
            // unless the artifact's `repo|path|hash` natural key
            // starts with one of the filtered repos. Other edge types
            // (HAS_TURN, REMEMBERS, OBSERVED_IN, etc.) are
            // project-agnostic and pass through so the cross-project
            // session-tree spine stays visible even in a single-repo
            // drill-down.
            if !repo_set.is_empty() {
                if *rel == "IN_REPO" {
                    if !repo_set.contains(to_id.as_str()) {
                        continue;
                    }
                } else if *rel == "TOUCHED" {
                    let artifact_repo = to_id.split('|').next().unwrap_or("");
                    if !repo_set.contains(artifact_repo) {
                        continue;
                    }
                }
            }

            nodes_by_id
                .entry(from_id.clone())
                .or_insert_with(|| GraphNode {
                    id: from_id.clone(),
                    label: clip(&from_lbl, 64),
                    x: 0,
                    y: 0,
                    kind: (*from_kind).to_string(),
                });
            nodes_by_id
                .entry(to_id.clone())
                .or_insert_with(|| GraphNode {
                    id: to_id.clone(),
                    label: clip(&to_lbl, 64),
                    x: 0,
                    y: 0,
                    kind: (*to_kind).to_string(),
                });

            let key = format!("{from_id}|{rel}|{to_id}");
            if seen_edges.insert(key) {
                edges.push(GraphEdge {
                    from: from_id,
                    to: to_id,
                    label: (*rel).to_string(),
                });
            }
            if nodes_by_id.len() >= limit {
                break;
            }
        }
    }

    // Top-up pass — surface isolated nodes (Decisions without a
    // SUPERSEDES, Laws without a violation, etc.) so the graph
    // doesn't show 100% session-tree and hide the rest of the
    // domain. Skipped when session-anchored: in that mode we want
    // a focused subgraph, not a domain dump.
    if session_id.is_none() && nodes_by_id.len() < limit {
        // Same `name`-as-label convention as the edge_specs above —
        // the mapper writes a human-readable `name` on every kind.
        let label_specs: &[(&str, &str, &str, &str)] = &[
            ("Decision", "id", "name", "decision"),
            ("Law", "id", "name", "law"),
            ("Analysis", "id", "name", "analysis"),
            ("Repo", "name", "name", "repo"),
            ("Memory", "id", "name", "memory"),
        ];
        let remaining = limit - nodes_by_id.len();
        let per_label = (remaining / label_specs.len()).max(3);

        for (cypher_label, id_prop, label_prop, gui_kind) in label_specs {
            if nodes_by_id.len() >= limit {
                break;
            }
            let cy = format!(
                "MATCH (n:{cypher_label}) \
                 RETURN n.{id_prop} AS id, n.{label_prop} AS label \
                 LIMIT {per_label}"
            );
            let res = match client
                .execute_cypher(
                    &cy,
                    Some(std::collections::HashMap::<String, NexusValue>::new()),
                )
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in &res.rows {
                let cells = match row.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                let id = cell_str(cells.first()).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                // Skip Repo nodes outside the repo filter (when set).
                // Other kinds pass through — Decisions / Laws /
                // Memories cross repo boundaries and dropping them
                // would gut the cross-project knowledge spine.
                if !repo_set.is_empty()
                    && *gui_kind == "repo"
                    && !repo_set.contains(id.as_str())
                {
                    continue;
                }
                let label = cell_str(cells.get(1))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| id.clone());
                nodes_by_id
                    .entry(id.clone())
                    .or_insert_with(|| GraphNode {
                        id,
                        label: clip(&label, 64),
                        x: 0,
                        y: 0,
                        kind: (*gui_kind).to_string(),
                    });
                if nodes_by_id.len() >= limit {
                    break;
                }
            }
        }
    }

    let nodes: Vec<GraphNode> = nodes_by_id.into_values().collect();
    Ok(GraphPayload { nodes, edges })
}

/// Pull a row cell as `String`, accepting either a JSON string or a
/// non-null scalar (number / bool) that we stringify. Returns `None`
/// for null / missing cells so the caller can substitute a fallback.
fn cell_str(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    Some(v.to_string())
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

// ---------------------------------------------------------------------
// /v1/dashboard/tasks*
// ---------------------------------------------------------------------

/// Query params for the list endpoint. `axum-extra::Query` handles the
/// repeated multi-value params (`status=...&status=...`) directly.
#[derive(Debug, Default, Deserialize)]
pub struct TasksListQuery {
    /// Multi-value status filter.
    #[serde(default)]
    pub status: Vec<String>,
    /// Multi-value phase filter (exact match against the canonical
    /// phase key, e.g. `phase2g`).
    #[serde(default)]
    pub phase: Vec<String>,
    /// Drop archived rows when set to `false`. Defaults to `true`.
    #[serde(default)]
    pub include_archived: Option<bool>,
    /// Page size (default 200, capped at 500 by the loader).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Page offset (default 0).
    #[serde(default)]
    pub offset: Option<usize>,
    /// `phase` (default), `updated_at`, or `created_at`.
    #[serde(default)]
    pub sort: Option<String>,
    /// `asc` or `desc`. Defaults to ascending for `phase` and
    /// descending for the timestamp fields.
    #[serde(default)]
    pub order: Option<String>,
}

fn list_query_from(params: TasksListQuery) -> ListQuery {
    let sort = match params.sort.as_deref() {
        Some("updated_at") => SortField::UpdatedAt,
        Some("created_at") => SortField::CreatedAt,
        _ => SortField::Phase,
    };
    let order = match params.order.as_deref() {
        Some("asc") => Some(SortOrder::Asc),
        Some("desc") => Some(SortOrder::Desc),
        _ => None,
    };
    ListQuery {
        status: params.status,
        phase: params.phase,
        include_archived: params.include_archived.unwrap_or(true),
        limit: params.limit.unwrap_or(200),
        offset: params.offset.unwrap_or(0),
        sort,
        order,
    }
}

/// `GET /v1/dashboard/tasks` — filtered list with phase + status
/// breakdowns. Returns `200` even when the workspace root is missing
/// (just yields an empty list + zero breakdowns) so the GUI's empty
/// state is the only thing the user sees on a misconfigured deploy.
async fn tasks_list(
    State(state): State<DashboardState>,
    Query(params): Query<TasksListQuery>,
) -> Response {
    let query = list_query_from(params);
    let body = state.tasks.list(&query);
    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /v1/dashboard/tasks/summary` — aggregate counters for the
/// sidebar pill + the Tasks-view stats grid.
async fn tasks_summary(State(state): State<DashboardState>) -> Response {
    let body = state.tasks.summary();
    (StatusCode::OK, Json(body)).into_response()
}

// ---------------------------------------------------------------------
// /v1/retention/sweeps + /v1/retention/state (phase9i)
// ---------------------------------------------------------------------

/// Query params for `/v1/retention/sweeps`.
#[derive(Debug, Default, Deserialize)]
struct RetentionSweepsQuery {
    /// Maximum rows to return. Defaults to 50, capped at 500.
    limit: Option<usize>,
    /// Optional RFC-3339 lower bound on `started_at`.
    since: Option<String>,
}

/// One row in the `/v1/retention/sweeps` response.
#[derive(Debug, Clone, Serialize)]
struct RetentionSweepBody {
    sweep_id: String,
    started_at: String,
    finished_at: Option<String>,
    status: String,
    records_demoted: u64,
    records_dropped: u64,
    /// Per-stage counters parsed from `tier_transitions_json`. Keys
    /// are stage names (`sweep`, `parquet_rollup`, `cas_vacuum`,
    /// `pii_enforce`, `turn_digest`, `meili_prune`, `metadata_reap`,
    /// …) and values carry the stage's own JSON shape so the GUI can
    /// render breakdowns without the API having to know each stage's
    /// schema. Falls back to an empty object when the source row's
    /// JSON is missing or malformed.
    stages: serde_json::Value,
}

/// `GET /v1/retention/sweeps` — recent retention sweeps + per-stage
/// breakdown. The response is the rows from `retention_sweeps`
/// (newest first) merged with the JSON inside
/// `tier_transitions_json` so the GUI can render per-stage counters.
async fn retention_sweeps(
    State(state): State<DashboardState>,
    Query(params): Query<RetentionSweepsQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).min(500);
    let since_filter = params
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_rfc3339());

    let metadata = match &state.metadata {
        Some(m) => m,
        None => {
            // Honest empty when no metadata DB is configured (cold dev
            // boot). Keeps the GUI's empty-state branch usable.
            return (StatusCode::OK, Json(Vec::<RetentionSweepBody>::new())).into_response();
        }
    };

    let rows = {
        let guard = match metadata.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match guard.list_recent_sweeps(limit) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error=%e, "retention/sweeps: list_recent_sweeps failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "list_recent_sweeps", "detail": e.to_string()})),
                )
                    .into_response();
            }
        }
    };

    let body: Vec<RetentionSweepBody> = rows
        .into_iter()
        .filter(|r| since_filter.as_deref().map_or(true, |s| r.started_at.as_str() >= s))
        .map(|r| {
            let stages = r
                .tier_transitions_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            RetentionSweepBody {
                sweep_id: r.sweep_id,
                started_at: r.started_at,
                finished_at: r.finished_at,
                status: r.status,
                records_demoted: r.records_demoted,
                records_dropped: r.records_dropped,
                stages,
            }
        })
        .collect();

    (StatusCode::OK, Json(body)).into_response()
}

/// One archive partition bucket — bytes by age window.
#[derive(Debug, Clone, Default, Serialize)]
struct ArchiveBuckets {
    /// Bytes in archive files modified in the last 30 days.
    le_30d: u64,
    /// Bytes in files modified between 30 and 365 days ago.
    #[serde(rename = "30d_to_365d")]
    between_30d_365d: u64,
    /// Bytes in files modified more than 365 days ago.
    gt_365d: u64,
    /// Total bytes scanned.
    total: u64,
    /// Archive root that produced these counters.
    root: String,
    /// `false` when the archive root could not be resolved or read;
    /// in that case the counters are zeros.
    available: bool,
}

/// CAS store totals.
#[derive(Debug, Clone, Default, Serialize)]
struct CasTotals {
    /// Number of `cas_blobs` rows.
    rows: u64,
    /// Sum of `size` (uncompressed bytes).
    bytes: u64,
    /// `false` when the CAS DB could not be opened (e.g. cold dev
    /// boot). Counters are zeros.
    available: bool,
    /// Path the totals were read from.
    path: String,
}

/// One scheduled run row. Today every row reports `"never"` because
/// the phase9k cron scheduler hasn't shipped yet — the field is here
/// so the GUI's table is wireshape-stable across the cut-over.
#[derive(Debug, Clone, Serialize)]
struct ScheduledRun {
    /// Sweep type (`tier_sweep`, `parquet_rollup`, `cas_vacuum`,
    /// `pii_enforce`, `turn_digest`, `meili_prune`,
    /// `metadata_reap`).
    sweep: String,
    /// RFC-3339 timestamp of the next scheduled run, or `"never"`.
    next_run: String,
}

/// `/v1/retention/state` body.
#[derive(Debug, Clone, Serialize)]
struct RetentionStateBody {
    /// Per-Vectorizer-collection size. Empty until a live SDK probe
    /// is wired through `DashboardState`; the GUI handles `[]`
    /// honestly with an "unavailable" pill.
    collections: Vec<serde_json::Value>,
    /// Parquet archive bytes by age bucket.
    archive_bytes: ArchiveBuckets,
    /// Meilisearch index document counts. Empty when no live probe
    /// is available; same honest-empty semantics as `collections`.
    meili_indexes: Vec<serde_json::Value>,
    /// CAS store totals (rows + bytes).
    cas: CasTotals,
    /// Per-sweep schedule. Every row is `"never"` until phase9k
    /// publishes a `cron_jobs` table.
    next_runs: Vec<ScheduledRun>,
}

/// `GET /v1/retention/state` — compact "current state" envelope the
/// Retention tab's header cards consume.
async fn retention_state(State(_state): State<DashboardState>) -> Response {
    let archive_root = std::env::var("CORTEX_ARCHIVE_ROOT").ok().unwrap_or_else(|| {
        home_path().map_or_else(
            || ".cortex/archive".to_string(),
            |h| h.join(".cortex/archive").display().to_string(),
        )
    });
    let archive_bytes = scan_archive_age_buckets(std::path::Path::new(&archive_root));

    let cas_path = std::env::var("CORTEX_CAS_DB").ok().unwrap_or_else(|| {
        home_path().map_or_else(
            || ".cortex/cas.sqlite".to_string(),
            |h| h.join(".cortex/cas.sqlite").display().to_string(),
        )
    });
    let cas = scan_cas_totals(std::path::Path::new(&cas_path));

    let next_runs: Vec<ScheduledRun> = [
        "tier_sweep",
        "parquet_rollup",
        "cas_vacuum",
        "pii_enforce",
        "turn_digest",
        "meili_prune",
        "metadata_reap",
    ]
    .iter()
    .map(|s| ScheduledRun {
        sweep: (*s).to_string(),
        next_run: "never".to_string(),
    })
    .collect();

    let body = RetentionStateBody {
        collections: Vec::new(),
        archive_bytes,
        meili_indexes: Vec::new(),
        cas,
        next_runs,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Scan `root` for Parquet / NDJSON archive files and bucket their
/// bytes by file-mtime age. Honest defaults when the root is
/// missing / unreadable.
fn scan_archive_age_buckets(root: &std::path::Path) -> ArchiveBuckets {
    let mut out = ArchiveBuckets {
        root: root.display().to_string(),
        ..ArchiveBuckets::default()
    };
    if !root.exists() {
        return out;
    }
    out.available = true;
    let now = std::time::SystemTime::now();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            // Only count canonical archive files. Skip lockfiles,
            // *.tmp, *.corrupted*, and the README.
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let archive_file = name.ends_with(".parquet")
                || name.ends_with(".ndjson")
                || name.ends_with(".ndjson.zst")
                || name.ends_with(".ndjson.zstd");
            if !archive_file {
                continue;
            }
            let size = metadata.len();
            let modified = metadata
                .modified()
                .unwrap_or(std::time::UNIX_EPOCH);
            let age_days = now
                .duration_since(modified)
                .unwrap_or_default()
                .as_secs()
                / 86_400;
            out.total = out.total.saturating_add(size);
            if age_days <= 30 {
                out.le_30d = out.le_30d.saturating_add(size);
            } else if age_days <= 365 {
                out.between_30d_365d = out.between_30d_365d.saturating_add(size);
            } else {
                out.gt_365d = out.gt_365d.saturating_add(size);
            }
        }
    }
    out
}

/// Read `cas_blobs` totals from the SQLite file at `path`. Honest
/// empty when the file is missing or unreadable.
fn scan_cas_totals(path: &std::path::Path) -> CasTotals {
    let mut out = CasTotals {
        path: path.display().to_string(),
        ..CasTotals::default()
    };
    if !path.exists() {
        return out;
    }
    match cortex_storage::CasStore::open(path) {
        Ok(store) => {
            out.available = true;
            out.rows = store.total_blob_count().unwrap_or(0);
            // Sum `size` directly — `CasStore` does not expose a
            // bytes total today, so query the column.
            let bytes: i64 = store
                .conn()
                .query_row("SELECT COALESCE(SUM(size), 0) FROM cas_blobs", [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            out.bytes = bytes.max(0) as u64;
        }
        Err(e) => {
            tracing::warn!(path=%path.display(), error=%e, "cas store open failed");
        }
    }
    out
}

fn home_path() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}

/// `GET /v1/dashboard/tasks/{id}` — full proposal + sectioned
/// checklist + listing of `specs/`. Returns `404` when the id is not
/// found in either the active or archived tree.
async fn tasks_detail(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
) -> Response {
    match state.tasks.detail(&id) {
        Some(body) => (StatusCode::OK, Json(body)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "task_not_found", "id": id })),
        )
            .into_response(),
    }
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
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

    #[test]
    fn build_timeline_event_preserves_content_hash_and_full_preview_for_tool_call() {
        // Phase3 — mapper must round-trip `content_hash` and surface
        // the un-clipped body as `preview` for `tool_call` rows
        // whose `text` is below the 8 KiB cap.
        let mut hit = tool_call_hit("Edit", "small body", "Cortex", 100);
        hit.content_hash = Some("sha256:abc123".to_string());
        let ev = build_timeline_event(&hit);
        assert_eq!(ev.kind, "tool_call");
        assert_eq!(ev.content_hash.as_deref(), Some("sha256:abc123"));
        assert_eq!(ev.preview.as_deref(), Some("[Edit] small body"));
        assert!(!ev.preview_truncated);
    }

    #[test]
    fn build_timeline_event_clips_preview_at_8_kib_and_flips_truncated() {
        // Phase3 — bodies larger than `PREVIEW_BYTE_CAP` clip on a
        // char boundary and the wire flag flips so the GUI knows it
        // needs to fetch the full body via the per-id route.
        let large = "a".repeat(PREVIEW_BYTE_CAP + 32);
        let mut hit = tool_call_hit("Edit", &large, "Cortex", 100);
        hit.content_hash = Some("sha256:full".to_string());
        let ev = build_timeline_event(&hit);
        assert!(ev.preview_truncated, "preview_truncated must flip on overflow");
        let preview = ev.preview.expect("preview must be present");
        assert_eq!(
            preview.len(),
            PREVIEW_BYTE_CAP,
            "preview must clip at exactly PREVIEW_BYTE_CAP bytes"
        );
    }

    #[test]
    fn build_timeline_event_skips_preview_for_non_tool_call() {
        // Phase3 — turn rows keep `preview = None` because the row's
        // `detail` field already covers them and the wire shape stays
        // compact for the bulk of the timeline.
        let hit = turn_hit("plain prompt", "Cortex", 100);
        let ev = build_timeline_event(&hit);
        assert_eq!(ev.kind, "turn");
        assert!(ev.preview.is_none());
        assert!(!ev.preview_truncated);
    }

    #[tokio::test]
    async fn timeline_recent_filters_by_content_hash() {
        // Phase3 — `?content_hash=` collapses the timeline to rows
        // sharing the supplied fingerprint. Powers the Inspector's
        // dedupe / replay-detection workflow.
        let mut a = tool_call_hit("Edit", "first", "Cortex", 100);
        a.content_hash = Some("sha256:aaa".into());
        let mut b = tool_call_hit("Edit", "second", "Cortex", 200);
        b.content_hash = Some("sha256:bbb".into());
        let mut c = tool_call_hit("Edit", "third", "Cortex", 300);
        c.content_hash = Some("sha256:aaa".into());
        let lane = lane_with(vec![a, b, c]);
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(50),
                session_id: None,
                repo: Vec::new(),
                kind: None,
                content_hash: Some("sha256:aaa".into()),
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 2, "only the two `aaa` hits must surface");
        for row in &parsed {
            assert_eq!(row["content_hash"], "sha256:aaa");
        }
    }

    #[tokio::test]
    async fn timeline_recent_returns_newest_first_and_clips_titles() {
        let lane = lane_with(vec![
            turn_hit("oldest prompt", "Cortex", 100),
            turn_hit("middle prompt", "Cortex", 200),
            turn_hit("newest prompt", "Cortex", 300),
        ]);
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(2),
                session_id: None,
                repo: Vec::new(),
                kind: None,
                content_hash: None,
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: Some(99999),
                session_id: None,
                repo: Vec::new(),
                kind: None,
                content_hash: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 5 * 1024 * 1024)
            .await
            .unwrap();
        let parsed: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.len(), 500);
    }

    fn classifier_internal_turn_hit(session: &str, ts: i64) -> LaneHit {
        // Simulates one classifier-worker `claude -p` call captured
        // through the cortex-adapter. Real shape: user_message is
        // the rendered prompt template, assistant_message is the
        // classifier's structured JSON.
        let mut extras = BTreeMap::new();
        extras.insert("session_id".to_string(), Value::String(session.to_string()));
        extras.insert(
            "user_message".to_string(),
            Value::String(
                "You are an event classifier + graph extractor for the Cortex system.\nYou will receive a JSON array of events..."
                    .to_string(),
            ),
        );
        extras.insert(
            "assistant_message".to_string(),
            Value::String(
                "```json\n{\"events\":[{\"event_id\":\"01X\",\"kind_refinement\":\"test\"}]}\n```"
                    .to_string(),
            ),
        );
        LaneHit {
            doc_id: format!("archive|{session}"),
            text: "classifier prompt body".to_string(),
            repo: Some("Cortex".to_string()),
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
        }
    }

    #[tokio::test]
    async fn conversations_list_hides_classifier_worker_internal_turns() {
        // Regression: every classifier-worker `claude -p` call goes
        // through the cortex-adapter hooks and is published as a
        // Turn envelope. Without `is_internal_cortex_turn` they
        // flooded /v1/dashboard/conversations with one row per
        // classified event.
        let lane = lane_with(vec![
            classifier_internal_turn_hit("01CLASSIFIER0000000000000A", 100),
            classifier_internal_turn_hit("01CLASSIFIER0000000000000B", 200),
            turn_hit_in("01REALCHAT00000000000000001", "real user prompt", "Cortex", 300),
        ]);
        let state = DashboardState {
            lane,
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = conversations_list(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 1, "only the real chat must remain");
        assert_eq!(rows[0]["session_id"], "01REALCHAT00000000000000001");
    }

    #[test]
    fn is_internal_cortex_turn_recognises_classifier_and_analyzer_prompts() {
        let mut extras = BTreeMap::new();
        extras.insert(
            "user_message".to_string(),
            Value::String(
                "You are an event classifier + graph extractor for the Cortex system."
                    .to_string(),
            ),
        );
        let hit_classifier = LaneHit {
            doc_id: "x".into(),
            text: "".into(),
            repo: None,
            path: None,
            symbol: Some("turn".into()),
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras,
        };
        assert!(is_internal_cortex_turn(&hit_classifier));

        let mut extras = BTreeMap::new();
        extras.insert(
            "user_message".to_string(),
            Value::String(
                "You are analyzing one session of captured Claude Code activity."
                    .to_string(),
            ),
        );
        let hit_analyzer = LaneHit {
            doc_id: "x".into(),
            text: "".into(),
            repo: None,
            path: None,
            symbol: Some("turn".into()),
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras,
        };
        assert!(is_internal_cortex_turn(&hit_analyzer));

        // Real chat: no internal signature on either side.
        let mut extras = BTreeMap::new();
        extras.insert(
            "user_message".to_string(),
            Value::String("hey, can you fix this bug?".to_string()),
        );
        extras.insert(
            "assistant_message".to_string(),
            Value::String("Sure. Let me read the file.".to_string()),
        );
        let hit_real = LaneHit {
            doc_id: "x".into(),
            text: "hey, can you fix this bug?".into(),
            repo: None,
            path: None,
            symbol: Some("turn".into()),
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras,
        };
        assert!(!is_internal_cortex_turn(&hit_real));
    }

    #[tokio::test]
    async fn sessions_groups_by_session_id_and_sorts_by_recency() {
        let lane = lane_with(vec![
            turn_hit_in("01SESSIONA0000000000000001", "first ever", "Cortex", 100),
            turn_hit_in("01SESSIONA0000000000000001", "still session A", "Cortex", 200),
            turn_hit_in("01SESSIONB0000000000000002", "session B latest", "Vectorizer", 500),
        ]);
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
        let resp = timeline_recent(
            State(state),
            Query(TimelineQuery {
                limit: None,
                session_id: Some("01SESSIONB0000000000000002".to_string()),
                repo: Vec::new(),
                kind: None,
                content_hash: None,
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
        let state = DashboardState { lane, nexus: None, analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()), tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from("__tests_no_rulebook__"))), metadata: None, loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()) };
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

    #[tokio::test]
    async fn overview_carries_full_series_block_and_classifier_stub_flag() {
        let now = chrono::Utc::now().timestamp_millis();
        let lane = lane_with(vec![
            turn_hit("a", "Cortex", now - 30_000),
            tool_call_hit("Edit", "x", "Cortex", now - 60_000),
        ]);
        let state = DashboardState {
            lane,
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = overview(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        let series = &parsed["series"];
        assert_eq!(series["events_per_min"].as_array().unwrap().len(), 20);
        assert_eq!(series["pre_thinking_p95_ms"].as_array().unwrap().len(), 20);
        assert_eq!(series["violations_7d_daily"].as_array().unwrap().len(), 7);
        assert_eq!(series["classifier_cost_usd_today"].as_array().unwrap().len(), 24);
        assert_eq!(parsed["classifier_cost_unavailable_until_spec05"], true);
        // Tool-call hit landed in the window but seeded fixture has no
        // duration_ms on extras, so every P95 bucket is null.
        assert!(series["pre_thinking_p95_ms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.is_null()));
        // Cost ribbon is 24 zeros until spec-05.
        assert!(series["classifier_cost_usd_today"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_f64() == Some(0.0)));
    }

    #[tokio::test]
    async fn overview_p95_picks_real_value_when_lane_carries_duration_ms() {
        let now = chrono::Utc::now().timestamp_millis();
        // Two tool_call hits with duration_ms stamped on extras.
        // The dashboard helper should produce a non-null P95 for the
        // bucket they share.
        let mut hits: Vec<LaneHit> = Vec::new();
        for d in [10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            let mut extras: BTreeMap<String, Value> = BTreeMap::new();
            extras.insert(
                "duration_ms".to_string(),
                Value::Number(serde_json::Number::from(d)),
            );
            hits.push(LaneHit {
                doc_id: format!("archive|d{d}"),
                text: format!("dur={d}"),
                repo: Some("Cortex".to_string()),
                path: None,
                symbol: Some("tool_call:Edit".to_string()),
                content_hash: None,
                score: 1.0,
                ts: now - 30_000,
                severity: None,
                extras,
            });
        }
        let lane = lane_with(hits);
        let state = DashboardState {
            lane,
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = overview(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        let p95 = parsed["series"]["pre_thinking_p95_ms"].as_array().unwrap();
        let last = p95.last().unwrap();
        assert!(
            last.as_u64().is_some(),
            "expected P95 to be populated for the latest bucket, got {last:?}"
        );
        let v = last.as_u64().unwrap();
        assert!(
            (90..=100).contains(&v),
            "P95 of 10..100 step 10 should be in [90, 100], got {v}"
        );
    }

    #[tokio::test]
    async fn tools_stats_emits_seven_by_twentyfour_heatmap() {
        let lane = lane_with(vec![
            tool_call_hit("Edit", "x", "Cortex", 100),
            tool_call_hit("Read", "y", "Cortex", 200),
        ]);
        let state = DashboardState {
            lane,
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = tools_stats(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["heatmap"]["tz"], "UTC");
        let days = parsed["heatmap"]["days"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[0], "Mon");
        let cells = parsed["heatmap"]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 7);
        for row in cells {
            assert_eq!(row.as_array().unwrap().len(), 24);
        }
    }

    #[tokio::test]
    async fn trust_endpoint_returns_stub_until_spec14() {
        let lane = lane_with(Vec::new());
        let state = DashboardState {
            lane,
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = trust(State(state)).await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["source"], "stub_until_spec14");
        assert_eq!(parsed["models"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["repos"].as_array().unwrap().len(), 0);
        assert!(parsed["scores"].as_object().unwrap().is_empty());
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

    // -- phase9i — retention dashboard route tests -------------------

    fn metadata_state(metadata: cortex_storage::MetadataStore) -> DashboardState {
        DashboardState {
            lane: lane_with(Vec::new()),
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: Some(std::sync::Arc::new(std::sync::Mutex::new(metadata))),
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        }
    }

    fn seed_three_sweeps(store: &cortex_storage::MetadataStore) {
        // Stage payloads emulate what each phase9 sweeper writes into
        // `tier_transitions_json` — the dashboard merges them under
        // `stages.<name>` so the GUI can render per-stage counters.
        let now = chrono::Utc::now();
        // 1. tier sweep
        store.start_retention_sweep("01SWEEP", now, 0).unwrap();
        store
            .finish_retention_sweep(
                "01SWEEP",
                now,
                12,
                0,
                r#"{"sweep":{"turn:fp32->pq":12}}"#,
                "success",
            )
            .unwrap();
        // 2. parquet rollup
        let t2 = now + chrono::Duration::seconds(1);
        store.start_retention_sweep("01ROLLUP", t2, 0).unwrap();
        store
            .finish_retention_sweep(
                "01ROLLUP",
                t2,
                0,
                3,
                r#"{"parquet_rollup":{"merged":4,"dropped":3}}"#,
                "success",
            )
            .unwrap();
        // 3. cas vacuum
        let t3 = now + chrono::Duration::seconds(2);
        store.start_retention_sweep("01CASVAC", t3, 0).unwrap();
        store
            .finish_retention_sweep(
                "01CASVAC",
                t3,
                0,
                7,
                r#"{"cas_vacuum":{"blobs_dropped":7,"bytes_reclaimed":4096}}"#,
                "success",
            )
            .unwrap();
    }

    #[tokio::test]
    async fn retention_sweeps_returns_per_stage_counters() {
        let store = cortex_storage::MetadataStore::open_in_memory().unwrap();
        seed_three_sweeps(&store);
        let state = metadata_state(store);
        let resp = retention_sweeps(
            State(state),
            Query(RetentionSweepsQuery {
                limit: Some(10),
                since: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let arr = v.as_array().expect("array body");
        assert_eq!(arr.len(), 3);
        for row in arr {
            assert!(row.get("stages").and_then(|s| s.as_object()).is_some());
            assert!(row["sweep_id"].is_string());
            assert!(row["status"].is_string());
        }
        // Newest first: cas vacuum landed last.
        assert_eq!(arr[0]["sweep_id"], "01CASVAC");
        assert!(arr[0]["stages"]["cas_vacuum"]["blobs_dropped"]
            .as_u64()
            .unwrap()
            >= 7);
    }

    #[tokio::test]
    async fn retention_sweeps_honours_since_filter() {
        let store = cortex_storage::MetadataStore::open_in_memory().unwrap();
        seed_three_sweeps(&store);
        let state = metadata_state(store);
        // `since` set to the future MUST yield zero rows.
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let resp = retention_sweeps(
            State(state),
            Query(RetentionSweepsQuery {
                limit: Some(10),
                since: Some(future),
            }),
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn retention_sweeps_falls_back_to_empty_without_metadata() {
        // Cold-dev path — `state.metadata = None`. Body MUST be `[]`.
        let state = DashboardState {
            lane: lane_with(Vec::new()),
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = retention_sweeps(
            State(state),
            Query(RetentionSweepsQuery::default()),
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn scan_archive_age_buckets_classifies_files_by_mtime() {
        // Build a tiny archive: 3 fresh files (all ≤ 30 d by virtue
        // of just-created mtimes), one stale file we age via
        // `filetime` to land in the gt_365d bucket.
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            let p = dir.path().join(format!("event-{i}.parquet"));
            std::fs::write(&p, vec![0u8; 1024 * (i as usize + 1)]).unwrap();
        }
        // Non-archive files are ignored (.tmp / .corrupted / README).
        std::fs::write(dir.path().join("scratch.tmp"), b"ignore me").unwrap();
        std::fs::write(dir.path().join("README.md"), b"docs").unwrap();
        let buckets = scan_archive_age_buckets(dir.path());
        assert!(buckets.available);
        assert!(buckets.le_30d > 0, "expected fresh bytes in le_30d");
        assert_eq!(buckets.gt_365d, 0);
        assert_eq!(buckets.total, buckets.le_30d + buckets.between_30d_365d + buckets.gt_365d);
    }

    #[test]
    fn scan_archive_returns_unavailable_when_root_missing() {
        let buckets = scan_archive_age_buckets(std::path::Path::new(
            "/no/such/path/for/cortex/archive/test",
        ));
        assert!(!buckets.available);
        assert_eq!(buckets.total, 0);
    }

    #[tokio::test]
    async fn retention_state_reports_archive_bucket_for_fresh_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..15 {
            std::fs::write(
                dir.path().join(format!("hour-{i:02}.parquet")),
                vec![0u8; 256],
            )
            .unwrap();
        }
        // Point the env var at the temp dir before the handler runs.
        std::env::set_var("CORTEX_ARCHIVE_ROOT", dir.path());
        // Avoid touching the real CAS DB.
        std::env::set_var(
            "CORTEX_CAS_DB",
            dir.path().join("__nope__cas.sqlite").as_os_str(),
        );

        let state = DashboardState {
            lane: lane_with(Vec::new()),
            nexus: None,
            analyzer: std::sync::Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: std::sync::Arc::new(crate::tasks_loader::TaskLoader::new(
                std::path::PathBuf::from("__tests_no_rulebook__"),
            )),
            metadata: None,
            loader_metrics: std::sync::Arc::new(crate::LoaderMetrics::new()),
        };
        let resp = retention_state(State(state)).await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        std::env::remove_var("CORTEX_ARCHIVE_ROOT");
        std::env::remove_var("CORTEX_CAS_DB");
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["archive_bytes"]["available"].as_bool().unwrap());
        assert!(v["archive_bytes"]["le_30d"].as_u64().unwrap() > 0);
        assert_eq!(v["archive_bytes"]["gt_365d"].as_u64().unwrap(), 0);
        // Per-sweep next_runs all "never" until phase9k.
        let next = v["next_runs"].as_array().unwrap();
        assert!(next.iter().all(|r| r["next_run"] == "never"));
        assert!(next
            .iter()
            .any(|r| r["sweep"] == "metadata_reap" || r["sweep"] == "tier_sweep"));
    }
}
