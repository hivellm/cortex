//! Dashboard backend (spec 16, MVP slice).
//!
//! Three read endpoints under `/v1/dashboard/*`. The Electron GUI in
//! `gui/` is the consumer — `cortex-api` does not serve any HTML or
//! JS itself; it only answers JSON. Production targets (SSE, OIDC,
//! the rest of the spec-16 surface) live under §1–§9 of
//! `phase2_dashboard/tasks.md`.

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use nexus_sdk::NexusClient;
use serde::Serialize;
use serde_json::Value;

use crate::lanes::MemoryKeywordLane;
use crate::tasks_loader::MultiTaskLoader;
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
    pub tasks: Arc<MultiTaskLoader>,
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
    /// Phase18 §7.2 — shared temporal/branch/cross-project metrics
    /// registry. Bumped from the orchestrator's classifier + branch +
    /// cross-project wedges; rendered alongside `loader_metrics` on
    /// the Prometheus `/metrics` endpoint.
    pub temporal_metrics: Arc<crate::TemporalMetrics>,
    /// Phase11m §2.4 — push channel carrying dashboard delta events
    /// from the file-system watcher (and, in §4, the Synap consumer).
    /// SSE subscribers on `/v1/dashboard/stream` fan these out to GUI
    /// clients. Cloning is cheap (`Arc` inside).
    pub events_bus: crate::dashboard_watcher::DashboardEventBus,
}

// ---------------------------------------------------------------------------
// Submodule declarations
// ---------------------------------------------------------------------------

mod overview;
use self::overview::overview;

mod timeline;
use self::timeline::{timeline_recent, timeline_stream};

mod session_timeline;
pub use self::session_timeline::handle_session_timeline;

mod stream;
use self::stream::dashboard_stream;

mod memory;
use self::memory::memory;

mod decisions;
use self::decisions::{decision_detail, decisions};

mod laws;
use self::laws::laws;

mod violations;
use self::violations::violations;

mod analyses;
use self::analyses::{analyses, analysis_detail};

mod tools_stats;
use self::tools_stats::tools_stats;

mod trust;
use self::trust::trust;

mod sessions;
use self::sessions::sessions;

mod conversations;
use self::conversations::{conversation_detail, conversation_summary, conversations_list};

mod handoffs;
use self::handoffs::handoffs;

mod classifications;
use self::classifications::classifications;

mod graph;
use self::graph::graph;

mod tasks;
use self::tasks::{tasks_detail, tasks_list, tasks_summary};

mod retention;
use self::retention::{retention_state, retention_sweeps};

pub mod consolidations;
use self::consolidations::{consolidation_detail, consolidations};
pub use self::consolidations::{ConsolidationFilter, ConsolidationReportView};

mod coverage;
use self::coverage::coverage;

mod producers;
use self::producers::producers;

mod canary;
use self::canary::canary;

