//! Hook → canonical Cortex envelope mapping.
//!
//! Spec 10 §Envelope mapping aligns to spec 04 (`cortex-core`): every
//! publishable hook becomes a [`cortex_core::events::Envelope`] whose
//! `kind` is one of the canonical eight (`turn` / `tool_call` /
//! `agent_call` / …). Hooks that don't have a canonical analogue
//! (`PreToolUse`, `Stop`, `SessionStart`, `Notification`) still fire
//! the synchronous `HookResponse` path (law-check verdicts,
//! pre-thinking bundles) but produce no published event — see the
//! mapping table in `docs/specs/10-claude-code-adapter.md` §Envelope
//! mapping.
//!
//! The dispatcher calls [`build_event`] which returns
//! `Option<Envelope>`. `None` means "this hook only ran the sync
//! path — no publish." That keeps the publisher queue free of
//! signals that have no canonical kind, which `cortex-ingestion`
//! would reject with HTTP 422.

use chrono::{SecondsFormat, Utc};
use cortex_core::canonical_json::canonicalize;
use cortex_core::events::{
    AgentCall, Context, Envelope, Kind, Stream, ToolCall, ToolCallOutput, Turn,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::session::SessionManager;

/// Caller / `tool` advertised on every envelope. Matches the
/// `tool` enum in `crates/cortex-core/schemas/envelope.schema.json`.
pub const TOOL_CLAUDE_CODE: &str = "claude-code";

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

    /// Canonical kind this hook publishes as, or `None` when the hook
    /// only fires the sync path.
    pub fn publishes_as(self) -> Option<Kind> {
        match self {
            HookKind::UserPromptSubmit => Some(Kind::Turn),
            HookKind::PostToolUse => Some(Kind::ToolCall),
            HookKind::SubagentStop => Some(Kind::AgentCall),
            HookKind::PreToolUse
            | HookKind::Stop
            | HookKind::SessionStart
            | HookKind::Notification => None,
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

/// Adapter-side correlation IDs that ride along under
/// `context.extras["claude_code"]`. Lets the indexing layer
/// reconstruct turn / tool-call lineage without polluting the
/// envelope itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeExtras {
    /// `cc-turn-<ulid>` — present once a turn has opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// `cc-tc-<ulid>` — present for `PostToolUse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// `true` when correlation could not pin the parent. Spec-10
    /// §Failure mode signal.
    #[serde(default, skip_serializing_if = "is_false")]
    pub orphan: bool,
    /// Echoed `tool_use_id` from Claude Code's hook payload. Lets us
    /// match Pre↔Post pairs when re-indexing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Build a canonical envelope for a hook frame. Returns `None` for
/// the non-publishable hooks (`PreToolUse`, `Stop`, `SessionStart`,
/// `Notification`) — those still fire the sync path upstream of
/// this call but do not become published events.
///
/// Side-effect: updates the [`SessionManager`] correlation tables
/// (open turn / tool call / close turn) regardless of whether the
/// hook publishes.
pub fn build_event(
    hook: HookKind,
    frame: &HookFrame,
    sessions: &SessionManager,
    pid: u32,
) -> Option<Envelope> {
    let session_id = sessions.resolve_or_synthesize(frame.session_id.as_deref(), pid);
    sessions.ensure(&session_id);

    // Update correlation state for every hook (incl. non-publishing
    // ones) so the next publishable hook sees the right ids.
    let extras = update_correlation(&session_id, hook, frame, sessions);

    let canonical_kind = hook.publishes_as()?;

    // Redact the hook payload before we destructure it into the
    // canonical per-kind shape.
    let mut redacted = frame.payload.clone();
    let report = cortex_core::redact::redact(&mut redacted);

    // Per-kind payload construction.
    let payload_value = match canonical_kind {
        Kind::Turn => build_turn_payload(&redacted),
        Kind::ToolCall => build_tool_call_payload(&redacted),
        Kind::AgentCall => build_agent_call_payload(&redacted),
        // The other canonical kinds aren't produced by Claude Code
        // hooks today; `publishes_as` above guards us from reaching
        // this branch.
        _ => return None,
    };

    let context = build_context(frame, &extras);
    let occurred_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let content_hash = canonical_sha256(&payload_value);

    Some(Envelope {
        event_id: cortex_core::ids::event_id(),
        schema_version: "1".to_string(),
        occurred_at,
        ingested_at: None,
        session_id,
        stream: Stream::Live,
        tool: TOOL_CLAUDE_CODE.to_string(),
        model: std::env::var("CLAUDE_MODEL").ok(),
        kind: canonical_kind,
        context,
        payload: payload_value,
        redactions: report.tokens,
        content_hash,
        parent_event_id: None,
    })
}

fn update_correlation(
    session_id: &str,
    hook: HookKind,
    frame: &HookFrame,
    sessions: &SessionManager,
) -> ClaudeCodeExtras {
    let mut extras = ClaudeCodeExtras::default();
    match hook {
        HookKind::UserPromptSubmit => {
            extras.turn_id = Some(sessions.open_turn(session_id));
        }
        HookKind::Stop => {
            extras.turn_id = sessions.current_turn(session_id);
            sessions.close_turn(session_id);
        }
        HookKind::PreToolUse => {
            extras.turn_id = sessions.current_turn(session_id);
            if extras.turn_id.is_none() {
                extras.orphan = true;
            }
            let tool_use_id = read_tool_use_id(&frame.payload);
            extras.tool_use_id = Some(tool_use_id.clone());
            extras.tool_call_id = Some(sessions.open_tool_call(session_id, &tool_use_id));
        }
        HookKind::PostToolUse => {
            extras.turn_id = sessions.current_turn(session_id);
            let tool_use_id = read_tool_use_id(&frame.payload);
            extras.tool_use_id = Some(tool_use_id.clone());
            match sessions.lookup_tool_call(session_id, &tool_use_id) {
                Some(id) => {
                    extras.tool_call_id = Some(id);
                    sessions.close_tool_call(session_id, &tool_use_id);
                }
                None => {
                    extras.orphan = true;
                    extras.tool_call_id =
                        Some(sessions.open_tool_call(session_id, &tool_use_id));
                    sessions.close_tool_call(session_id, &tool_use_id);
                }
            }
        }
        HookKind::SubagentStop | HookKind::Notification | HookKind::SessionStart => {
            extras.turn_id = sessions.current_turn(session_id);
        }
    }
    extras
}

fn build_turn_payload(redacted: &Value) -> Value {
    let user_message = read_string_field(redacted, "prompt")
        .or_else(|| read_string_field(redacted, "user_message"))
        .unwrap_or_default();
    let turn = Turn {
        user_message,
        assistant_message: None,
        tokens: None,
        tool_call_event_ids: Vec::new(),
    };
    serde_json::to_value(&turn).unwrap_or(Value::Null)
}

fn build_tool_call_payload(redacted: &Value) -> Value {
    let tool_name = read_string_field(redacted, "tool_name").unwrap_or_else(|| "unknown".into());
    let input = redacted.get("input").cloned().unwrap_or(json!({}));
    let input_obj = input.as_object().cloned().unwrap_or_default();
    let raw_output = redacted.get("output").or_else(|| redacted.get("response"));
    let output = raw_output.map(|v| ToolCallOutput {
        stdout: read_optional_string(v, "stdout"),
        stderr: read_optional_string(v, "stderr"),
        exit_code: v.get("exit_code").and_then(|x| x.as_i64()).map(|x| x as i32),
        truncated: v
            .get("truncated")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        cas_ref: read_optional_string(v, "cas_ref"),
        size: v.get("size").and_then(|x| x.as_u64()),
    });
    let exit_code_for_outcome = raw_output
        .and_then(|v| v.get("exit_code"))
        .and_then(|x| x.as_i64());
    let outcome = match exit_code_for_outcome {
        Some(0) | None => "success",
        Some(_) => "error",
    }
    .to_string();
    let duration_ms = redacted
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| redacted.get("duration").and_then(|v| v.as_u64()));

    let tc = ToolCall {
        tool_name,
        input: Value::Object(input_obj),
        output,
        duration_ms,
        touched: Vec::new(),
        outcome,
    };
    serde_json::to_value(&tc).unwrap_or(Value::Null)
}

fn build_agent_call_payload(redacted: &Value) -> Value {
    let agent_type = read_string_field(redacted, "agent_type")
        .or_else(|| read_string_field(redacted, "subagent_type"))
        .unwrap_or_else(|| "unknown".into());
    let description = read_string_field(redacted, "description")
        .or_else(|| read_string_field(redacted, "task"))
        .unwrap_or_default();
    let prompt = read_string_field(redacted, "prompt");
    let model = read_string_field(redacted, "model");
    let team_name = read_string_field(redacted, "team_name");
    let result = redacted.get("result").cloned();
    let duration_ms = redacted.get("duration_ms").and_then(|v| v.as_u64());
    let outcome = read_string_field(redacted, "outcome").unwrap_or_else(|| "success".into());

    let ac = AgentCall {
        agent_type,
        description,
        prompt,
        model,
        team_name,
        child_event_ids: Vec::new(),
        result,
        duration_ms,
        outcome,
    };
    serde_json::to_value(&ac).unwrap_or(Value::Null)
}

fn build_context(frame: &HookFrame, extras: &ClaudeCodeExtras) -> Context {
    let cwd = frame.cwd.clone();
    let repo = cwd.as_deref().and_then(repo_from_cwd);
    let user = std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok());
    let mut bag = std::collections::BTreeMap::new();
    bag.insert(
        "claude_code".to_string(),
        serde_json::to_value(extras).unwrap_or(Value::Null),
    );

    Context {
        repo,
        branch: None,
        commit: None,
        cwd,
        user,
        platform: detect_platform().to_string(),
        ide: Some("claude-code".to_string()),
        extras: bag,
    }
}

fn read_string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn read_optional_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        })
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

