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

/// Phase36 — `tool` value for Codex CLI hook frames. Canonical live-capture
/// id from the envelope schema enum; the Claude archive classifier's
/// `openai-codex` dotted kinds are a separate, archive-only vocabulary.
pub const TOOL_CODEX: &str = "codex";

/// Client that emitted a hook frame (phase36). Serialized on the wire as
/// the envelope `tool` id. A frame without `client` is Claude Code, so
/// older shims keep working; an unknown value fails frame parsing, which
/// the daemon answers with `{}` instead of mislabelling the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HookClient {
    /// Claude Code (`claude-code`).
    #[default]
    #[serde(rename = "claude-code")]
    ClaudeCode,
    /// Codex CLI (`codex`).
    #[serde(rename = "codex")]
    Codex,
    /// OpenCode (`opencode`).
    #[serde(rename = "opencode")]
    OpenCode,
}

impl HookClient {
    /// Parse a wire / CLI client id. `None` for anything outside the
    /// three supported ids.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            TOOL_CLAUDE_CODE => Some(Self::ClaudeCode),
            TOOL_CODEX => Some(Self::Codex),
            TOOL_OPENCODE => Some(Self::OpenCode),
            _ => None,
        }
    }

    /// Envelope `tool` id and `context.ide` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => TOOL_CLAUDE_CODE,
            Self::Codex => TOOL_CODEX,
            Self::OpenCode => TOOL_OPENCODE,
        }
    }

    /// Key of this client's correlation bag under `context.extras`.
    pub fn extras_key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    /// Prefix of the turn / tool-call correlation tags. Claude keeps `cc`
    /// so ids published before phase36 stay stable.
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::ClaudeCode => "cc",
            Self::Codex => "cx",
            Self::OpenCode => "oc",
        }
    }
}

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
    /// Verbatim client hook JSON.
    #[serde(default)]
    pub payload: Value,
    /// Client that emitted the frame; absent means Claude Code.
    #[serde(default)]
    pub client: HookClient,
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

/// Phase36 — correlation bag for Codex frames, under
/// `context.extras["codex"]`. The shared correlation tags (`cx-turn-…`,
/// `cx-tc-…`) are flattened in beside the Codex-native identifiers read
/// from the hook payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexExtras {
    /// Cortex correlation tags, same fields as [`ClaudeCodeExtras`].
    #[serde(flatten)]
    pub correlation: ClaudeCodeExtras,
    /// Codex's own `turn_id` from turn-scoped hook payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_turn_id: Option<String>,
    /// `model` from the hook payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// SessionStart `source` (`startup`, `resume`, `clear`, `compact`, `fork`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `permission_mode` from the hook payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// SubagentStop `agent_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
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
        Kind::Turn => build_turn_payload(&redacted, hook, frame.client),
        Kind::ToolCall => build_tool_call_payload(&redacted, frame.client),
        Kind::AgentCall => build_agent_call_payload(&redacted, frame.client),
        // The other canonical kinds aren't produced by Claude Code
        // hooks today; `publishes_as` above guards us from reaching
        // this branch.
        _ => return None,
    };

    let context = build_context(frame, &extras, &redacted);
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
        tool: frame.client.as_str().to_string(),
        // CLAUDE_MODEL belongs to Claude Code's environment only.
        model: read_string_field(&redacted, "model").or_else(|| {
            (frame.client == HookClient::ClaudeCode)
                .then(|| std::env::var("CLAUDE_MODEL").ok())
                .flatten()
        }),
        kind: canonical_kind,
        context,
        payload: payload_value,
        redactions: report.tokens,
        content_hash,
        parent_event_id,
        class_level: None,
        class_compartments: None,
    })
}

