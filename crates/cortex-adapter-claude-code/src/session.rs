//! Session / turn / tool-call correlation.
//!
//! Spec 10 §Session correlation: `session_id` is sticky for the
//! lifetime of a Claude Code instance, `turn_id` rolls over at every
//! `UserPromptSubmit` (and resets on `Stop`), `tool_call_id` is
//! short-lived and joined to `PreToolUse`/`PostToolUse` via Claude
//! Code's `tool_use_id`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Per-session state cached by the daemon.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// Sticky `Session.id`.
    pub session_id: String,
    /// Currently-active `Turn.id`. `None` between Stop and the next
    /// UserPromptSubmit.
    pub current_turn: Option<String>,
    /// `tool_use_id` → `tool_call_id` map for correlating
    /// `PreToolUse` ↔ `PostToolUse` pairs.
    pub tool_calls: HashMap<String, String>,
}

impl SessionState {
    /// Build a fresh state stamped with `session_id`.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            current_turn: None,
            tool_calls: HashMap::new(),
        }
    }
}

/// Thread-safe registry of session states keyed by `session_id`.
#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionState>>,
    /// Maps the inbound `CLAUDE_SESSION_ID` (or `__pid_<n>` when
    /// absent) to the canonical ULID we publish on every envelope.
    /// Spec 04's `session_id` regex `^[0-9A-HJ-KM-NP-TV-Z]{26}$` is
    /// strict, so we never echo a free-form hint back to the wire.
    hint_to_id: Mutex<HashMap<String, String>>,
}

impl SessionManager {
    /// Fresh manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve or synthesize a session id.
    ///
    /// Always returns a 26-char Crockford ULID. The (hint → ULID) and
    /// (pid → ULID) mappings are cached so every hook in the same
    /// Claude session resolves to the same canonical id.
    pub fn resolve_or_synthesize(&self, hint: Option<&str>, pid_fallback: u32) -> String {
        let key = match hint {
            Some(h) if !h.is_empty() => h.to_string(),
            _ => format!("__pid_{pid_fallback}"),
        };
        if let Ok(mut g) = self.hint_to_id.lock() {
            return g
                .entry(key)
                .or_insert_with(|| ulid::Ulid::new().to_string())
                .clone();
        }
        ulid::Ulid::new().to_string()
    }

    /// Register the state for `session_id` if it's new; otherwise
    /// return the existing state.
    pub fn ensure(&self, session_id: &str) -> SessionState {
        if let Ok(mut g) = self.sessions.lock() {
            return g
                .entry(session_id.to_string())
                .or_insert_with(|| SessionState::new(session_id))
                .clone();
        }
        SessionState::new(session_id)
    }

    /// Open a fresh turn for `session_id` and return its id.
    pub fn open_turn(&self, session_id: &str) -> String {
        let turn_id = format!("cc-turn-{}", ulid::Ulid::new());
        if let Ok(mut g) = self.sessions.lock() {
            let state = g
                .entry(session_id.to_string())
                .or_insert_with(|| SessionState::new(session_id));
            state.current_turn = Some(turn_id.clone());
            state.tool_calls.clear();
        }
        turn_id
    }

    /// Get the currently-active turn id, if any.
    pub fn current_turn(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).and_then(|s| s.current_turn.clone()))
    }

    /// Mark the turn closed (Stop hook). Subsequent
    /// `UserPromptSubmit` opens a new one.
    pub fn close_turn(&self, session_id: &str) {
        if let Ok(mut g) = self.sessions.lock() {
            if let Some(state) = g.get_mut(session_id) {
                state.current_turn = None;
            }
        }
    }

    /// Open a fresh tool-call slot anchored under `tool_use_id`. The
    /// returned id is what we stamp on the envelope; it joins
    /// PreToolUse / PostToolUse together.
    pub fn open_tool_call(&self, session_id: &str, tool_use_id: &str) -> String {
        let tool_call_id = format!("cc-tc-{}", ulid::Ulid::new());
        if let Ok(mut g) = self.sessions.lock() {
            let state = g
                .entry(session_id.to_string())
                .or_insert_with(|| SessionState::new(session_id));
            state
                .tool_calls
                .insert(tool_use_id.to_string(), tool_call_id.clone());
        }
        tool_call_id
    }

    /// Look up the existing `tool_call_id` for `tool_use_id`. `None`
    /// when no PreToolUse correlation was recorded — the post-call
    /// event will be flagged as orphan.
    pub fn lookup_tool_call(&self, session_id: &str, tool_use_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).and_then(|s| s.tool_calls.get(tool_use_id).cloned()))
    }

    /// Drop a tool-call after PostToolUse handles it so the map
    /// doesn't grow without bound.
    pub fn close_tool_call(&self, session_id: &str, tool_use_id: &str) {
        if let Ok(mut g) = self.sessions.lock() {
            if let Some(state) = g.get_mut(session_id) {
                state.tool_calls.remove(tool_use_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_canonical_ulid(s: &str) -> bool {
        s.len() == 26 && s.chars().all(|c| matches!(c,
            '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z'))
    }

    #[test]
    fn synthesizes_canonical_ulid_when_hint_missing() {
        let mgr = SessionManager::new();
        let id = mgr.resolve_or_synthesize(None, 1234);
        assert!(is_canonical_ulid(&id), "expected ULID, got {id}");
        // Same pid resolves to the same id across calls (cached).
        let id2 = mgr.resolve_or_synthesize(None, 1234);
        assert_eq!(id, id2);
    }

    #[test]
    fn maps_hint_to_canonical_ulid_consistently() {
        let mgr = SessionManager::new();
        let id1 = mgr.resolve_or_synthesize(Some("env-session-X"), 0);
        let id2 = mgr.resolve_or_synthesize(Some("env-session-X"), 0);
        assert!(is_canonical_ulid(&id1));
        assert_eq!(id1, id2, "same hint must resolve to the same ULID");
        let id3 = mgr.resolve_or_synthesize(Some("env-session-Y"), 0);
        assert_ne!(id1, id3, "different hint must resolve to different ULIDs");
    }

    #[test]
    fn open_turn_replaces_any_existing_active_turn() {
        let mgr = SessionManager::new();
        let s = "s1";
        mgr.ensure(s);
        let t1 = mgr.open_turn(s);
        let t2 = mgr.open_turn(s);
        assert_ne!(t1, t2);
        assert_eq!(mgr.current_turn(s), Some(t2));
    }

    #[test]
    fn close_turn_clears_current() {
        let mgr = SessionManager::new();
        let s = "s2";
        mgr.open_turn(s);
        mgr.close_turn(s);
        assert!(mgr.current_turn(s).is_none());
    }

    #[test]
    fn tool_call_round_trip_through_lookup() {
        let mgr = SessionManager::new();
        let s = "s3";
        let tc = mgr.open_tool_call(s, "tu-1");
        assert_eq!(mgr.lookup_tool_call(s, "tu-1"), Some(tc.clone()));
        mgr.close_tool_call(s, "tu-1");
        assert!(mgr.lookup_tool_call(s, "tu-1").is_none());
    }
}
