//! Hook → Cortex envelope mapping.
//!
//! Spec 10 §Envelope mapping: every Claude Code hook becomes one
//! envelope-compliant event whose `kind` carries the hook name. The
//! payload is a redacted projection of the hook's verbatim JSON.

use chrono::{DateTime, Utc};
use cortex_core::canonical_json::canonicalize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::session::SessionManager;

/// Coarse hook id Claude Code surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookKind {
    /// Session boot.
    SessionStart,
    /// User prompt submitted to the model.
    UserPromptSubmit,
    /// About to invoke a tool — synchronous law-check happens here.
    PreToolUse,
    /// Tool invocation finished.
    PostToolUse,
    /// Sub-agent finished.
    SubagentStop,
    /// Session stop / wrap-up.
    Stop,
    /// Notification (permission prompt, idle warning, …).
    Notification,
}

impl HookKind {
    /// Spec-10 §Envelope mapping `kind` string this hook resolves to.
    pub fn cortex_kind(self) -> &'static str {
        match self {
            HookKind::SessionStart => "turn.session_start",
            HookKind::UserPromptSubmit => "turn.user",
            HookKind::PreToolUse => "tool_call.requested",
            HookKind::PostToolUse => "tool_call.completed",
            HookKind::SubagentStop => "turn.subagent_complete",
            HookKind::Stop => "turn.session_stop",
            HookKind::Notification => "event.notification",
        }
    }

    /// Parse the hook discriminator coming from the wire. The hooks
    /// shim sends one of the variant names verbatim.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "SessionStart" => Some(Self::SessionStart),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            "SubagentStop" => Some(Self::SubagentStop),
            "Stop" => Some(Self::Stop),
            "Notification" => Some(Self::Notification),
            _ => None,
        }
    }
}

/// Wire shape of one hook frame the shim posts to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookFrame {
    /// Hook discriminator (matches [`HookKind`] PascalCase names).
    pub hook: String,
    /// Session id from `CLAUDE_SESSION_ID` env or the daemon's
    /// pid-keyed fallback.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Working directory at the time the hook fired.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Verbatim Claude Code hook JSON.
    #[serde(default)]
    pub payload: Value,
}

/// Built envelope ready for publication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeEvent {
    /// ULID.
    pub event_id: String,
    /// ms since epoch.
    pub ts: i64,
    /// `turn.user`, `tool_call.requested`, …
    pub kind: String,
    /// Always `"claude-code"`.
    pub adapter: String,
    /// Source object — repo + path + git_ref + symbol when available.
    pub source: Value,
    /// Active `Session.id`.
    pub session_id: String,
    /// Active `Turn.id`. `None` for SessionStart / Notification before
    /// the first prompt.
    pub turn_id: Option<String>,
    /// Active `ToolCall.id`. Populated for the two ToolUse hooks.
    pub tool_call_id: Option<String>,
    /// True when correlation could not pin the parent. Carries the
    /// spec-10 §Failure mode signal downstream.
    pub orphan: bool,
    /// Redacted payload.
    pub redacted_payload: Value,
    /// `sha256:<hex>` over `canonical_json(redacted_payload)`.
    pub content_hash: String,
    /// Number of redaction tokens recorded.
    pub redactions: u32,
}

/// Build the envelope for a hook frame. The redaction step happens
/// before this function returns so secrets never reach the publisher
/// queue.
pub fn build_event(
    hook: HookKind,
    frame: &HookFrame,
    sessions: &SessionManager,
    pid: u32,
) -> ClaudeEvent {
    let session_id = sessions
        .resolve_or_synthesize(frame.session_id.as_deref(), pid);
    sessions.ensure(&session_id);

    let turn_id: Option<String>;
    let mut tool_call_id: Option<String> = None;
    let mut orphan = false;

    match hook {
        HookKind::UserPromptSubmit => {
            turn_id = Some(sessions.open_turn(&session_id));
        }
        HookKind::Stop => {
            turn_id = sessions.current_turn(&session_id);
            sessions.close_turn(&session_id);
        }
        HookKind::PreToolUse => {
            turn_id = sessions.current_turn(&session_id);
            if turn_id.is_none() {
                orphan = true;
            }
            let tool_use_id = read_tool_use_id(&frame.payload);
            tool_call_id = Some(sessions.open_tool_call(&session_id, &tool_use_id));
        }
        HookKind::PostToolUse => {
            turn_id = sessions.current_turn(&session_id);
            let tool_use_id = read_tool_use_id(&frame.payload);
            match sessions.lookup_tool_call(&session_id, &tool_use_id) {
                Some(id) => {
                    tool_call_id = Some(id);
                    sessions.close_tool_call(&session_id, &tool_use_id);
                }
                None => {
                    orphan = true;
                    tool_call_id = Some(sessions.open_tool_call(&session_id, &tool_use_id));
                    sessions.close_tool_call(&session_id, &tool_use_id);
                }
            }
        }
        HookKind::SubagentStop | HookKind::Notification | HookKind::SessionStart => {
            turn_id = sessions.current_turn(&session_id);
        }
    }

    let mut payload = frame.payload.clone();
    let report = cortex_core::redact::redact(&mut payload);

    let mut source = json!({
        "adapter": "claude-code",
    });
    if let Some(cwd) = frame.cwd.as_deref() {
        source["cwd"] = Value::String(cwd.to_string());
        if let Some(repo) = repo_from_cwd(cwd) {
            source["repo"] = Value::String(repo);
        }
    }
    if let Ok(model) = std::env::var("CLAUDE_MODEL") {
        source["model"] = Value::String(model);
    }

    let now: DateTime<Utc> = Utc::now();
    let content_hash = canonical_sha256(&payload);

    ClaudeEvent {
        event_id: ulid::Ulid::new().to_string(),
        ts: now.timestamp_millis(),
        kind: hook.cortex_kind().to_string(),
        adapter: "claude-code".to_string(),
        source,
        session_id,
        turn_id,
        tool_call_id,
        orphan,
        redacted_payload: payload,
        content_hash,
        redactions: u32::try_from(report.tokens.len()).unwrap_or(u32::MAX),
    }
}

