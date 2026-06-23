//! phase24 §1 — consolidator trigger producer.
//!
//! The consolidator daemon (`daemon::ConsolidatorDaemon`) is event-driven:
//! it dispatches one consolidation per envelope read off the Synap stream
//! [`daemon::TRIGGER_STREAM`] (`cortex.consolidator.triggers`). Until this
//! module landed, nothing published to that stream in the live stack — the
//! daemon ran with `dispatched=0` and the `cortex_consolidations` /
//! `cortex_topic_cards` indexes were never created, leaving
//! `cortex_similar_sessions`, `cortex_topic_search`, and the
//! `cortex_consolidations_*` MCP surfaces empty.
//!
//! This module builds the trigger envelopes (§3 wire shape in
//! `docs/specs/27-consolidation.md`). The classifier worker hosts the
//! decision-landed hook (§1.4): it already sees every enriched event with
//! its [`Kind`] and holds a Synap publisher, so a `Kind::Decision` event
//! fans out one `decision_landed` trigger. The producer is gated behind a
//! default-off config flag because the decision-trace grain auto-promotes
//! to Opus — firing it per decision is real spend the operator must opt
//! into.
//!
//! Re-triggering the same decision is safe: the daemon writes a
//! `producer_checkpoints` row keyed on `decision:<decision_id>` (spec 27
//! §2.4), so a decision already consolidated is skipped on the next fire.

use std::collections::BTreeMap;

use cortex_core::events::{DecisionPayload, Kind, TopicCardPayload};
use serde_json::{json, Value};

use crate::embedder::EnrichedEvent;
use crate::topic_cards::trigger::{Trigger as TopicTrigger, TriggerDecision};

/// phase24 §1.2 — a session is treated as ended after this much quiet
/// (30 minutes). Conservative: a real Claude Code session rarely pauses
/// this long mid-work, so the window avoids firing a `session_end`
/// (and its Opus-promoting session-grain consolidation) on a coffee
/// break while still closing sessions promptly once the user stops.
pub const SESSION_IDLE_MS: i64 = 30 * 60 * 1000;

/// Build a `session_end` trigger envelope (spec 27 §3) for `session_id`,
/// or `None` when the id is blank.
///
/// Wire shape:
/// ```json
/// { "kind": "session_end", "session_id": "01HXSESS..." }
/// ```
#[must_use]
pub fn session_end_trigger(session_id: &str) -> Option<Value> {
    let id = session_id.trim();
    if id.is_empty() {
        return None;
    }
    Some(json!({ "kind": "session_end", "session_id": id }))
}

/// phase24 §1.2 — idle-window session-end detection.
///
/// The Claude Code `Stop` hook lands as an ordinary `Kind::Turn` (no
/// distinct session-end envelope), so a session boundary can't be read
/// off a single event. Instead the producer tracks the last-seen time
/// per session and treats a session as ended once it has been quiet for
/// `idle_ms`.
///
/// Call this once per processed event: it stamps `now_ms` for the
/// event's own session (`active_session`, kept alive), then returns
/// every OTHER session whose last activity predates `now_ms - idle_ms`,
/// **pruning** them from `last_seen` so each fires its `session_end`
/// trigger exactly once. A re-opened session simply re-enters the map.
/// (A fully quiet stream leaves the final idle session unflagged until
/// the next event arrives — acceptable: no traffic means no
/// consolidation urgency.)
#[must_use]
pub fn evaluate_idle_sessions(
    last_seen: &mut BTreeMap<String, i64>,
    active_session: Option<&str>,
    now_ms: i64,
    idle_ms: i64,
) -> Vec<String> {
    if let Some(s) = active_session.map(str::trim).filter(|s| !s.is_empty()) {
        last_seen.insert(s.to_string(), now_ms);
    }
    let ended: Vec<String> = last_seen
        .iter()
        .filter(|(_, &ts)| now_ms - ts > idle_ms)
        .map(|(k, _)| k.clone())
        .collect();
    for k in &ended {
        last_seen.remove(k);
    }
    ended
}