// Loaders + lifecycle modules absorbed into `dashboard/` during the
// 2026-05-25 reorg. Their pre-bucket paths (`crate::tasks_loader`,
// `crate::memory_tail`, `crate::dashboard_consumer`,
// `crate::dashboard_series`, `crate::dashboard_watcher`) are
// preserved via `pub use` re-exports in `lib.rs`.
pub mod consumer;
pub mod memory_tail;
pub mod series;
pub mod tasks_loader;
pub mod watcher;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the dashboard sub-router carrying the `/v1/dashboard/*` JSON
/// endpoints the GUI consumes. Endpoints whose upstream subsystem is
/// not built yet (laws / decisions / analyses — specs 13/14/15) still
/// answer with an honest empty list rather than mocked rows.
pub fn build_dashboard_router(state: DashboardState) -> Router {
    Router::new()
        .route("/v1/dashboard/overview", get(overview))
        .route("/v1/dashboard/timeline/recent", get(timeline_recent))
        .route("/v1/dashboard/timeline/stream", get(timeline_stream))
        .route("/v1/dashboard/stream", get(dashboard_stream))
        .route("/v1/dashboard/memory", get(memory))
        .route("/v1/dashboard/decisions", get(decisions))
        .route("/v1/dashboard/laws", get(laws))
        .route("/v1/dashboard/violations", get(violations))
        .route("/v1/dashboard/analyses", get(analyses))
        .route("/v1/dashboard/analyses/{id}", get(analysis_detail))
        .route("/v1/dashboard/consolidations", get(consolidations))
        .route(
            "/v1/dashboard/consolidations/{id}",
            get(consolidation_detail),
        )
        .route("/v1/dashboard/tools/stats", get(tools_stats))
        .route("/v1/dashboard/graph", get(graph))
        .route("/v1/dashboard/sessions", get(sessions))
        .route("/v1/dashboard/trust", get(trust))
        .route("/v1/dashboard/decisions/{id}", get(decision_detail))
        .route("/v1/dashboard/conversations", get(conversations_list))
        .route(
            "/v1/dashboard/conversations/{session_id}",
            get(conversation_detail),
        )
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
        .route("/v1/dashboard/coverage", get(coverage))
        .route("/v1/dashboard/producers", get(producers))
        .route("/v1/dashboard/canary", get(canary))
        .route(
            "/v1/dashboard/active-work",
            get(crate::active_work::active_work_handler),
        )
        // phase11w — admin lane projection used by
        // `cortex-ops tool-call-digest --apply` (and any other
        // operator binary that needs to walk the keyword lane's
        // event view without re-parsing parquet).
        .route(
            "/v1/admin/list-events",
            get(crate::admin_list_events::handle_list_events),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Shared helpers — pub(super) so every submodule can call them
// ---------------------------------------------------------------------------

/// Pull the `session_id` extras a hit was stamped with by the
/// archive loader. Helper kept inline so all sites share the same
/// fall-through logic.
pub(super) fn session_id_of(hit: &crate::lanes::LaneHit) -> Option<&str> {
    hit.extras
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Canonicalise a repo identifier to ASCII lowercase. Repo names enter
/// the lane from many sources (walker, archives, hand-edited handoffs)
/// with inconsistent case (`Cortex` vs `cortex`). Treating them as the
/// same project at every aggregation + filter boundary keeps the GUI's
/// repo list deduplicated and makes click-filters match regardless of
/// the captured spelling.
pub(super) fn normalize_repo(r: &str) -> String {
    r.to_ascii_lowercase()
}

/// Map the `symbol` field (e.g. `"tool_call:Edit"`) of a lane hit onto
/// one of the canonical kind labels the dashboard exposes. Used by
/// every handler that buckets or filters by kind; kept in the parent
/// so all submodules share the same mapping without re-importing.
pub(super) fn symbol_to_kind(symbol: Option<&str>) -> &'static str {
    match symbol {
        Some(s) if s.starts_with("tool_call") => "tool_call",
        Some(s) if s.starts_with("agent_call") => "agent_call",
        Some("decision") => "decision",
        Some("analysis") => "analysis",
        Some("law_violation") => "law_violation",
        // phase10e — knowledge / learnings now have their own
        // dedicated kinds; without these branches they collapsed
        // into the catch-all "turn" bucket and the dashboard's
        // Memory tab never surfaced them distinctly.
        Some("knowledge") => "knowledge",
        Some("learning") => "learning",
        // `memory` is the canonical kind the meili_loader stamps on
        // hits projected from `.rulebook/{handoff,specs,knowledge,
        // learnings}/**` — without this branch they collapsed into
        // "turn" and the Handoffs endpoint never matched them.
        Some("memory") => "memory",
        Some("consolidation") => "consolidation",
        Some("turn") | None => "turn",
        Some(_) => "turn",
    }
}

/// One row of the per-kind breakdown. Shared by overview, sessions,
/// classifications, and tools_stats submodules.
#[derive(Debug, Clone, Serialize)]
pub struct KindCount {
    /// Canonical kind label (`turn` / `tool_call` / `agent_call`).
    pub kind: String,
    /// Number of events with that kind.
    pub count: u64,
}

/// One row of the per-repo breakdown. Shared by overview and
/// classifications submodules.
#[derive(Debug, Clone, Serialize)]
pub struct RepoCount {
    /// Repo name (best-effort from `context.repo`).
    pub repo: String,
    /// Event count.
    pub count: u64,
}

/// phase10f — canonical set the dashboard accepts on
/// `/v1/dashboard/memory?kind=...` (plus `?facets=...` alias).
/// Mirrors [`symbol_to_kind`]'s output domain. Any other value
/// surfaces as a structured `400 unknown_kind` so callers see
/// the bug at the API boundary instead of silently getting an
/// empty result set.
pub(super) const CANONICAL_KINDS: &[&str] = &[
    "turn",
    "tool_call",
    "agent_call",
    "memory",
    "decision",
    "analysis",
    "law_violation",
    "knowledge",
    "learning",
];

/// Resolve the user-supplied `?kind=` and `?facets=` lists into
/// the canonical filter set. Returns `Err((unknown_value))` when
/// any value is outside [`CANONICAL_KINDS`] so the handler can
/// emit the structured 400 directly. An empty selection (no
/// `kind` AND no `facets`) returns `Ok(None)` — the caller
/// treats `None` as "all kinds".
pub(super) fn resolve_kind_filter(
    kinds: &[String],
    facets: &[String],
) -> Result<Option<std::collections::HashSet<String>>, String> {
    let merged: Vec<&String> = kinds.iter().chain(facets.iter()).collect();
    if merged.iter().all(|k| k.is_empty()) {
        return Ok(None);
    }
    let mut allowed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in merged {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !CANONICAL_KINDS.contains(&trimmed) {
            return Err(trimmed.to_string());
        }
        allowed.insert(trimmed.to_string());
    }
    if allowed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(allowed))
    }
}

pub(super) fn title_from_hit(h: &crate::lanes::LaneHit) -> String {
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

pub(super) fn ts_to_clock_string(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

pub(super) fn clip(s: &str, max: usize) -> String {
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

pub(super) fn ts_to_relative(ts_ms: i64) -> String {
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

pub(super) fn collect_lane_hits(lane: &MemoryKeywordLane) -> Vec<crate::lanes::LaneHit> {
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
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
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
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
        principal: None,
    }
}

#[allow(dead_code)]
pub(crate) fn raw_value(v: &Value) -> Value {
    v.clone()
}

// ---------------------------------------------------------------------------
// Tests — only the shared-helper ts_to_relative lives here; every
// per-handler test lives in its own submodule file.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
