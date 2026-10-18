//! Fail-open local lifecycle adapters. No provider, daemon or transcript access.
use crate::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub fn run(root: &Path, client: &str, event: Option<&str>, input: Value) -> Value {
    attempt(root, client, event, input).unwrap_or_else(|_| json!({}))
}

fn attempt(root: &Path, client: &str, event: Option<&str>, mut input: Value) -> Result<Value> {
    let invalid = || LocalError::Invalid("unsupported hook input".into());
    if !["claude-code", "codex", "opencode"].contains(&client) || !input.is_object() {
        return Err(invalid());
    }
    let event = event
        .or_else(|| input["hook_event_name"].as_str())
        .ok_or_else(invalid)?
        .to_owned();
    if ![
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "SubagentStop",
    ]
    .contains(&event.as_str())
    {
        return Err(invalid());
    }
    let cwd = input["cwd"].as_str().ok_or_else(invalid)?;
    if !Path::new(cwd).is_absolute() {
        return Err(invalid());
    }
    let repository = crate::runtime::resolve_repository(Path::new(cwd), None)?;
    let store = LocalStore::open_for_hook(root, Duration::from_millis(150))?;
    redact(&mut input)?;
    let mut response = json!({});
    if event == "SessionStart" || event == "UserPromptSubmit" {
        let mut records = Vec::new();
        if event == "UserPromptSubmit" {
            let query = input["prompt"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .take(24)
                .collect::<Vec<_>>()
                .join(" ");
            if !query.is_empty() && query.len() <= 4096 {
                records = store
                    .search_memories(SearchQuery {
                        repository: repository.clone(),
                        text: query,
                        limit: 10,
                        include_inactive: false,
                    })?
                    .into_iter()
                    .map(|hit| hit.record)
                    .filter(|record| record.kind != RecordKind::Event)
                    .collect();
            }
        }
        for record in store.recent_memories(&repository, 10, false)? {
            if !records.iter().any(|old| old.id == record.id) && records.len() < 10 {
                records.push(record);
            }
        }
        if !records.is_empty() {
            response = json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":crate::mcp::context(&records,6144)}});
        }
    }
    if event != "SessionStart" {
        let body = match event.as_str() {
            "UserPromptSubmit" => input.get("prompt").cloned(),
            "PostToolUse" => input.get("tool_response").cloned(),
            _ => input.get("last_assistant_message").cloned(),
        }
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default();
        let mut metadata = json!({"client":client,"event":event});
        for key in ["session_id", "turn_id", "agent_id", "agent_type"] {
            if let Some(value) = input.get(key).filter(|value| value.is_string()) {
                metadata[key] = value.clone();
            }
        }
        if event == "PostToolUse"
            && input["tool_name"]
                .as_str()
                .is_some_and(|name| name.ends_with("apply_patch"))
        {
            let patch = input["tool_input"]
                .as_str()
                .or_else(|| input["tool_input"]["patch"].as_str())
                .or_else(|| input["tool_input"]["input"].as_str())
                .or_else(|| input["tool_input"]["command"].as_str())
                .unwrap_or("");
            let mut touched = std::collections::BTreeSet::new();
            for line in patch.lines() {
                for (prefix, operation) in [
                    ("*** Add File: ", "create"),
                    ("*** Update File: ", "write"),
                    ("*** Delete File: ", "delete"),
                    ("*** Move to: ", "create"),
                ] {
                    if let Some(path) = line.strip_prefix(prefix) {
                        touched.insert((path.to_owned(), operation));
                    }
                }
            }
            metadata["touched_paths"] = json!(touched
                .into_iter()
                .map(|(path, operation)| json!({"path":path,"operation":operation}))
                .collect::<Vec<_>>());
        }
        let native_id = input["event_id"]
            .as_str()
            .or_else(|| {
                if event == "PostToolUse" {
                    input["tool_use_id"].as_str()
                } else if event == "SubagentStop" {
                    None
                } else {
                    input["turn_id"].as_str()
                }
            })
            .filter(|id| !id.is_empty());
        // A content hash alone would merge distinct identical events without native identity.
        let id = if native_id.is_some()
            && input["session_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        {
            let identity = serde_json::to_vec(&json!([
                client,
                event,
                repository,
                input["session_id"],
                native_id
            ]))?;
            format!("hook-{:x}", Sha256::digest(&identity))
        } else {
            format!("hook-{}", ulid::Ulid::new())
        };
        store.capture(CaptureRequest {
            id: Some(id.clone()),
            repository,
            kind: RecordKind::Event,
            authority: Authority::Observation,
            title: format!("{client} {event}"),
            body,
            source: Source {
                kind: "lifecycle_hook".into(),
                locator: format!("{client}:{event}"),
                source_id: Some(id),
                payload: input,
                ..Default::default()
            },
            metadata,
            ..Default::default()
        })?;
    }
    Ok(response)
}

fn redact(value: &mut Value) -> Result<()> {
    cortex_core::redact::redact(value);
    let patterns = [
        r#"(?i)(?:api[_-]?key|password|passwd|secret|token|access[_-]?token|refresh[_-]?token)["']?\s*[=:]\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;}]+)"#,
        r"\b(?:sk-(?:proj|svcacct)-[A-Za-z0-9_-]{16,}|github_pat_[A-Za-z0-9_]{20,})\b",
        r"(?i)\bBasic\s+[A-Za-z0-9+/]+={0,2}",
        r"(?i)\b[a-z][a-z0-9+.-]*://[^\s/@]+:[^\s/@]+@",
    ].into_iter().map(regex::Regex::new).collect::<std::result::Result<Vec<_>, _>>().map_err(|error| LocalError::Invalid(error.to_string()))?;
    fn visit(value: &mut Value, patterns: &[regex::Regex]) {
        match value {
            Value::String(text) => {
                for pattern in patterns {
                    *text = pattern
                        .replace_all(text, "[REDACTED:credential]")
                        .into_owned();
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, patterns);
                }
            }
            Value::Object(values) => {
                for (key, value) in values.iter_mut() {
                    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
                    if [
                        "password",
                        "passwd",
                        "secret",
                        "token",
                        "apikey",
                        "accesstoken",
                        "refreshtoken",
                        "authorization",
                        "cookie",
                        "setcookie",
                        "clientsecret",
                        "privatekey",
                    ]
                    .contains(&key.as_str())
                    {
                        *value = json!("[REDACTED:credential]");
                    } else {
                        visit(value, patterns);
                    }
                }
            }
            _ => {}
        }
    }
    visit(value, &patterns);
    Ok(())
}
