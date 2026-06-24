use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};

use crate::lanes::MemoryKeywordLane;

use super::{
    clip, collect_lane_hits, normalize_repo, session_id_of, symbol_to_kind, ts_to_clock_string,
    DashboardState,
};

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

pub(super) async fn timeline_recent(
    State(state): State<DashboardState>,
    Query(params): Query<TimelineQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let mut hits = collect_lane_hits(&state.lane);
    if let Some(sid) = params.session_id.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| session_id_of(h) == Some(sid));
    }
    if !params.repo.is_empty() {
        let allow: std::collections::HashSet<String> =
            params.repo.iter().map(|r| normalize_repo(r)).collect();
        hits.retain(|h| {
            h.repo
                .as_deref()
                .map(|r| allow.contains(&normalize_repo(r)))
                .unwrap_or(false)
        });
    }
    if let Some(kind) = params.kind.as_deref().filter(|k| !k.is_empty()) {
        hits.retain(|h| symbol_to_kind(h.symbol.as_deref()) == kind);
    }
    if let Some(hash) = params.content_hash.as_deref().filter(|s| !s.is_empty()) {
        hits.retain(|h| h.content_hash.as_deref() == Some(hash));
    }
    // Newest first by `ts`.
    hits.sort_by_key(|h| std::cmp::Reverse(h.ts));
    hits.truncate(limit);

    let events: Vec<TimelineEvent> = hits.iter().map(build_timeline_event).collect();
    (axum::http::StatusCode::OK, Json(events)).into_response()
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
pub(super) async fn timeline_stream(
    State(state): State<DashboardState>,
    Query(params): Query<TimelineQuery>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let lane = state.lane.clone();
    let session_filter = params.session_id.clone().filter(|s| !s.is_empty());
    let repo_filter: std::collections::HashSet<String> =
        params.repo.iter().map(|r| normalize_repo(r)).collect();
    let kind_filter = params.kind.clone().filter(|s| !s.is_empty());
    let content_hash_filter = params.content_hash.clone().filter(|s| !s.is_empty());

    // Per-subscriber loop. Polls the in-memory lane every 500 ms and
    // emits the diff against the previously-seen ids. Heartbeat
    // every 15 s decouples liveness signal from event volume.
    let stream = async_stream::stream! {
        // Phase14j — emit an immediate heartbeat so the response
        // body's first chunk lands inside the first millisecond.
        // Without this the stream sits silent for the full 15s
        // keep-alive interval, which (a) the Vite dev-proxy buffers
        // until the connection closes — surfacing in the GUI as
        // "stream cancelado", and (b) hides liveness from any
        // proxy chain that flushes only on first chunk. Browsers
        // ignore the heartbeat event type by default unless the
        // caller subscribes to it; the GUI's `useSSE` hook DOES
        // subscribe to refresh its `lastHeartbeatAt` gauge.
        yield Ok::<SseEvent, Infallible>(
            SseEvent::default()
                .event("heartbeat")
                .data(r#"{"ok":true,"phase":"open"}"#)
        );

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
pub(super) fn filtered_hits(
    lane: &MemoryKeywordLane,
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
        hits.retain(|h| {
            h.repo
                .as_deref()
                .map(|r| repo_filter.contains(&normalize_repo(r)))
                .unwrap_or(false)
        });
    }
    if let Some(kind) = kind_filter {
        hits.retain(|h| symbol_to_kind(h.symbol.as_deref()) == kind);
    }
    if let Some(hash) = content_hash_filter {
        hits.retain(|h| h.content_hash.as_deref() == Some(hash));
    }
    hits.sort_by_key(|h| h.ts);
    hits
}

pub(super) fn build_timeline_event(h: &crate::lanes::LaneHit) -> TimelineEvent {
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
        title: super::title_from_hit(h),
        detail: clip(&h.text, 280),
        repo: h.repo.clone(),
        session_id: session_id_of(h).map(String::from),
        model: "claude-code".to_string(),
        content_hash: h.content_hash.clone(),
        preview,
        preview_truncated,
    }
}

pub(super) fn encode_sse(event: &TimelineEvent) -> Result<SseEvent, Infallible> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    Ok(SseEvent::default()
        .id(event.id.clone())
        .event("timeline")
        .data(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum_extra::extract::Query;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn make_state(hits: Vec<crate::lanes::LaneHit>) -> super::super::DashboardState {
        let lane = crate::lanes::MemoryKeywordLane::new();
        lane.seed("cortex-code", hits);
        super::super::DashboardState {
            lane: Arc::new(lane),
            nexus: None,
            analyzer: Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: Arc::new(crate::tasks_loader::MultiTaskLoader::new(vec![
                crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from(
                    "__tests_no_rulebook__",
                )),
            ])),
            metadata: None,
            loader_metrics: Arc::new(crate::LoaderMetrics::new()),
            temporal_metrics: Arc::new(crate::TemporalMetrics::new()),
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
            acl_metrics: None,
        }
    }

    fn turn_hit(text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        turn_hit_in("session-default", text, repo, ts)
    }

    fn turn_hit_in(session: &str, text: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String(session.to_string()),
        );
        crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        }
    }

    fn tool_call_hit(tool: &str, body: &str, repo: &str, ts: i64) -> crate::lanes::LaneHit {
        crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[test]
    fn build_timeline_event_preserves_content_hash_and_full_preview_for_tool_call() {
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
        let large = "a".repeat(PREVIEW_BYTE_CAP + 32);
        let mut hit = tool_call_hit("Edit", &large, "Cortex", 100);
        hit.content_hash = Some("sha256:full".to_string());
        let ev = build_timeline_event(&hit);
        assert!(
            ev.preview_truncated,
            "preview_truncated must flip on overflow"
        );
        let preview = ev.preview.expect("preview must be present");
        assert_eq!(
            preview.len(),
            PREVIEW_BYTE_CAP,
            "preview must clip at exactly PREVIEW_BYTE_CAP bytes"
        );
    }

    #[test]
    fn build_timeline_event_skips_preview_for_non_tool_call() {
        let hit = turn_hit("plain prompt", "Cortex", 100);
        let ev = build_timeline_event(&hit);
        assert_eq!(ev.kind, "turn");
        assert!(ev.preview.is_none());
        assert!(!ev.preview_truncated);
    }

    #[tokio::test]
    async fn timeline_recent_filters_by_content_hash() {
        let mut a = tool_call_hit("Edit", "first", "Cortex", 100);
        a.content_hash = Some("sha256:aaa".into());
        let mut b = tool_call_hit("Edit", "second", "Cortex", 200);
        b.content_hash = Some("sha256:bbb".into());
        let mut c = tool_call_hit("Edit", "third", "Cortex", 300);
        c.content_hash = Some("sha256:aaa".into());
        let state = make_state(vec![a, b, c]);
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
        let state = make_state(vec![
            turn_hit("oldest prompt", "Cortex", 100),
            turn_hit("middle prompt", "Cortex", 200),
            turn_hit("newest prompt", "Cortex", 300),
        ]);
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
    async fn timeline_clamps_limit_to_max() {
        let state = make_state(
            (0..600)
                .map(|i| turn_hit(&format!("p{i}"), "X", i))
                .collect(),
        );
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

    #[tokio::test]
    async fn timeline_filter_by_session_id_only_returns_matching_rows() {
        let state = make_state(vec![
            turn_hit_in("01SESSIONA0000000000000001", "alpha", "Cortex", 100),
            turn_hit_in("01SESSIONB0000000000000002", "beta", "Cortex", 200),
        ]);
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
}