/// Spec-04 envelope `context.platform` enum: `win32` / `darwin` / `linux`.
fn detect_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
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
            session_id: Some("env-session-fixture".into()),
            cwd: Some("/repos/Vectorizer".into()),
            payload,
        }
    }

    #[test]
    fn user_prompt_submit_publishes_canonical_turn() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "hi" })),
            &mgr,
            42,
        )
        .expect("UserPromptSubmit must publish");
        assert_eq!(env.kind, Kind::Turn);
        assert_eq!(env.tool, TOOL_CLAUDE_CODE);
        assert_eq!(env.schema_version, "1");
        assert_eq!(env.stream, Stream::Live);
        assert_eq!(env.event_id.len(), 26);
        assert_eq!(env.session_id.len(), 26);
        let payload: Turn = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.user_message, "hi");
        assert!(payload.assistant_message.is_none());
        let extras = env
            .context
            .extras
            .get("claude_code")
            .expect("claude_code extras present");
        let cc: ClaudeCodeExtras = serde_json::from_value(extras.clone()).unwrap();
        assert!(cc.turn_id.is_some());
    }

    #[test]
    fn post_tool_publishes_canonical_tool_call() {
        let mgr = SessionManager::new();
        // Open a turn so the tool call isn't orphaned.
        build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "go" })),
            &mgr,
            42,
        );
        // Pre then Post — Pre returns None (sync only), Post publishes.
        assert!(build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({ "tool_name": "Edit", "tool_use_id": "tu-1" })
            ),
            &mgr,
            42,
        )
        .is_none());
        let env = build_event(
            HookKind::PostToolUse,
            &frame(
                "PostToolUse",
                json!({
                    "tool_name": "Edit",
                    "tool_use_id": "tu-1",
                    "input": { "file_path": "x.rs" },
                    "output": { "stdout": "ok", "exit_code": 0 },
                    "duration_ms": 12
                }),
            ),
            &mgr,
            42,
        )
        .expect("PostToolUse must publish");
        assert_eq!(env.kind, Kind::ToolCall);
        let tc: ToolCall = serde_json::from_value(env.payload).unwrap();
        assert_eq!(tc.tool_name, "Edit");
        assert_eq!(tc.outcome, "success");
        assert_eq!(tc.duration_ms, Some(12));
        assert_eq!(tc.output.as_ref().unwrap().exit_code, Some(0));
        let cc: ClaudeCodeExtras = serde_json::from_value(
            env.context
                .extras
                .get("claude_code")
                .cloned()
                .unwrap_or(Value::Null),
        )
        .unwrap();
        assert_eq!(cc.tool_use_id.as_deref(), Some("tu-1"));
        assert!(!cc.orphan);
    }

    #[test]
    fn pre_tool_use_does_not_publish() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({ "tool_name": "Bash", "tool_use_id": "tu-X" }),
            ),
            &mgr,
            42,
        );
        assert!(env.is_none(), "PreToolUse must not publish");
    }

    #[test]
    fn session_lifecycle_hooks_do_not_publish() {
        let mgr = SessionManager::new();
        for h in [HookKind::SessionStart, HookKind::Stop, HookKind::Notification] {
            let env = build_event(h, &frame("X", json!({})), &mgr, 42);
            assert!(env.is_none(), "{h:?} must not publish");
        }
    }

    #[test]
    fn redaction_strips_synthetic_secrets() {
        let mgr = SessionManager::new();
        // Open a turn first so the tool call isn't orphaned.
        build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "go" })),
            &mgr,
            42,
        );
        let env = build_event(
            HookKind::PostToolUse,
            &frame(
                "PostToolUse",
                json!({
                    "tool_name": "Bash",
                    "tool_use_id": "tu-secret",
                    "input": { "command": "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000" },
                    "output": { "stdout": "", "exit_code": 0 }
                }),
            ),
            &mgr,
            42,
        )
        .expect("PostToolUse must publish");
        let tc: ToolCall = serde_json::from_value(env.payload).unwrap();
        let body = tc.input["command"].as_str().unwrap_or_default();
        assert!(!body.contains("AKIAIOSFODNN7EXAMPLE0000"));
        assert!(!env.redactions.is_empty(), "redaction tokens must be recorded");
    }

    #[test]
    fn content_hash_starts_with_sha256_prefix() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "x" })),
            &mgr,
            42,
        )
        .unwrap();
        assert!(env.content_hash.starts_with("sha256:"));
    }

    #[test]
    fn envelope_round_trips_through_canonical_validator() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "validate me" })),
            &mgr,
            42,
        )
        .unwrap();
        let value = serde_json::to_value(&env).unwrap();
        cortex_core::validate_event(&value)
            .expect("adapter envelope must satisfy spec-04 schema");
    }
}
