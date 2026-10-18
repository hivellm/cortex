use cortex_local::{hooks, mcp, Authority, LocalStore};
use serde_json::{json, Value};

fn capture(store: &LocalStore, repository: &str) -> Value {
    mcp::call(store,"cortex_capture_memory",json!({"repository":repository,"title":"Build rule","body":"Run cargo check before commit","source":"test:decision","authority":"accepted"}),None).unwrap()
}

#[test]
fn dispatch_scope_and_aliases_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    capture(&store, "Cortex");
    assert!(mcp::call(&store, "cortex_status", json!({}), None).is_err());
    assert!(mcp::call(
        &store,
        "cortex_status",
        json!({"repo":"Cortex","repository":"Other"}),
        None
    )
    .is_err());
    assert!(mcp::call(
        &store,
        "cortex_status",
        json!({"repo":"Cortex","unknown":true}),
        None
    )
    .is_err());
    let result = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repository":"cortex"}),
        None,
    )
    .unwrap();
    assert!(!result["context"].as_str().unwrap().contains("Build rule"));
    let result = mcp::call(
        &store,
        "cortex_pre_thinking",
        json!({"scope":{"repo":"Cortex"}}),
        None,
    )
    .unwrap();
    let context = result["context"].as_str().unwrap();
    assert!(
        context.contains("Build rule")
            && context.contains("source=test:decision")
            && context.contains("authority=Accepted")
    );
    assert_eq!(result["delivery_consumption"], "not_observed");
    assert!(mcp::call(&store,"cortex_capture_memory",json!({"repo":"Cortex","title":"x","body":"x","source":"x","kind":"lesson","category":"decision"}),None).is_err());
    assert!(mcp::call(
        &store,
        "cortex_capture_memory",
        json!({"repo":"Cortex","title":"x","body":"x","source":" "}),
        None
    )
    .is_err());
    assert!(mcp::call(
        &store,
        "cortex_pre_thinking",
        json!({"repo":"Cortex","task":"Build","prompt":"Other"}),
        None
    )
    .is_err());
    assert!(mcp::call(
        &store,
        "cortex_pre_thinking",
        json!({"repo":"Cortex","task":"Build"}),
        None
    )
    .unwrap()["context"]
        .as_str()
        .unwrap()
        .contains("Build rule"));
}

#[test]
fn json_rpc_recovers_and_notifications_never_mutate() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let mut input = String::from("bad json\n");
    input.push_str(&json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"cortex_capture_memory","arguments":{"title":"x","body":"x","source":"x"}}}).to_string());
    input.push('\n');
    input.push_str(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}).to_string());
    input.push('\n');
    input.push_str(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string());
    input.push('\n');
    let mut output = Vec::new();
    mcp::run_io(&store, Some("Cortex"), input.as_bytes(), &mut output).unwrap();
    let responses: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert!(responses[2]["result"]["tools"].as_array().unwrap().len() >= 10);
    assert!(store.recent("Cortex", 100, true).unwrap().is_empty());
    let mut oversized = vec![b'x'; 4 * 1024 * 1024 + 20];
    oversized.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n");
    let mut output = Vec::new();
    mcp::run_io(&store, None, oversized.as_slice(), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().count(), 2);
}

