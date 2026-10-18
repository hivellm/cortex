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
    let identity_fields = [
        "session_id",
        "turn_id",
        "event_id",
        "tool_use_id",
        "agent_id",
    ];
    let observed_ids: Vec<_> = identity_fields
        .iter()
        .map(|key| (*key, input.get(*key).cloned()))
        .collect();
    redact(&mut input)?;
    // Redaction placeholders cannot become native identity or join unrelated turns.
    let mut actor_unverifiable = false;
    for (key, original) in observed_ids {
        let valid = input.get(key).and_then(Value::as_str).is_some_and(|value| {
            original.as_ref() == input.get(key)
                && !value.contains("[REDACTED:")
                && crate::audit::bounded_id(value).is_ok()
        });
        if !valid {
            actor_unverifiable |=
                key == "agent_id" && original.as_ref().is_some_and(|value| !value.is_null());
            input.as_object_mut().ok_or_else(invalid)?.remove(key);
        }
    }
    if actor_unverifiable {
        // An invalid child identity must not become the parent's absent agent ID.
        input.as_object_mut().ok_or_else(invalid)?.remove("turn_id");
    }
    let mut response = json!({});
    if event == "SessionStart" || event == "UserPromptSubmit" {
        let mut query = if event == "UserPromptSubmit" {
            input["prompt"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .take(24)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        };
        if query.len() > 4096 {
            query.clear();
        }
        let origin = match (client, event.as_str()) {
            ("claude-code", "SessionStart") => AuditOrigin::ClaudeSessionStart,
            ("claude-code", _) => AuditOrigin::ClaudeUserPromptSubmit,
            ("codex", "SessionStart") => AuditOrigin::CodexSessionStart,
            ("codex", _) => AuditOrigin::CodexUserPromptSubmit,
            ("opencode", "SessionStart") => AuditOrigin::OpenCodeSessionStart,
            _ => AuditOrigin::OpenCodeUserPromptSubmit,
        };
        let prepared = crate::recall::prepare(
            &store,
            crate::recall::RecallRequest {
                repository: repository.clone(),
                query,
                limit: 10,
                budget_bytes: 6144,
                origin,
                attribution: observed_attribution(client, &input),
                fallback_recent: true,
            },
        )?;
        if prepared.selected_count > 0 && !prepared.context.is_empty() {
            response = json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":prepared.context}});
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
        for key in [
            "session_id",
            "turn_id",
            "agent_id",
            "agent_type",
            "event_id",
        ] {
            if let Some(value) = input.get(key).filter(|value| value.is_string()) {
                metadata[key] = value.clone();
            }
        }
        if metadata.get("event_id").is_none() {
            if let Some(id) = input.get("tool_use_id").filter(|value| value.is_string()) {
                metadata["event_id"] = id.clone();
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
            let mut identity = json!([client, event, repository, input["session_id"], native_id]);
            if event == "Stop" && input.get("event_id").is_none() {
                // A blocked stop may run again in the same turn with a new answer.
                identity.as_array_mut().unwrap().extend([
                    input["agent_id"].clone(),
                    json!(format!("{:x}", Sha256::digest(body.as_bytes()))),
                ]);
            }
            let identity = serde_json::to_vec(&identity)?;
            format!("hook-{:x}", Sha256::digest(&identity))
        } else {
            format!("hook-{}", ulid::Ulid::new())
        };
        let capture = store.capture(CaptureRequest {
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
        });
        // A later event-capture failure cannot retract already prepared context.
        if event != "UserPromptSubmit" {
            let captured = capture?;
            store.observe_event(&captured.record)?;
        }
    }
    Ok(response)
}

fn observed_attribution(client: &str, input: &Value) -> AuditAttribution {
    fn id(input: &Value, key: &str) -> Option<String> {
        input[key]
            .as_str()
            .filter(|value| crate::audit::bounded_id(value).is_ok())
            .map(str::to_owned)
    }
    AuditAttribution {
        client: Some(client.into()),
        session_id: id(input, "session_id"),
        turn_id: id(input, "turn_id"),
        event_id: id(input, "event_id").or_else(|| id(input, "tool_use_id")),
        agent_id: id(input, "agent_id"),
    }
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
