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
    AgentCall, Context, Envelope, Kind, Stream, ToolCall, ToolCallOutput, TouchedArtifact, Turn,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::session::SessionManager;

/// Caller / `tool` advertised on every envelope. Matches the
/// `tool` enum in `crates/cortex-core/schemas/envelope.schema.json`.
pub const TOOL_CLAUDE_CODE: &str = "claude-code";

/// Phase11w §2.2 — `tool` value the OpenCode plugin
/// (`@hivellm/cortex-opencode-plugin`) stamps on every envelope it
/// posts to the daemon's HTTP listener. The envelope schema's
/// `tool` enum at `crates/cortex-core/schemas/envelope.schema.json`
/// carries the matching `"opencode"` entry — both literals move
/// together.
pub const TOOL_OPENCODE: &str = "opencode";

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
            // Stop fires a SECOND `Turn` envelope at end-of-turn so
            // the assistant_message gets captured. UserPromptSubmit's
            // envelope only knows the user's prompt; the assistant's
            // reply doesn't exist yet at that point. Both envelopes
            // share the same `turn_id` via `context.extras["claude_code"]`
            // so downstream (dashboard, RRF, archive readers) can fold
            // them into a single turn record.
            HookKind::Stop => Some(Kind::Turn),
            HookKind::PreToolUse | HookKind::SessionStart | HookKind::Notification => None,
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
        Kind::Turn => build_turn_payload(&redacted, hook),
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

    // Allocate the envelope's canonical event_id up-front so we can
    // (a) stamp it on the Envelope below, and (b) for Turn envelopes,
    // record it on the SessionManager so subsequent tool_call /
    // agent_call envelopes in this turn can reference it via
    // `parent_event_id`.
    let event_id = cortex_core::ids::event_id();

    // Anchor every child event under its parent Turn so the
    // graph writer can build the canonical Session->Turn->ToolCall
    // chain instead of falling back to Session->ToolCall.
    //
    // Earlier revisions used `extras.turn_id` (the `cc-turn-<ulid>`
    // correlation tag) as `parent_event_id`. That was wrong: spec-04
    // requires `parent_event_id` to match the canonical
    // `^[0-9A-HJ-KM-NP-TV-Z]{26}$` (26-char ULID, no prefix). The
    // graph writer accepted both shapes loosely, but `cortex-ingestion`
    // validates against the JSON schema and rejected every tool_call
    // — invisibly, until the 2026-04-28 logging fix made it loud.
    //
    // The canonical reference is the actual `event_id` of the Turn
    // envelope (recorded below via `sessions.record_turn_event_id`).
    // Turn envelopes themselves keep `parent_event_id = None` (a Turn
    // parented on itself is meaningless and the schema allows null).
    let parent_event_id = match canonical_kind {
        Kind::Turn => None,
        _ => sessions.current_turn_event_id(&session_id),
    };

    if matches!(canonical_kind, Kind::Turn) {
        // Stamp the new turn id BEFORE we publish — even if publish
        // is async, every subsequent build_event in this process
        // reads it back synchronously. Both UserPromptSubmit and
        // Stop emit a Turn envelope; the latter overwrites the
        // former so a late tool_call between them parents on the
        // most recent Turn (= the Stop envelope's id, when it has
        // already fired, otherwise the prompt-side Turn).
        sessions.record_turn_event_id(&session_id, &event_id);
    }

    Some(Envelope {
        event_id,
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
        parent_event_id,
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
                    extras.tool_call_id = Some(sessions.open_tool_call(session_id, &tool_use_id));
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

fn build_turn_payload(redacted: &Value, hook: HookKind) -> Value {
    let (user_message, assistant_message) = match hook {
        HookKind::UserPromptSubmit => {
            // The user's prompt is on the inbound hook payload. The
            // assistant_message can't be set yet — the model hasn't
            // replied. Stop's envelope below fills that side in.
            let user_message = read_string_field(redacted, "prompt")
                .or_else(|| read_string_field(redacted, "user_message"))
                .unwrap_or_default();
            (user_message, None)
        }
        HookKind::Stop => {
            // Claude Code's Stop hook payload carries `transcript_path`
            // — a JSONL of every user/assistant message in the
            // session. The most recent `assistant` line with a `text`
            // content block is the reply we need. Failure to read /
            // parse falls back to an empty message rather than
            // dropping the envelope (a recorded turn-close with no
            // text is still useful — it pins the boundary timestamp).
            let assistant_message = read_string_field(redacted, "transcript_path")
                .as_deref()
                .and_then(read_last_assistant_text);
            (String::new(), assistant_message)
        }
        // build_event guards against this branch — only the two
        // hooks above route into Kind::Turn — but the match needs
        // to be exhaustive over HookKind.
        _ => (String::new(), None),
    };
    let turn = Turn {
        user_message,
        assistant_message,
        tokens: None,
        tool_call_event_ids: Vec::new(),
    };
    serde_json::to_value(&turn).unwrap_or(Value::Null)
}

/// Walk a Claude Code transcript JSONL backwards to find the most
/// recent `assistant` message's text content. Each transcript line
/// has shape `{ "type": "assistant" | "user" | ..., "message":
/// { "role": "...", "content": [...] } }`, where the content array
/// can mix `{ "type": "text", "text": ... }` and `{ "type":
/// "tool_use", ... }` blocks. We concatenate every `text` block from
/// the last assistant line so a reply that arrived in multiple
/// streamed chunks lands as a single string.
///
/// Returns `None` when the file is missing, malformed, or contains
/// no assistant text — the caller treats that as "no message
/// available", not as a hard error.
fn read_last_assistant_text(path: &str) -> Option<String> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    let f = File::open(path).ok()?;
    let reader = BufReader::new(f);
    let mut last_text: Option<String> = None;
    for line in reader.lines().map_while(Result::ok) {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let content = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)?;
        let mut buf = String::new();
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(text);
                }
            }
        }
        if !buf.is_empty() {
            last_text = Some(buf);
        }
    }
    last_text
}