fn update_correlation(
    session_id: &str,
    hook: HookKind,
    frame: &HookFrame,
    sessions: &SessionManager,
) -> ClaudeCodeExtras {
    let client = frame.client;
    let mut extras = ClaudeCodeExtras::default();
    match hook {
        HookKind::UserPromptSubmit => {
            extras.turn_id = Some(sessions.open_turn(session_id, client));
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
            extras.tool_call_id = Some(sessions.open_tool_call(session_id, &tool_use_id, client));
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
                    // The Codex plugin registers no PreToolUse, so a missing
                    // slot is expected there; only a missing turn orphans it.
                    extras.orphan = client != HookClient::Codex || extras.turn_id.is_none();
                    extras.tool_call_id =
                        Some(sessions.open_tool_call(session_id, &tool_use_id, client));
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

fn build_turn_payload(redacted: &Value, hook: HookKind, client: HookClient) -> Value {
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
        HookKind::Stop if client == HookClient::Codex => {
            // Codex hands the reply over in the payload. Its
            // `transcript_path` is nullable and its format is not a
            // stable contract, so it is never read.
            let assistant_message =
                read_string_field(redacted, "last_assistant_message").filter(|s| !s.is_empty());
            (String::new(), assistant_message)
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

fn build_tool_call_payload(redacted: &Value, client: HookClient) -> Value {
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
    let input_obj = match input {
        Value::Object(map) => map,
        // Codex tool inputs need not be objects (apply_patch may send the
        // raw patch text); keep them under `input` instead of dropping them.
        Value::Null => serde_json::Map::new(),
        other if client == HookClient::Codex => {
            serde_json::Map::from_iter([("input".to_string(), other)])
        }
        _ => serde_json::Map::new(),
    };
    let raw_output = redacted
        .get("tool_response")
        .or_else(|| redacted.get("output"))
        .or_else(|| redacted.get("response"));
    let output = match raw_output {
        // Codex documents no `tool_response` shape: text is the output,
        // other non-object JSON is kept serialized, null means none.
        Some(Value::Null) if client == HookClient::Codex => None,
        Some(Value::String(text)) if client == HookClient::Codex => Some(ToolCallOutput {
            stdout: Some(text.clone()),
            ..ToolCallOutput::default()
        }),
        Some(other) if client == HookClient::Codex && !other.is_object() => Some(ToolCallOutput {
            stdout: Some(other.to_string()),
            ..ToolCallOutput::default()
        }),
        raw => raw.map(|v| ToolCallOutput {
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
        }),
    };
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
    let touched = match client {
        HookClient::Codex => infer_codex_touched_artifacts(&tool_name, &input_obj),
        _ => infer_touched_artifacts(&tool_name, &input_obj),
    };

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

/// Phase36 — `touched[]` for Codex tools. Only `apply_patch` names its
/// files: the patch text (a bare string, or `command` / `patch` / `input`
/// in the input object) carries `*** Add File:`, `*** Update File:`,
/// `*** Delete File:` and `*** Move to:` headers. Shell and MCP tools
/// stay empty for the same reason Claude's Bash does.
fn infer_codex_touched_artifacts(
    tool_name: &str,
    input: &serde_json::Map<String, Value>,
) -> Vec<TouchedArtifact> {
    if tool_name != "apply_patch" {
        return Vec::new();
    }
    let Some(patch) = ["command", "patch", "input"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
    else {
        return Vec::new();
    };
    let mut touched: Vec<TouchedArtifact> = Vec::new();
    for line in patch.lines() {
        let entry = [
            ("*** Add File: ", "file_create"),
            ("*** Update File: ", "file_write"),
            ("*** Delete File: ", "file_delete"),
            ("*** Move to: ", "file_create"),
        ]
        .iter()
        .find_map(|(header, kind)| {
            line.strip_prefix(header)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(|path| TouchedArtifact {
                    kind: (*kind).to_string(),
                    path: path.to_string(),
                })
        });
        if let Some(entry) = entry {
            if !touched.contains(&entry) {
                touched.push(entry);
            }
        }
    }
    touched
}

fn build_agent_call_payload(redacted: &Value, client: HookClient) -> Value {
    let agent_type = read_string_field(redacted, "agent_type")
        .or_else(|| read_string_field(redacted, "subagent_type"))
        .unwrap_or_else(|| "unknown".into());
    let description = read_string_field(redacted, "description")
        .or_else(|| read_string_field(redacted, "task"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "agent call".into());
    let prompt = read_string_field(redacted, "prompt");
    let model = read_string_field(redacted, "model");
    let team_name = read_string_field(redacted, "team_name");
    // Codex SubagentStop carries the sub-agent's reply as
    // `last_assistant_message`; it is the call's result. The schema
    // requires `result` to be an object, so the text keeps its field name.
    let result = redacted.get("result").cloned().or_else(|| {
        (client == HookClient::Codex)
            .then(|| read_string_field(redacted, "last_assistant_message"))
            .flatten()
            .map(|text| json!({ "last_assistant_message": text }))
    });
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

fn build_context(frame: &HookFrame, extras: &ClaudeCodeExtras, redacted: &Value) -> Context {
    let cwd = frame.cwd.clone();
    let repo = cwd.as_deref().and_then(repo_from_cwd);
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok());
    let extras_value = match frame.client {
        HookClient::Codex => serde_json::to_value(CodexExtras {
            correlation: extras.clone(),
            codex_turn_id: read_string_field(redacted, "turn_id"),
            model: read_string_field(redacted, "model"),
            source: read_string_field(redacted, "source"),
            permission_mode: read_string_field(redacted, "permission_mode"),
            agent_id: read_string_field(redacted, "agent_id"),
        }),
        _ => serde_json::to_value(extras),
    };
    let mut bag = std::collections::BTreeMap::new();
    bag.insert(
        frame.client.extras_key().to_string(),
        extras_value.unwrap_or(Value::Null),
    );

    Context {
        repo,
        branch: None,
        commit: None,
        cwd,
        user,
        platform: detect_platform().to_string(),
        ide: Some(frame.client.as_str().to_string()),
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
            client: HookClient::ClaudeCode,
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

    #[test]
    fn subagent_stop_with_no_description_defaults_to_agent_call_string() {
        let mgr = SessionManager::new();
        // SubagentStop frame with no description field — mirrors the live hook payload
        let env = build_event(
            HookKind::SubagentStop,
            &frame("SubagentStop", json!({ "subagent_type": "code-reviewer" })),
            &mgr,
            42,
        )
        .expect("SubagentStop must publish");
        assert_eq!(env.kind, Kind::AgentCall);
        let payload: cortex_core::events::AgentCall = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.agent_type, "code-reviewer");
        assert_eq!(payload.description, "agent call");
        assert!(
            !payload.description.is_empty(),
            "description must be non-empty for schema minLength:1"
        );
    }

    #[test]
    fn subagent_stop_with_empty_description_defaults_to_agent_call_string() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::SubagentStop,
            &frame(
                "SubagentStop",
                json!({ "subagent_type": "researcher", "description": "" }),
            ),
            &mgr,
            42,
        )
        .expect("SubagentStop must publish");
        let payload: cortex_core::events::AgentCall = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.description, "agent call");
    }

    #[test]
    fn subagent_stop_with_description_preserves_it() {
        let mgr = SessionManager::new();
        let env = build_event(
            HookKind::SubagentStop,
            &frame(
                "SubagentStop",
                json!({ "subagent_type": "implementer", "description": "Implement feature X" }),
            ),
            &mgr,
            42,
        )
        .expect("SubagentStop must publish");
        let payload: cortex_core::events::AgentCall = serde_json::from_value(env.payload).unwrap();
        assert_eq!(payload.description, "Implement feature X");
    }

    // ── phase36: client identity + Codex payload mapping ─────────────────

    /// Fields every Codex hook payload carries (learn.chatgpt.com/docs/hooks).
    fn codex_payload(event: &str, specific: Value) -> Value {
        let mut payload = json!({
            "session_id": "019a7c1e-codex-session",
            "transcript_path": null,
            "cwd": "/repos/Vectorizer",
            "hook_event_name": event,
            "model": "gpt-5.5-codex",
        });
        if let (Some(base), Some(extra)) = (payload.as_object_mut(), specific.as_object()) {
            base.extend(extra.clone());
        }
        payload
    }

    fn client_frame(client: HookClient, hook: &str, payload: Value) -> HookFrame {
        HookFrame {
            client,
            ..frame(hook, payload)
        }
    }

    fn codex_event(mgr: &SessionManager, hook: HookKind, specific: Value) -> Option<Envelope> {
        let name = format!("{hook:?}");
        let frame = client_frame(HookClient::Codex, &name, codex_payload(&name, specific));
        build_event(hook, &frame, mgr, 42)
    }

    fn codex_extras(env: &Envelope) -> CodexExtras {
        assert!(!env.context.extras.contains_key("claude_code"));
        serde_json::from_value(env.context.extras["codex"].clone()).unwrap()
    }

    fn tool_call(mgr: &SessionManager, specific: Value) -> (ToolCall, CodexExtras) {
        let env = codex_event(mgr, HookKind::PostToolUse, specific).expect("PostToolUse publishes");
        cortex_core::validate_event(&serde_json::to_value(&env).unwrap()).unwrap();
        let extras = codex_extras(&env);
        (serde_json::from_value(env.payload).unwrap(), extras)
    }

    #[test]
    fn frame_client_defaults_to_claude_code_round_trips_and_rejects_unknown_ids() {
        let legacy: HookFrame =
            serde_json::from_value(json!({"hook": "Stop", "payload": {}})).unwrap();
        assert_eq!(legacy.client, HookClient::ClaudeCode);
        for client in [
            HookClient::ClaudeCode,
            HookClient::Codex,
            HookClient::OpenCode,
        ] {
            let wire = serde_json::to_value(client_frame(client, "Stop", json!({}))).unwrap();
            assert_eq!(wire["client"], client.as_str());
            assert_eq!(HookClient::parse(client.as_str()), Some(client));
            let back: HookFrame = serde_json::from_value(wire).unwrap();
            assert_eq!(back.client, client);
        }
        for unknown in [json!("gemini"), json!("Codex"), json!(""), json!(7)] {
            let raw = json!({"hook": "Stop", "payload": {}, "client": unknown});
            assert!(serde_json::from_value::<HookFrame>(raw).is_err());
        }
        assert_eq!(HookClient::parse("openai-codex"), None);
    }

    #[test]
    fn tool_ide_extras_and_id_prefix_follow_the_client() {
        for (client, extras_key, prefix) in [
            (HookClient::ClaudeCode, "claude_code", "cc-turn-"),
            (HookClient::Codex, "codex", "cx-turn-"),
            (HookClient::OpenCode, "opencode", "oc-turn-"),
        ] {
            let mgr = SessionManager::new();
            let frame = client_frame(client, "UserPromptSubmit", json!({"prompt": "hi"}));
            let env = build_event(HookKind::UserPromptSubmit, &frame, &mgr, 42).unwrap();
            assert_eq!(env.tool, client.as_str());
            assert_eq!(env.context.ide.as_deref(), Some(client.as_str()));
            assert_eq!(env.context.extras.len(), 1, "{client:?}");
            let turn = env.context.extras[extras_key]["turn_id"].as_str().unwrap();
            assert!(turn.starts_with(prefix), "{turn}");
            cortex_core::validate_event(&serde_json::to_value(&env).unwrap()).unwrap();
        }
    }

    #[test]
    fn codex_session_start_publishes_nothing_and_prompt_turn_carries_codex_ids() {
        let mgr = SessionManager::new();
        assert!(codex_event(&mgr, HookKind::SessionStart, json!({"source": "startup"})).is_none());
        let env = codex_event(
            &mgr,
            HookKind::UserPromptSubmit,
            json!({"turn_id": "turn-1", "permission_mode": "default", "prompt": "fix the bug"}),
        )
        .expect("UserPromptSubmit publishes");
        assert_eq!(env.kind, Kind::Turn);
        assert_eq!(env.tool, TOOL_CODEX);
        assert_eq!(env.model.as_deref(), Some("gpt-5.5-codex"));
        let turn: Turn = serde_json::from_value(env.payload.clone()).unwrap();
        assert_eq!(turn.user_message, "fix the bug");
        let extras = codex_extras(&env);
        assert!(extras
            .correlation
            .turn_id
            .as_deref()
            .is_some_and(|id| id.starts_with("cx-turn-")));
        assert_eq!(extras.codex_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(extras.model.as_deref(), Some("gpt-5.5-codex"));
        assert_eq!(extras.permission_mode.as_deref(), Some("default"));
        cortex_core::validate_event(&serde_json::to_value(&env).unwrap()).unwrap();
    }

    #[test]
    fn codex_model_never_falls_back_to_claude_model_env() {
        std::env::set_var("CLAUDE_MODEL", "claude-env-model");
        let mgr = SessionManager::new();
        let frame = client_frame(
            HookClient::Codex,
            "UserPromptSubmit",
            json!({"prompt": "no model field"}),
        );
        let env = build_event(HookKind::UserPromptSubmit, &frame, &mgr, 42).unwrap();
        assert_eq!(env.model, None);
    }

    #[test]
    fn codex_post_tool_use_maps_string_object_and_other_tool_responses() {
        let mgr = SessionManager::new();
        codex_event(&mgr, HookKind::UserPromptSubmit, json!({"prompt": "go"}));
        let (tc, extras) = tool_call(
            &mgr,
            json!({
                "turn_id": "turn-1",
                "tool_name": "shell",
                "tool_use_id": "call_1",
                "tool_input": {"command": ["bash", "-lc", "ls"]},
                "tool_response": "file-a\nfile-b"
            }),
        );
        assert_eq!(tc.tool_name, "shell");
        assert_eq!(tc.input["command"][2], "ls");
        let output = tc.output.expect("string response is output text");
        assert_eq!(output.stdout.as_deref(), Some("file-a\nfile-b"));
        assert_eq!(tc.outcome, "success");
        assert!(tc.touched.is_empty(), "shell commands are not parsed");
        // No PreToolUse in the Codex plugin: an open turn is enough.
        assert!(!extras.correlation.orphan);
        assert_eq!(extras.correlation.tool_use_id.as_deref(), Some("call_1"));
        assert!(extras
            .correlation
            .tool_call_id
            .as_deref()
            .is_some_and(|id| id.starts_with("cx-tc-")));
        assert_eq!(extras.codex_turn_id.as_deref(), Some("turn-1"));

        let (tc, _) = tool_call(
            &mgr,
            json!({
                "tool_name": "shell",
                "tool_use_id": "call_2",
                "tool_input": {"command": "false"},
                "tool_response": {"stdout": "", "stderr": "boom", "exit_code": 2}
            }),
        );
        let output = tc.output.expect("object response");
        assert_eq!(output.stderr.as_deref(), Some("boom"));
        assert_eq!(output.exit_code, Some(2));
        assert_eq!(tc.outcome, "error");

        let (tc, _) = tool_call(
            &mgr,
            json!({
                "tool_name": "mcp__cortex__cortex_memory_read",
                "tool_use_id": "call_3",
                "tool_input": {"repo": "Cortex"},
                "tool_response": [{"type": "text", "text": "saved"}]
            }),
        );
        assert_eq!(
            tc.output.and_then(|o| o.stdout).as_deref(),
            Some(r#"[{"type":"text","text":"saved"}]"#)
        );

        let (tc, _) = tool_call(
            &mgr,
            json!({"tool_name": "shell", "tool_use_id": "call_4", "tool_response": null}),
        );
        assert!(tc.output.is_none());
    }

    #[test]
    fn codex_post_tool_use_without_an_open_turn_is_orphaned() {
        let mgr = SessionManager::new();
        let (_, extras) = tool_call(
            &mgr,
            json!({"tool_name": "shell", "tool_use_id": "call_9", "tool_response": "x"}),
        );
        assert!(extras.correlation.orphan);
    }

    #[test]
    fn codex_apply_patch_touches_every_patched_path() {
        let patch = "*** Begin Patch\n\
                     *** Add File: src/new.rs\n\
                     +pub fn added() {}\n\
                     *** Update File: src/lib.rs\n\
                     *** Move to: src/renamed.rs\n\
                     @@\n\
                     -old\n\
                     +new\n\
                     *** Update File: src/lib.rs\n\
                     *** Delete File: docs/old.md\r\n\
                     *** End Patch\n";
        let expected = vec![
            ("file_create", "src/new.rs"),
            ("file_write", "src/lib.rs"),
            ("file_create", "src/renamed.rs"),
            ("file_delete", "docs/old.md"),
        ];
        let mgr = SessionManager::new();
        codex_event(&mgr, HookKind::UserPromptSubmit, json!({"prompt": "go"}));
        for (index, input) in [json!({"command": patch}), json!(patch)]
            .into_iter()
            .enumerate()
        {
            let (tc, _) = tool_call(
                &mgr,
                json!({
                    "tool_name": "apply_patch",
                    "tool_use_id": format!("patch-{index}"),
                    "tool_input": input,
                    "tool_response": "Success. Updated the following files:"
                }),
            );
            let touched: Vec<(&str, &str)> = tc
                .touched
                .iter()
                .map(|t| (t.kind.as_str(), t.path.as_str()))
                .collect();
            assert_eq!(touched, expected, "input shape {index}");
            assert!(
                tc.input.is_object(),
                "a bare-string input is kept as an object"
            );
        }
        // The same tool name from Claude Code is not parsed as a patch.
        let frame = frame(
            "PostToolUse",
            json!({"tool_name": "apply_patch", "tool_input": {"command": patch}}),
        );
        let env = build_event(HookKind::PostToolUse, &frame, &mgr, 42).unwrap();
        let tc: ToolCall = serde_json::from_value(env.payload).unwrap();
        assert!(tc.touched.is_empty());
    }

    #[test]
    fn codex_stop_reads_last_assistant_message_and_never_the_transcript() {
        let transcript = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            transcript.path(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"from transcript"}]}}"#,
        )
        .unwrap();
        let mgr = SessionManager::new();
        let prompt = codex_event(&mgr, HookKind::UserPromptSubmit, json!({"prompt": "go"}))
            .expect("UserPromptSubmit publishes");
        let stop = codex_event(
            &mgr,
            HookKind::Stop,
            json!({
                "turn_id": "turn-1",
                "stop_hook_active": false,
                "last_assistant_message": "Done: tests pass.",
                "transcript_path": transcript.path().to_string_lossy()
            }),
        )
        .expect("Stop publishes");
        let turn: Turn = serde_json::from_value(stop.payload.clone()).unwrap();
        assert_eq!(turn.user_message, "");
        assert_eq!(turn.assistant_message.as_deref(), Some("Done: tests pass."));
        assert_eq!(
            codex_extras(&prompt).correlation.turn_id,
            codex_extras(&stop).correlation.turn_id
        );
        cortex_core::validate_event(&serde_json::to_value(&stop).unwrap()).unwrap();

        // transcript_path null and no message: still a turn boundary.
        let stop = codex_event(
            &mgr,
            HookKind::Stop,
            json!({"stop_hook_active": true, "last_assistant_message": null}),
        )
        .expect("Stop publishes without a message");
        let turn: Turn = serde_json::from_value(stop.payload).unwrap();
        assert_eq!(turn.assistant_message, None);
    }

    #[test]
    fn codex_subagent_stop_publishes_agent_call_with_the_reply_as_result() {
        let mgr = SessionManager::new();
        let env = codex_event(
            &mgr,
            HookKind::SubagentStop,
            json!({
                "turn_id": "turn-2",
                "agent_id": "agent-7",
                "agent_type": "explorer",
                "agent_transcript_path": null,
                "stop_hook_active": false,
                "last_assistant_message": "Found 3 call sites."
            }),
        )
        .expect("SubagentStop publishes");
        assert_eq!(env.kind, Kind::AgentCall);
        assert_eq!(env.tool, TOOL_CODEX);
        let call: AgentCall = serde_json::from_value(env.payload.clone()).unwrap();
        assert_eq!(call.agent_type, "explorer");
        assert_eq!(call.description, "agent call");
        assert_eq!(
            call.result,
            Some(json!({"last_assistant_message": "Found 3 call sites."}))
        );
        assert_eq!(codex_extras(&env).agent_id.as_deref(), Some("agent-7"));
        cortex_core::validate_event(&serde_json::to_value(&env).unwrap()).unwrap();
    }
}
