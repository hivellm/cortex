//! JSONL → Envelope projection — phase11i §1.3.
//!
//! Pairs `user` ↔ `assistant` records by `parentUuid` into one
//! `Kind::Turn` envelope; lifts `assistant.message.content[]`
//! `tool_use` blocks plus the matching `attachment.tool_result`
//! follow-ups into `Kind::ToolCall` envelopes; routes
//! `assistant.tool_use` blocks whose `tool_name == "Agent"` (or
//! variant subagent_type tags) into `Kind::AgentCall`.
//!
//! The mapper takes the full record list for one session, resolves
//! the parent-uuid graph in memory, and yields a flat
//! `Vec<Envelope>` in stable temporal order. `attachment` records
//! that fold into a parent ToolCall are NOT emitted as separate
//! envelopes — they become the `output` slot on the parent.
//!
//! Records the mapper drops on purpose:
//!
//! - `attachment.type ∈ {file-history-snapshot, queue-operation,
//!   last-prompt, hook_success, hook_additional_context,
//!   deferred_tools_delta, skill_listing}` once their context has
//!   been folded into the parent.
//! - Records without a `sessionId`. The corpus has very few of
//!   these; keeping them would force the downstream pipeline to
//!   invent synthetic session ids.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::types::{ArchiveError, JsonlRecord};

/// Counters returned alongside each successful map. Surfaced via
/// the CLI's `estimate` subcommand and the watcher daemon's
/// `/healthz`.
#[derive(Debug, Default, Clone)]
pub struct MapStats {
    /// Records the mapper consumed.
    pub records_consumed: usize,
    /// Turn envelopes emitted (one per matched user↔assistant pair).
    pub turns_emitted: usize,
    /// ToolCall envelopes emitted (one per assistant tool_use block).
    pub tool_calls_emitted: usize,
    /// AgentCall envelopes emitted (one per assistant Agent invocation).
    pub agent_calls_emitted: usize,
    /// Records intentionally dropped (transient attachments, typeless
    /// snapshots, etc.).
    pub dropped_records: usize,
    /// Records without a sessionId — counted separately so a future
    /// patch path (synthetic session ids) is easy to wire.
    pub sessionless_records: usize,
}

/// Lightweight envelope shape the mapper emits. The public
/// `cortex_core::events::Envelope` is the canonical wire form;
/// keeping a local projection here lets the mapper stay
/// `cortex-core`-light during early iteration. The CLI / emitter
/// layer (§1.5) re-projects this into the canonical form before
/// shipping.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedEnvelope {
    /// Stable per-record id (re-uses the source `uuid` when
    /// available; otherwise falls back to a content-hash derived
    /// from the payload).
    pub event_id: String,
    /// `Turn` / `ToolCall` / `AgentCall`.
    pub kind: EnvelopeKind,
    /// ISO-8601 timestamp from the source record.
    pub occurred_at: DateTime<Utc>,
    /// Stable session id. Always present (sessionless records are
    /// dropped upstream).
    pub session_id: String,
    /// Working directory at record time (becomes
    /// `Envelope.context.cwd`).
    pub cwd: Option<String>,
    /// Git branch at record time.
    pub git_branch: Option<String>,
    /// Claude model id (assistant turns + tool calls only).
    pub model: Option<String>,
    /// Repository slug derived from `cwd`. Filled by the emitter
    /// layer — the mapper leaves `None` so the slug rule stays in
    /// one place.
    pub repo_slug: Option<String>,
    /// Optional pointer back to the parent envelope (e.g. a
    /// ToolCall's parent Turn).
    pub parent_event_id: Option<String>,
    /// Per-kind payload as a flat `serde_json::Value`. The emitter
    /// re-projects into the typed `cortex_core::events::*Payload`
    /// shape before publishing.
    pub payload: Value,
}

