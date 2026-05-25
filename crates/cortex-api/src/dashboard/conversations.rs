use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::{clip, collect_lane_hits, session_id_of, symbol_to_kind, DashboardState};

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
pub(super) fn turn_id_of(hit: &crate::lanes::LaneHit) -> Option<String> {
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
pub(super) fn is_internal_cortex_turn(hit: &crate::lanes::LaneHit) -> bool {
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

pub(super) async fn conversations_list(State(state): State<DashboardState>) -> Response {
    let hits = collect_lane_hits(&state.lane);

    // Group turn-kind hits by session. `is_internal_cortex_turn`
    // drops classifier-worker / analyzer CLI invocations so the
    // panel shows only real user chats — those tooling calls were
    // creating one session row per classified event.
    let mut by_session: std::collections::BTreeMap<String, Vec<crate::lanes::LaneHit>> =
        std::collections::BTreeMap::new();
    for h in hits
        .into_iter()
        .filter(|h| symbol_to_kind(h.symbol.as_deref()) == "turn" && !is_internal_cortex_turn(h))
    {
        if let Some(sid) = session_id_of(&h) {
            by_session.entry(sid.to_string()).or_default().push(h);
        }
    }

    let mut rows: Vec<ConversationSummary> = by_session
        .into_iter()
        .map(|(session_id, mut bucket)| {
            bucket.sort_by_key(|h| h.ts);
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

    rows.sort_by_key(|r| std::cmp::Reverse(r.last_at_ms));
    (StatusCode::OK, Json(rows)).into_response()
}

pub(super) async fn conversation_detail(
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
    turns.sort_by_key(|t| t.started_at_ms);

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
pub(super) async fn conversation_summary(
    State(state): State<DashboardState>,
    Path(session_id): Path<String>,
) -> Response {
    match state
        .analyzer
        .summarize_session(&state.lane, &session_id)
        .await
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
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
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
        }
    }

    fn classifier_internal_turn_hit(session: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String(session.to_string()),
        );
        extras.insert(
            "user_message".to_string(),
            serde_json::Value::String(
                "You are an event classifier + graph extractor for the Cortex system.\nYou will receive a JSON array of events..."
                    .to_string(),
            ),
        );
        extras.insert(
            "assistant_message".to_string(),
            serde_json::Value::String(
                "```json\n{\"events\":[{\"event_id\":\"01X\",\"kind_refinement\":\"test\"}]}\n```"
                    .to_string(),
            ),
        );
        crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        }
    }

    fn real_turn_hit(session: &str, text: &str, ts: i64) -> crate::lanes::LaneHit {
        let mut extras = BTreeMap::new();
        extras.insert(
            "session_id".to_string(),
            serde_json::Value::String(session.to_string()),
        );
        crate::lanes::LaneHit {
            doc_id: format!("archive|{}", text),
            text: text.to_string(),
            repo: Some("Cortex".to_string()),
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

    #[tokio::test]
    async fn conversations_list_hides_classifier_worker_internal_turns() {
        let state = make_state(vec![
            classifier_internal_turn_hit("01CLASSIFIER0000000000000A", 100),
            classifier_internal_turn_hit("01CLASSIFIER0000000000000B", 200),
            real_turn_hit("01REALCHAT00000000000000001", "real user prompt", 300),
        ]);
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
            serde_json::Value::String(
                "You are an event classifier + graph extractor for the Cortex system.".to_string(),
            ),
        );
        let hit_classifier = crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        };
        assert!(is_internal_cortex_turn(&hit_classifier));

        let mut extras = BTreeMap::new();
        extras.insert(
            "user_message".to_string(),
            serde_json::Value::String(
                "You are analyzing one session of captured Claude Code activity.".to_string(),
            ),
        );
        let hit_analyzer = crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        };
        assert!(is_internal_cortex_turn(&hit_analyzer));

        let mut extras = BTreeMap::new();
        extras.insert(
            "user_message".to_string(),
            serde_json::Value::String("hey, can you fix this bug?".to_string()),
        );
        extras.insert(
            "assistant_message".to_string(),
            serde_json::Value::String("Sure. Let me read the file.".to_string()),
        );
        let hit_real = crate::lanes::LaneHit {
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
            overlay: crate::lanes::Overlay::default(),
        };
        assert!(!is_internal_cortex_turn(&hit_real));
    }
}