fn build_tool_call_payload(redacted: &Value) -> Value {
    let tool_name = read_string_field(redacted, "tool_name").unwrap_or_else(|| "unknown".into());
    // Claude Code's PostToolUse hook payload uses `tool_input` /
    // `tool_response` field names (not `input` / `output`); fall back
    // to the canonical names so the same builder works for any
    // upstream that sends the cleaner shape.
    let input = redacted
        .get("tool_input")
        .or_else(|| redacted.get("input"))
        .cloned()
        .unwrap_or(json!({}));
    let input_obj = input.as_object().cloned().unwrap_or_default();
    let raw_output = redacted
        .get("tool_response")
        .or_else(|| redacted.get("output"))
        .or_else(|| redacted.get("response"));
    let output = raw_output.map(|v| ToolCallOutput {
        stdout: read_optional_string(v, "stdout"),
        stderr: read_optional_string(v, "stderr"),
        exit_code: v
            .get("exit_code")
            .and_then(|x| x.as_i64())
            .map(|x| x as i32),
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

    // Spec 01 lists `touched` as "resolved post-call by adapter" —
    // Claude Code's PostToolUse hook does not surface a structured
    // touched-files list, so the adapter infers it from each tool's
    // canonical `input` field. Earlier revisions left this empty,
    // which collapsed every TOUCHED edge in the graph (only one
    // edge across the entire bootstrap dataset). The mapping below
    // covers the file-mutating tools the agent actually uses; Bash
    // is intentionally skipped because its touched set requires
    // real shell parsing — `git status` doesn't touch a file the
    // way `Edit` does, and a wrong inference is worse than none.
    let touched = infer_touched_artifacts(&tool_name, &input_obj);

    let tc = ToolCall {
        tool_name,
        input: Value::Object(input_obj),
        output,
        duration_ms,
        touched,
        outcome,
    };
    serde_json::to_value(&tc).unwrap_or(Value::Null)
}

/// Best-effort `touched[]` inference from the canonical PostToolUse
/// `input` bag. Pulls `file_path` (and a couple of common synonyms)
/// for the tools whose semantics map cleanly onto a single file
/// kind. Tools whose touched set is multi-file (`Glob`, `Grep`) or
/// shell-shaped (`Bash`) are left empty for the writer to backfill
/// later from the tool's `output`.
fn infer_touched_artifacts(
    tool_name: &str,
    input: &serde_json::Map<String, Value>,
) -> Vec<TouchedArtifact> {
    fn pull_path(input: &serde_json::Map<String, Value>) -> Option<String> {
        for key in ["file_path", "path", "filename", "notebook_path"] {
            if let Some(Value::String(s)) = input.get(key) {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
        }
        None
    }
    let kind = match tool_name {
        "Read" | "NotebookRead" => "file_read",
        "Write" => "file_create",
        "Edit" | "MultiEdit" | "NotebookEdit" => "file_write",
        _ => return Vec::new(),
    };
    pull_path(input)
        .map(|path| {
            vec![TouchedArtifact {
                kind: kind.to_string(),
                path,
            }]
        })
        .unwrap_or_default()
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
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok());
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
    value.get(field).and_then(|v| match v {
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
    // phase10d — `repo` is canonical lowercase across every Cortex
    // surface (walker emit, lane projection, scope filter,
    // dashboard wire shape). Lowercasing the cwd basename here
    // keeps adapter-emitted envelopes aligned with bootstrap
    // envelopes so a `scope.repo = "Cortex"` query and a
    // `scope.repo = "cortex"` query hit the same rows.
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
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
    fn tool_call_parent_event_id_is_the_turn_envelope_event_id_not_the_correlation_tag() {
        // Regression for the 2026-04-28 silent-loss incident: the
        // adapter used to populate `parent_event_id` with the
        // `extras.turn_id` correlation tag (`cc-turn-<ulid>`), which
        // failed the spec-04 schema's `^[0-9A-HJ-KM-NP-TV-Z]{26}$`
        // pattern and got every tool_call rejected by ingestion at
        // the validate step. The fix is to reference the Turn
        // envelope's actual `event_id` (a clean ULID) so the chain
        // round-trips through validation.
        let mgr = SessionManager::new();
        let turn_env = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "go" })),
            &mgr,
            42,
        )
        .expect("UserPromptSubmit must publish");
        // Turn envelopes have parent_event_id = None.
        assert!(turn_env.parent_event_id.is_none());

        // Pre creates the correlation slot but does not publish.
        build_event(
            HookKind::PreToolUse,
            &frame(
                "PreToolUse",
                json!({ "tool_name": "Edit", "tool_use_id": "tu-7" }),
            ),
            &mgr,
            42,
        );
        let tc_env = build_event(
            HookKind::PostToolUse,
            &frame(
                "PostToolUse",
                json!({
                    "tool_name": "Edit",
                    "tool_use_id": "tu-7",
                    "input": { "file_path": "x.rs" },
                    "output": { "stdout": "ok", "exit_code": 0 },
                    "duration_ms": 1
                }),
            ),
            &mgr,
            42,
        )
        .expect("PostToolUse must publish");

        let parent = tc_env
            .parent_event_id
            .clone()
            .expect("tool_call must reference its parent Turn");
        // Must equal the Turn envelope's event_id verbatim.
        assert_eq!(parent, turn_env.event_id);
        // And must satisfy the canonical ULID shape (26 chars, no
        // `cc-turn-` prefix).
        assert_eq!(parent.len(), 26);
        assert!(
            parent.chars().all(|c| matches!(c,
                '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z')),
            "parent_event_id must be a clean Crockford ULID, got {parent}"
        );
        // The correlation tag in extras still carries the prefix —
        // they're two different identifiers tracked independently.
        let cc: ClaudeCodeExtras = serde_json::from_value(
            tc_env
                .context
                .extras
                .get("claude_code")
                .cloned()
                .unwrap_or(Value::Null),
        )
        .unwrap();
        let cc_turn = cc.turn_id.expect("extras.turn_id present");
        assert!(cc_turn.starts_with("cc-turn-"), "got {cc_turn}");
        assert_ne!(cc_turn, parent);
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
        // SessionStart and Notification stay sync-only — no canonical
        // kind exists for them. Stop now publishes a Kind::Turn
        // envelope carrying the assistant_message (see
        // `stop_emits_turn_envelope_with_assistant_message_from_transcript`).
        for h in [HookKind::SessionStart, HookKind::Notification] {
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
        assert!(
            !env.redactions.is_empty(),
            "redaction tokens must be recorded"
        );
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
    fn stop_emits_turn_envelope_with_assistant_message_from_transcript() {
        // Write a tempfile transcript matching Claude Code's JSONL
        // shape: a few user/attachment lines, then an assistant
        // message with both a tool_use block (which we ignore) and a
        // text block (which we keep). The Stop hook must produce a
        // Kind::Turn envelope where assistant_message reflects the
        // text content and user_message is empty (the prompt-side
        // already shipped on UserPromptSubmit).
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let transcript_lines = [
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"go"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"final reply line one"},{"type":"text","text":"line two"}]}}"#,
        ];
        std::fs::write(tmp.path(), transcript_lines.join("\n")).unwrap();
        let path_str = tmp.path().to_string_lossy().to_string();

        let mgr = SessionManager::new();
        // Open a turn first so the Stop envelope's correlation
        // carries a turn_id (the dashboard fold-by-turn-id only
        // works when the same turn_id is on both halves).
        let user_env = build_event(
            HookKind::UserPromptSubmit,
            &frame("UserPromptSubmit", json!({ "prompt": "do the thing" })),
            &mgr,
            42,
        )
        .expect("UserPromptSubmit must publish");

        let stop_env = build_event(
            HookKind::Stop,
            &frame(
                "Stop",
                json!({ "transcript_path": path_str, "stop_hook_active": false }),
            ),
            &mgr,
            42,
        )
        .expect("Stop must publish a Kind::Turn envelope");

        assert_eq!(stop_env.kind, Kind::Turn);
        let payload: Turn = serde_json::from_value(stop_env.payload).unwrap();
        assert_eq!(
            payload.user_message, "",
            "Stop envelope leaves user_message empty — the prompt side rode the UserPromptSubmit envelope"
        );
        assert_eq!(
            payload.assistant_message.as_deref(),
            Some("final reply line one\nline two"),
            "Stop envelope must carry the last assistant text (joined across content blocks)"
        );
        // Correlation: both envelopes carry the same turn_id under
        // context.extras.claude_code so downstream readers can fold
        // them.
        let user_turn = user_env
            .context
            .extras
            .get("claude_code")
            .and_then(|v| v.get("turn_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let stop_turn = stop_env
            .context
            .extras
            .get("claude_code")
            .and_then(|v| v.get("turn_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        assert!(user_turn.is_some());
        assert_eq!(user_turn, stop_turn, "turn_id must match across the pair");
    }

    #[test]
    fn stop_with_missing_transcript_path_still_publishes_envelope() {
        // A Stop hook with no transcript_path (or a path that points
        // at nothing) must still produce a Turn envelope — the
        // boundary timestamp is itself useful, and refusing to
        // publish would silently drop turn-end signals on any
        // session that hasn't written a transcript yet.
        let mgr = SessionManager::new();
        let env = build_event(HookKind::Stop, &frame("Stop", json!({})), &mgr, 42)
            .expect("Stop must publish even without transcript_path");
        let payload: Turn = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.user_message, "");
        assert_eq!(payload.assistant_message, None);
    }

    #[test]
    fn stop_with_malformed_transcript_returns_no_assistant_message() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "this is not jsonl\n{not json either").unwrap();
        let path_str = tmp.path().to_string_lossy().to_string();

        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::Stop,
            &frame("Stop", json!({ "transcript_path": path_str })),
            &mgr,
            42,
        )
        .expect("Stop publishes regardless of transcript health");
        let payload: Turn = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.assistant_message, None);
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
        cortex_core::validate_event(&value).expect("adapter envelope must satisfy spec-04 schema");
    }
}