#[test]
fn hooks_preserve_observed_identity_redact_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("cortex.toml"), "[cortex]\nid = 'Cortex'\n").unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    capture(&store, "Cortex");
    for client in ["claude-code", "codex", "opencode"] {
        let input = json!({"cwd":repo.path(),"session_id":"observed","turn_id":"native-turn","prompt":"token=secret-value sk-proj-123456789012345678901234","transcript_path":null});
        let result = hooks::run(root.path(), client, Some("UserPromptSubmit"), input.clone());
        assert!(result["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Build rule"));
        hooks::run(root.path(), client, Some("UserPromptSubmit"), input);
        hooks::run(
            root.path(),
            client,
            Some("UserPromptSubmit"),
            json!({"cwd":repo.path(),"session_id":"observed","turn_id":"native-turn","prompt":"changed replay"}),
        );
    }
    let records = store.recent("Cortex", 100, true).unwrap();
    assert_eq!(records.len(), 4);
    for record in records.iter().filter(|r| r.source.kind == "lifecycle_hook") {
        assert_eq!(record.authority, Authority::Observation);
        assert_eq!(record.metadata["session_id"], "observed");
        assert_eq!(record.metadata["turn_id"], "native-turn");
        let raw = serde_json::to_string(record).unwrap();
        assert!(!raw.contains("secret-value") && !raw.contains("sk-proj-123"));
    }
    let input = json!({"cwd":repo.path(),"tool_name":"apply_patch","tool_input":{"command":"*** Add File: a.rs\n*** Update File: b.rs\n*** Delete File: c.rs","password":"nested-secret"},"tool_response":{"ok":true,"api_key":"ordinary-secret","nested":{"authorization":"private-auth"}}});
    assert_eq!(
        hooks::run(root.path(), "codex", Some("PostToolUse"), input),
        json!({})
    );
    let records = store.recent("Cortex", 100, true).unwrap();
    let event = records
        .iter()
        .find(|r| r.metadata["event"] == "PostToolUse")
        .unwrap();
    assert_eq!(event.source.payload["tool_response"]["ok"], true);
    let persisted = serde_json::to_string(event).unwrap();
    for secret in ["nested-secret", "ordinary-secret", "private-auth"] {
        assert!(!persisted.contains(secret));
    }
    assert_eq!(event.metadata["touched_paths"].as_array().unwrap().len(), 3);
    for name in ["Stop", "SubagentStop"] {
        hooks::run(
            root.path(),
            "codex",
            Some(name),
            json!({"cwd":repo.path(),"agent_id":"real-agent","last_assistant_message":"done"}),
        );
    }
    assert_eq!(store.recent("Cortex", 100, true).unwrap().len(), 7);
    let repeated = json!({"cwd":repo.path(),"session_id":"session-only","prompt":"same prompt"});
    for _ in 0..2 {
        hooks::run(
            root.path(),
            "codex",
            Some("UserPromptSubmit"),
            repeated.clone(),
        );
    }
    assert_eq!(store.recent("Cortex", 100, true).unwrap().len(), 9);
}

#[test]
fn hooks_fail_open_without_creating_store() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    assert_eq!(
        hooks::run(
            &missing,
            "codex",
            Some("SessionStart"),
            json!({"cwd":root.path()})
        ),
        json!({})
    );
    assert!(!missing.exists());
}

#[test]
fn recall_budget_keeps_utf8_lines_and_citations() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let body = "memória comprovada\n".repeat(700);
    mcp::call(
        &store,
        "cortex_capture_memory",
        json!({"repo":"Cortex","title":"Evidence","body":body,"source":"test:source"}),
        None,
    )
    .unwrap();
    let result = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repo":"Cortex","budget_bytes":6144}),
        None,
    )
    .unwrap();
    let context = result["context"].as_str().unwrap();
    assert!(context.len() <= 6144);
    assert!(context.contains("authority=Proposed"));
    assert!(context.ends_with("<!-- cortex: memory clipped; cortex_memory_read returns more -->\n"));
}

#[test]
fn memory_transports_ignore_event_flood_before_limiting() {
    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("cortex.toml"), "[cortex]\nid='Cortex'\n").unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    capture(&store, "Cortex");
    for number in 0..15 {
        store
            .capture(cortex_local::CaptureRequest {
                repository: "Cortex".into(),
                kind: cortex_local::RecordKind::Event,
                title: format!("Build event {number}"),
                body: "Build noise".into(),
                ..Default::default()
            })
            .unwrap();
    }
    let context = mcp::call(
        &store,
        "cortex_memory_read",
        json!({"repo":"Cortex","query":"Build","limit":1}),
        None,
    )
    .unwrap();
    assert!(context["context"].as_str().unwrap().contains("Build rule"));
    assert!(!context["context"].as_str().unwrap().contains("Build event"));
    let context = hooks::run(
        root.path(),
        "codex",
        Some("SessionStart"),
        json!({"cwd":repo.path()}),
    );
    assert!(context["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("Build rule"));
}