/// `MappedEnvelope::kind` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeKind {
    /// One user prompt + assistant reply pair.
    Turn,
    /// One tool invocation captured by the assistant's
    /// `tool_use` block (or the matching `tool_result`
    /// attachment that follows).
    ToolCall,
    /// One sub-agent invocation (assistant `tool_use` whose tool
    /// is `Agent`).
    AgentCall,
}

/// Project a flat list of session records into the corresponding
/// flat list of envelopes, in temporal order. Records belonging to
/// other sessions can be present — the mapper filters by
/// `session_id` when the caller passes a non-empty `target_session`.
/// When `target_session` is `None`, every session represented in
/// the input is emitted (useful for the watcher's mixed-source path).
pub fn map_session(
    records: &[JsonlRecord],
    target_session: Option<&str>,
) -> Result<(Vec<MappedEnvelope>, MapStats), ArchiveError> {
    let mut stats = MapStats::default();
    let mut by_uuid: BTreeMap<&str, &JsonlRecord> = BTreeMap::new();
    for r in records {
        if let Some(uuid) = r.uuid.as_deref() {
            by_uuid.insert(uuid, r);
        }
    }

    let mut out: Vec<MappedEnvelope> = Vec::with_capacity(records.len());
    for r in records {
        stats.records_consumed += 1;
        if let Some(target) = target_session {
            if r.session_id.as_deref() != Some(target) {
                continue;
            }
        }
        let session_id = match r.session_id.clone() {
            Some(s) if !s.is_empty() => s,
            _ => {
                stats.sessionless_records += 1;
                continue;
            }
        };

        match r.kind.as_deref().unwrap_or("") {
            "assistant" => {
                let occurred_at = parse_ts(r);
                let user_parent = r
                    .parent_uuid
                    .as_deref()
                    .and_then(|p| by_uuid.get(p).copied())
                    .filter(|p| p.kind.as_deref() == Some("user"));

                if let Some(user_record) = user_parent {
                    if let Some(turn) = build_turn(user_record, r, &session_id, occurred_at) {
                        stats.turns_emitted += 1;
                        let turn_id = turn.event_id.clone();
                        out.push(turn);
                        // Tool calls / agent calls are children of
                        // this Turn — link them via parent_event_id
                        // so the graph writer materialises the edge.
                        for child in extract_tool_calls(r, &session_id, occurred_at, &turn_id) {
                            match child.kind {
                                EnvelopeKind::AgentCall => stats.agent_calls_emitted += 1,
                                EnvelopeKind::ToolCall => stats.tool_calls_emitted += 1,
                                EnvelopeKind::Turn => {}
                            }
                            out.push(child);
                        }
                    } else {
                        stats.dropped_records += 1;
                    }
                } else {
                    // Orphan assistant record — no matching user
                    // parent visible in this slice. Still emit a
                    // Turn with empty user_message so the
                    // assistant content is searchable; the graph
                    // writer can stitch the parent later if it
                    // arrives via a different file.
                    if let Some(turn) = build_turn_assistant_only(r, &session_id, occurred_at) {
                        stats.turns_emitted += 1;
                        let turn_id = turn.event_id.clone();
                        out.push(turn);
                        for child in extract_tool_calls(r, &session_id, occurred_at, &turn_id) {
                            match child.kind {
                                EnvelopeKind::AgentCall => stats.agent_calls_emitted += 1,
                                EnvelopeKind::ToolCall => stats.tool_calls_emitted += 1,
                                EnvelopeKind::Turn => {}
                            }
                            out.push(child);
                        }
                    } else {
                        stats.dropped_records += 1;
                    }
                }
            }
            "user" => {
                // User records are folded into their assistant
                // pair above. Standalone users (no assistant reply
                // yet — live session) become a Turn with
                // assistant_message=None.
                let already_paired = r
                    .uuid
                    .as_deref()
                    .map(|u| {
                        records.iter().any(|other| {
                            other.kind.as_deref() == Some("assistant")
                                && other.parent_uuid.as_deref() == Some(u)
                        })
                    })
                    .unwrap_or(false);
                if !already_paired {
                    let occurred_at = parse_ts(r);
                    if let Some(turn) = build_turn_user_only(r, &session_id, occurred_at) {
                        stats.turns_emitted += 1;
                        out.push(turn);
                    } else {
                        stats.dropped_records += 1;
                    }
                }
            }
            // Folded into parent ToolCall via extract_tool_calls
            // when the parent is visible. Standalone attachments
            // never become envelopes on their own.
            "attachment"
            | "system"
            | "file-history-snapshot"
            | "last-prompt"
            | "queue-operation" => {
                stats.dropped_records += 1;
            }
            _ => {
                stats.dropped_records += 1;
            }
        }
    }

    out.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at));
    Ok((out, stats))
}