fn read_tool_use_id(payload: &Value) -> String {
    payload
        .get("tool_use_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("anon-{}", ulid::Ulid::new()))
}

fn repo_from_cwd(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn canonical_sha256(value: &Value) -> String {
    let bytes = match canonicalize(value) {
        Ok(b) => b,
        Err(_) => serde_json::to_vec(value).unwrap_or_default(),
    };
    let mut h = Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let mut out = String::from("sha256:");
    for b in digest.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(hook: &str, payload: Value) -> HookFrame {
        HookFrame {
            hook: hook.to_string(),
            session_id: Some("test-session".into()),
            cwd: Some("/repos/Vectorizer".into()),
            payload,
        }
    }

    #[test]
    fn hook_kind_round_trips_through_strings() {
        for h in [
            HookKind::SessionStart,
            HookKind::UserPromptSubmit,
            HookKind::PreToolUse,
            HookKind::PostToolUse,
            HookKind::SubagentStop,
            HookKind::Stop,
            HookKind::Notification,
        ] {
            let s = format!("{h:?}");
            assert_eq!(HookKind::parse(&s), Some(h), "round-trip {s}");
            assert!(!h.cortex_kind().is_empty());
        }
    }

    #[test]
    fn user_prompt_opens_turn_and_stamps_it() {
        let mgr = SessionManager::new();
        let evt = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "hi" })),
            &mgr,
            42,
        );
        assert_eq!(evt.kind, "turn.user");
        assert!(evt.turn_id.is_some());
        assert!(!evt.orphan);
        assert_eq!(evt.session_id, "test-session");
    }

    #[test]
    fn pre_tool_without_open_turn_flags_orphan() {
        let mgr = SessionManager::new();
        let evt = build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({ "tool_name": "Edit", "tool_use_id": "tu-1" }),
            ),
            &mgr,
            42,
        );
        assert_eq!(evt.kind, "tool_call.requested");
        assert!(evt.tool_call_id.is_some());
        assert!(evt.orphan, "pre-tool without parent turn must be orphan");
    }

    #[test]
    fn post_tool_correlates_with_pre_tool() {
        let mgr = SessionManager::new();
        // First open a turn so PreToolUse isn't orphaned.
        build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({})),
            &mgr,
            42,
        );
        let pre = build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({ "tool_name": "Edit", "tool_use_id": "tu-7" }),
            ),
            &mgr,
            42,
        );
        let post = build_event(
            HookKind::PostToolUse,
            &frame(
                "PostToolUse",
                json!({ "tool_name": "Edit", "tool_use_id": "tu-7" }),
            ),
            &mgr,
            42,
        );
        assert_eq!(pre.tool_call_id, post.tool_call_id);
        assert!(!post.orphan);
    }

    #[test]
    fn redaction_strips_synthetic_secrets() {
        let mgr = SessionManager::new();
        let evt = build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({
                    "tool_name": "Bash",
                    "tool_use_id": "tu-secret",
                    "input": { "command": "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000" }
                }),
            ),
            &mgr,
            42,
        );
        let body = evt
            .redacted_payload["input"]["command"]
            .as_str()
            .unwrap_or_default();
        assert!(!body.contains("AKIAIOSFODNN7EXAMPLE0000"));
        assert!(evt.redactions >= 1);
    }

    #[test]
    fn content_hash_starts_with_sha256_prefix() {
        let mgr = SessionManager::new();
        let evt = build_event(
            HookKind::SessionStart,
            &frame("SessionStart", json!({ "model": "claude-opus" })),
            &mgr,
            42,
        );
        assert!(evt.content_hash.starts_with("sha256:"));
        assert_eq!(evt.kind, "turn.session_start");
    }
}