/// Build a `decision_landed` trigger envelope for a `Kind::Decision`
/// event, or `None` when the event is not a decision or carries no
/// usable `decision_id`.
///
/// Wire shape (spec 27 §3):
/// ```json
/// { "kind": "decision_landed", "decision_id": "ADR-0042", "force_deep": false }
/// ```
#[must_use]
pub fn decision_landed_trigger(event: &EnrichedEvent) -> Option<Value> {
    if !matches!(event.kind, Kind::Decision) {
        return None;
    }
    let payload: DecisionPayload = serde_json::from_value(event.redacted_payload.clone()).ok()?;
    if payload.decision_id.trim().is_empty() {
        return None;
    }
    Some(json!({
        "kind": "decision_landed",
        "decision_id": payload.decision_id,
        "force_deep": false,
    }))
}

/// phase24 §1.3 — topic-grain trigger via the topic-card threshold
/// evaluator.
///
/// Reuses [`crate::topic_cards::trigger::Trigger::evaluate`] (the same
/// event-count / high-impact-proximity / staleness heuristics the topic
/// synthesiser uses): when it returns [`TriggerDecision::Rewrite`], the
/// topic for `repo` has crossed the rewrite threshold, so we publish a
/// `nightly_topic` trigger and the daemon's topic grain re-consolidates
/// that repo's clusters. A `Hold` decision (or a blank `repo`) publishes
/// nothing. `card == None` is the first-emit path — the evaluator always
/// rewrites, so a repo with no card yet gets its first topic
/// consolidation.
///
/// The topic grain is repo-scoped (it clusters within a repo), so the
/// trigger reuses the `nightly_topic` wire shape regardless of whether a
/// nightly cron or this threshold fired it — both ask the topic grain to
/// re-cluster the same repo.
///
/// Wire shape (spec 27 §3, parsed by `cortex-consolidator::parse_trigger`):
/// ```json
/// { "kind": "nightly_topic", "repo": "cortex" }
/// ```
#[must_use]
pub fn topic_threshold_trigger(
    repo: &str,
    card: Option<&TopicCardPayload>,
    new_event_kind: Kind,
    embedding_distance_fn: &dyn Fn() -> f32,
    synthesis_age_d: u32,
) -> Option<Value> {
    let repo = repo.trim();
    if repo.is_empty() {
        return None;
    }
    match TopicTrigger::evaluate(card, new_event_kind, embedding_distance_fn, synthesis_age_d) {
        TriggerDecision::Rewrite => Some(json!({ "kind": "nightly_topic", "repo": repo })),
        TriggerDecision::Hold { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use serde_json::json;

    fn classifier_stub(event_id: &str) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.to_string(),
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            sensitivity: Default::default(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn event(kind: Kind, payload: Value) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "evt-1".into(),
            kind,
            content_hash: "h-1".into(),
            redacted_payload: payload,
            classifier: classifier_stub("evt-1"),
            context_repo: Some("cortex".into()),
            context_path: Some(".rulebook/decisions/042.md".into()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        }
    }

    #[test]
    fn decision_event_yields_trigger() {
        let ev = event(
            Kind::Decision,
            json!({
                "decision_id": "ADR-0042",
                "title": "Adopt Meili",
                "status": "accepted",
                "body": "body",
                "tags": []
            }),
        );
        let trigger = decision_landed_trigger(&ev).expect("decision yields a trigger");
        assert_eq!(trigger["kind"], "decision_landed");
        assert_eq!(trigger["decision_id"], "ADR-0042");
        assert_eq!(trigger["force_deep"], false);
    }

    #[test]
    fn non_decision_event_yields_nothing() {
        let ev = event(Kind::Turn, json!({ "user_message": "hi" }));
        assert!(decision_landed_trigger(&ev).is_none());
    }

    #[test]
    fn decision_without_id_yields_nothing() {
        let ev = event(
            Kind::Decision,
            json!({
                "decision_id": "",
                "title": "No id",
                "status": "proposed",
                "body": "body",
                "tags": []
            }),
        );
        assert!(decision_landed_trigger(&ev).is_none());
    }

    #[test]
    fn malformed_decision_payload_yields_nothing() {
        let ev = event(Kind::Decision, json!({ "not": "a decision" }));
        assert!(decision_landed_trigger(&ev).is_none());
    }

    #[test]
    fn session_end_trigger_builds_envelope() {
        let t = session_end_trigger("01HXSESS00000000000000000A").expect("non-blank id");
        assert_eq!(t["kind"], "session_end");
        assert_eq!(t["session_id"], "01HXSESS00000000000000000A");
    }

    #[test]
    fn session_end_trigger_rejects_blank() {
        assert!(session_end_trigger("   ").is_none());
        assert!(session_end_trigger("").is_none());
    }

    #[test]
    fn idle_session_fires_once_and_is_pruned() {
        let idle_ms = 60_000;
        let mut seen = BTreeMap::new();
        // t=0: session A active.
        assert!(evaluate_idle_sessions(&mut seen, Some("A"), 0, idle_ms).is_empty());
        // t=30s: B active, A still within window → nothing ends.
        assert!(evaluate_idle_sessions(&mut seen, Some("B"), 30_000, idle_ms).is_empty());
        // t=70s: C active; A last seen at 0 → 70s idle > 60s → A ends.
        let ended = evaluate_idle_sessions(&mut seen, Some("C"), 70_000, idle_ms);
        assert_eq!(ended, vec!["A".to_string()]);
        // A pruned → never fires again even far in the future.
        let again = evaluate_idle_sessions(&mut seen, Some("C"), 200_000, idle_ms);
        assert!(
            !again.contains(&"A".to_string()),
            "A must not re-fire: {again:?}"
        );
    }

    #[test]
    fn active_session_is_never_flagged_as_idle() {
        let mut seen = BTreeMap::new();
        // A keeps being the active session every tick — it is refreshed to
        // `now_ms` each call, so it is never older than the window.
        for t in [0, 100_000, 200_000, 300_000] {
            assert!(evaluate_idle_sessions(&mut seen, Some("A"), t, 60_000).is_empty());
        }
    }

    #[test]
    fn boundary_exactly_at_idle_ms_is_not_yet_ended() {
        let mut seen = BTreeMap::new();
        let _ = evaluate_idle_sessions(&mut seen, Some("A"), 0, 60_000);
        // Exactly idle_ms later: 60_000 - 0 == 60_000, not > 60_000 → alive.
        let ended = evaluate_idle_sessions(&mut seen, Some("B"), 60_000, 60_000);
        assert!(
            ended.is_empty(),
            "exactly-at-threshold is not yet ended: {ended:?}"
        );
    }

    // ---- phase24 §1.3 — topic-threshold trigger ----

    fn topic_card(events_since: u32) -> TopicCardPayload {
        TopicCardPayload {
            topic_card_id: cortex_core::events::derive_topic_card_id("auth", "cortex"),
            topic_slug: "auth".into(),
            repos: vec!["cortex".into()],
            revision: 1,
            synthesis_markdown: "x".repeat(250),
            evidence: vec![],
            contradictions: vec![],
            open_questions: vec![],
            related_topic_ids: vec![],
            confidence: 0.9,
            last_rev_at: "2026-01-01T00:00:00Z".into(),
            events_since_last_rev: events_since,
            synthesis_model: "claude-haiku-4-5".into(),
            synthesis_cost_cents: 5,
        }
    }

    #[test]
    fn topic_threshold_first_emit_fires_nightly_topic() {
        // No card yet → evaluator always rewrites → publish for the repo.
        let t = topic_threshold_trigger("cortex", None, Kind::Turn, &|| 1.0, 0)
            .expect("first-emit fires");
        assert_eq!(t["kind"], "nightly_topic");
        assert_eq!(t["repo"], "cortex");
    }

    #[test]
    fn topic_threshold_event_count_fires() {
        use crate::topic_cards::trigger::TRIGGER_EVENTS_THRESHOLD;
        let card = topic_card(TRIGGER_EVENTS_THRESHOLD);
        let t = topic_threshold_trigger("cortex", Some(&card), Kind::Turn, &|| 1.0, 0)
            .expect("at threshold fires");
        assert_eq!(t["kind"], "nightly_topic");
        assert_eq!(t["repo"], "cortex");
    }

    #[test]
    fn topic_threshold_hold_publishes_nothing() {
        // Fresh, low-impact, below event threshold → Hold → no trigger.
        let card = topic_card(0);
        assert!(topic_threshold_trigger("cortex", Some(&card), Kind::Turn, &|| 1.0, 0).is_none());
    }

    #[test]
    fn topic_threshold_blank_repo_publishes_nothing() {
        // Even a rewrite decision must not fire without a repo to scope.
        assert!(topic_threshold_trigger("  ", None, Kind::Turn, &|| 1.0, 0).is_none());
    }
}