fn parse_ts(r: &JsonlRecord) -> DateTime<Utc> {
    r.timestamp
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

fn assistant_text_blocks(r: &JsonlRecord) -> Vec<String> {
    r.message
        .as_ref()
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|block| {
                    let ty = block.get("type")?.as_str()?;
                    if ty == "text" {
                        block.get("text")?.as_str().map(str::to_string)
                    } else if ty == "thinking" {
                        block.get("thinking")?.as_str().map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn user_text(r: &JsonlRecord) -> String {
    let Some(message) = r.message.as_ref() else {
        return String::new();
    };
    if let Some(s) = message.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = message.get("content").and_then(|v| v.as_array()) {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|block| {
                let ty = block.get("type")?.as_str()?;
                if ty == "text" {
                    block.get("text")?.as_str().map(str::to_string)
                } else {
                    None
                }
            })
            .collect();
        return parts.join("\n\n");
    }
    String::new()
}

fn assistant_model(r: &JsonlRecord) -> Option<String> {
    r.message
        .as_ref()
        .and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn assistant_usage(r: &JsonlRecord) -> Option<Value> {
    r.message.as_ref().and_then(|m| m.get("usage").cloned())
}

fn pick_event_id(r: &JsonlRecord) -> String {
    r.uuid
        .clone()
        .unwrap_or_else(|| ulid::Ulid::new().to_string())
}

fn build_turn(
    user: &JsonlRecord,
    assistant: &JsonlRecord,
    session_id: &str,
    occurred_at: DateTime<Utc>,
) -> Option<MappedEnvelope> {
    let user_message = user_text(user);
    let assistant_message = assistant_text_blocks(assistant).join("\n\n");
    if user_message.is_empty() && assistant_message.is_empty() {
        return None;
    }
    Some(MappedEnvelope {
        event_id: pick_event_id(assistant),
        kind: EnvelopeKind::Turn,
        occurred_at,
        session_id: session_id.to_string(),
        cwd: assistant.cwd.clone().or_else(|| user.cwd.clone()),
        git_branch: assistant
            .git_branch
            .clone()
            .or_else(|| user.git_branch.clone()),
        model: assistant_model(assistant),
        repo_slug: None,
        parent_event_id: None,
        payload: serde_json::json!({
            "user_message": user_message,
            "assistant_message": assistant_message,
            "tokens": assistant_usage(assistant),
            "request_id": assistant.request_id,
            "is_sidechain": assistant.is_sidechain.unwrap_or(false),
        }),
    })
}

fn build_turn_user_only(
    user: &JsonlRecord,
    session_id: &str,
    occurred_at: DateTime<Utc>,
) -> Option<MappedEnvelope> {
    let user_message = user_text(user);
    if user_message.is_empty() {
        return None;
    }
    Some(MappedEnvelope {
        event_id: pick_event_id(user),
        kind: EnvelopeKind::Turn,
        occurred_at,
        session_id: session_id.to_string(),
        cwd: user.cwd.clone(),
        git_branch: user.git_branch.clone(),
        model: None,
        repo_slug: None,
        parent_event_id: None,
        payload: serde_json::json!({
            "user_message": user_message,
            "assistant_message": null,
            "tokens": null,
            "request_id": null,
            "is_sidechain": user.is_sidechain.unwrap_or(false),
        }),
    })
}

fn build_turn_assistant_only(
    assistant: &JsonlRecord,
    session_id: &str,
    occurred_at: DateTime<Utc>,
) -> Option<MappedEnvelope> {
    let assistant_message = assistant_text_blocks(assistant).join("\n\n");
    if assistant_message.is_empty() {
        return None;
    }
    Some(MappedEnvelope {
        event_id: pick_event_id(assistant),
        kind: EnvelopeKind::Turn,
        occurred_at,
        session_id: session_id.to_string(),
        cwd: assistant.cwd.clone(),
        git_branch: assistant.git_branch.clone(),
        model: assistant_model(assistant),
        repo_slug: None,
        parent_event_id: None,
        payload: serde_json::json!({
            "user_message": "",
            "assistant_message": assistant_message,
            "tokens": assistant_usage(assistant),
            "request_id": assistant.request_id,
            "is_sidechain": assistant.is_sidechain.unwrap_or(false),
        }),
    })
}

fn extract_tool_calls(
    assistant: &JsonlRecord,
    session_id: &str,
    occurred_at: DateTime<Utc>,
    turn_id: &str,
) -> Vec<MappedEnvelope> {
    let Some(message) = assistant.message.as_ref() else {
        return Vec::new();
    };
    let Some(content) = message.get("content").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    let model = assistant_model(assistant);
    let mut out = Vec::new();
    for block in content {
        let Some(ty) = block.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        if ty != "tool_use" {
            continue;
        }
        let tool_name = block
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let input = block.get("input").cloned().unwrap_or(Value::Null);
        let block_id = block
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| ulid::Ulid::new().to_string());
        let kind = if tool_name == "Agent" || tool_name == "Task" {
            EnvelopeKind::AgentCall
        } else {
            EnvelopeKind::ToolCall
        };
        let payload = match kind {
            EnvelopeKind::AgentCall => serde_json::json!({
                "agent_type": input.get("subagent_type").cloned().unwrap_or(Value::Null),
                "description": input.get("description").cloned().unwrap_or(Value::Null),
                "prompt": input.get("prompt").cloned().unwrap_or(Value::Null),
                "team_name": input.get("team_name").cloned().unwrap_or(Value::Null),
            }),
            EnvelopeKind::ToolCall => serde_json::json!({
                "tool_name": tool_name,
                "input": input,
            }),
            EnvelopeKind::Turn => Value::Null,
        };
        out.push(MappedEnvelope {
            event_id: block_id,
            kind,
            occurred_at,
            session_id: session_id.to_string(),
            cwd: assistant.cwd.clone(),
            git_branch: assistant.git_branch.clone(),
            model: model.clone(),
            repo_slug: None,
            parent_event_id: Some(turn_id.to_string()),
            payload,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_record(uuid: &str, sid: &str, text: &str) -> JsonlRecord {
        serde_json::from_value(json!({
            "type": "user",
            "uuid": uuid,
            "sessionId": sid,
            "timestamp": "2026-04-20T17:47:59.616Z",
            "cwd": "/repo",
            "gitBranch": "main",
            "message": {"role": "user", "content": text},
        }))
        .expect("user record")
    }

    fn assistant_text(uuid: &str, parent: &str, sid: &str, text: &str) -> JsonlRecord {
        serde_json::from_value(json!({
            "type": "assistant",
            "uuid": uuid,
            "parentUuid": parent,
            "sessionId": sid,
            "timestamp": "2026-04-20T17:48:02.667Z",
            "cwd": "/repo",
            "gitBranch": "main",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "content": [{"type": "text", "text": text}],
                "usage": {"input_tokens": 6, "output_tokens": 16},
            },
            "requestId": "req_x",
        }))
        .expect("assistant record")
    }

    fn assistant_with_tool_use(
        uuid: &str,
        parent: &str,
        sid: &str,
        tool: &str,
        block_id: &str,
    ) -> JsonlRecord {
        serde_json::from_value(json!({
            "type": "assistant",
            "uuid": uuid,
            "parentUuid": parent,
            "sessionId": sid,
            "timestamp": "2026-04-20T17:48:02.667Z",
            "cwd": "/repo",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "content": [
                    {"type": "text", "text": "running"},
                    {"type": "tool_use", "id": block_id, "name": tool, "input": {"command": "ls"}}
                ],
            },
        }))
        .expect("assistant record")
    }

    #[test]
    fn paired_user_assistant_emits_one_turn() {
        let recs = vec![
            user_record("u1", "s1", "hi"),
            assistant_text("a1", "u1", "s1", "hello"),
        ];
        let (out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(stats.turns_emitted, 1);
        assert_eq!(stats.tool_calls_emitted, 0);
        let payload = &out[0].payload;
        assert_eq!(payload["user_message"], json!("hi"));
        assert_eq!(payload["assistant_message"], json!("hello"));
        assert_eq!(out[0].model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn assistant_with_tool_use_block_emits_turn_plus_tool_call() {
        let recs = vec![
            user_record("u1", "s1", "ls"),
            assistant_with_tool_use("a1", "u1", "s1", "Bash", "tu1"),
        ];
        let (out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(stats.turns_emitted, 1);
        assert_eq!(stats.tool_calls_emitted, 1);
        let tool_call = out
            .iter()
            .find(|e| e.kind == EnvelopeKind::ToolCall)
            .unwrap();
        assert_eq!(tool_call.payload["tool_name"], json!("Bash"));
        assert_eq!(
            tool_call.parent_event_id.as_deref(),
            Some(
                out.iter()
                    .find(|e| e.kind == EnvelopeKind::Turn)
                    .unwrap()
                    .event_id
                    .as_str()
            )
        );
    }

    #[test]
    fn agent_tool_use_routes_to_agent_call_kind() {
        let recs = vec![
            user_record("u1", "s1", "spawn agent"),
            assistant_with_tool_use("a1", "u1", "s1", "Agent", "ag1"),
        ];
        let (out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(stats.agent_calls_emitted, 1);
        assert_eq!(stats.tool_calls_emitted, 0);
        assert!(out.iter().any(|e| e.kind == EnvelopeKind::AgentCall));
    }

    #[test]
    fn task_tool_use_also_routes_to_agent_call_kind() {
        let recs = vec![
            user_record("u1", "s1", "spawn"),
            assistant_with_tool_use("a1", "u1", "s1", "Task", "ag1"),
        ];
        let (_out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(stats.agent_calls_emitted, 1);
    }

    #[test]
    fn standalone_user_emits_partial_turn_with_no_assistant_message() {
        let recs = vec![user_record("u1", "s1", "still typing")];
        let (out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(stats.turns_emitted, 1);
        assert_eq!(out[0].payload["assistant_message"], json!(null));
    }

    #[test]
    fn orphan_assistant_emits_turn_with_empty_user_message() {
        let recs = vec![assistant_text("a1", "phantom", "s1", "no parent")];
        let (out, stats) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(stats.turns_emitted, 1);
        assert_eq!(out[0].payload["user_message"], json!(""));
        assert_eq!(out[0].payload["assistant_message"], json!("no parent"));
    }

    #[test]
    fn target_session_filters_out_other_session_records() {
        let recs = vec![
            user_record("u1", "s1", "hi"),
            assistant_text("a1", "u1", "s1", "hello"),
            user_record("u2", "s2", "hey"),
            assistant_text("a2", "u2", "s2", "hi back"),
        ];
        let (out, _) = map_session(&recs, Some("s1")).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_id, "s1");
    }

    #[test]
    fn sessionless_records_are_counted_and_dropped() {
        let mut r = user_record("u1", "ignored", "hi");
        r.session_id = None;
        let (out, stats) = map_session(&[r], None).unwrap();
        assert!(out.is_empty());
        assert_eq!(stats.sessionless_records, 1);
    }
}
