//! Integration tests for the dispatcher + sync paths + publisher flow
//! that drive spec-10's acceptance criteria.

use std::sync::Arc;

use cortex_adapter_claude_code::{
    dispatch_inline, AdapterSection, ClaudeCodeExtras, Dispatcher, MemoryPublisher, Metrics,
    SessionManager, SyncClient,
};
use cortex_core::events::{Kind, ToolCall};
use serde_json::{json, Value};

fn cc_extras(e: &cortex_core::events::Envelope) -> ClaudeCodeExtras {
    e.context
        .extras
        .get("claude_code")
        .cloned()
        .map(|v| serde_json::from_value(v).unwrap_or_default())
        .unwrap_or_default()
}

fn build_dispatcher(cfg: AdapterSection) -> (Arc<Dispatcher>, Arc<MemoryPublisher>) {
    let metrics = Arc::new(Metrics::new());
    let sessions = Arc::new(SessionManager::new());
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn cortex_adapter_claude_code::Publisher> = publisher.clone();
    let sync = Arc::new(SyncClient::new(&cfg, metrics.clone()));
    let dispatcher = Arc::new(Dispatcher::new(sessions, pub_dyn, sync, 12345));
    (dispatcher, publisher)
}

fn frame(hook: &str, payload: Value) -> Value {
    json!({
        "hook": hook,
        "session_id": "test-session",
        "cwd": "/repos/Vectorizer",
        "payload": payload,
    })
}

#[tokio::test]
async fn user_prompt_returns_empty_additional_context_on_unreachable_api() {
    // Point at a port that never answers; the 100 ms timeout fires
    // and the dispatcher returns the empty fail-open shape.
    let cfg = AdapterSection {
        api_endpoint: "http://127.0.0.1:1".into(),
        pre_thinking: cortex_adapter_claude_code::PreThinkingSection {
            timeout_ms: 100,
            ..Default::default()
        },
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(
        frame("UserPromptSubmit", json!({ "prompt": "hi" })),
        dispatcher,
    )
    .await;
    // Empty fail-open ⇒ {} reply ⇒ no additionalContext field.
    assert!(resp.additional_context.is_none());
    assert!(resp.permission_decision.is_none());
    // UserPromptSubmit is publishable as a canonical `turn` event.
    assert_eq!(publisher.count().await, 1);
    let snap = publisher.snapshot().await;
    assert_eq!(snap[0].kind, Kind::Turn);
}

#[tokio::test]
async fn pre_tool_use_allows_when_law_check_unreachable() {
    let cfg = AdapterSection {
        api_endpoint: "http://127.0.0.1:1".into(),
        laws: cortex_adapter_claude_code::LawsSection {
            timeout_ms: 50,
            ..Default::default()
        },
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(
        frame(
            "PreToolUse",
            json!({
                "tool_name": "Bash",
                "tool_use_id": "tu-1",
                "input": { "command": "echo ok" }
            }),
        ),
        dispatcher,
    )
    .await;
    // Fail-open: no permission_decision means Claude Code proceeds.
    // PreToolUse is sync-only — no canonical event published.
    assert!(resp.permission_decision.is_none());
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn pre_tool_use_denies_on_critical_violation_from_mock_api() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/laws/check"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "violations": [
                    {
                        "law_id": "LAW-007",
                        "severity": "critical",
                        "message": "no --no-verify"
                    }
                ]
            })),
        )
        .mount(&mock_server)
        .await;

    let cfg = AdapterSection {
        api_endpoint: mock_server.uri(),
        laws: cortex_adapter_claude_code::LawsSection {
            timeout_ms: 1000,
            ..Default::default()
        },
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(
        frame(
            "PreToolUse",
            json!({
                "tool_name": "Bash",
                "tool_use_id": "tu-bad",
                "input": { "command": "git commit --no-verify" }
            }),
        ),
        dispatcher,
    )
    .await;
    assert_eq!(resp.permission_decision.as_deref(), Some("deny"));
    let reason = resp.permission_decision_reason.unwrap_or_default();
    assert!(reason.contains("LAW-007"));
    assert!(reason.contains("no --no-verify"));
    // PreToolUse is sync-only; the deny is recorded by the law-check
    // path, not by an envelope event.
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn user_prompt_returns_bundle_when_api_responds() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/query"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "decisions": ["DEC-0042"],
                "active_laws": ["LAW-007"]
            })),
        )
        .mount(&mock_server)
        .await;

    let cfg = AdapterSection {
        api_endpoint: mock_server.uri(),
        pre_thinking: cortex_adapter_claude_code::PreThinkingSection {
            timeout_ms: 1000,
            ..Default::default()
        },
        ..Default::default()
    };
    let (dispatcher, _publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(
        frame("UserPromptSubmit", json!({ "prompt": "fix the bug" })),
        dispatcher,
    )
    .await;
    let bundle = resp.additional_context.expect("bundle present");
    assert_eq!(bundle["decisions"][0], "DEC-0042");
    assert_eq!(bundle["active_laws"][0], "LAW-007");
}

#[tokio::test]
async fn malformed_hook_input_replies_empty_and_publishes_nothing() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    // Send something that's valid JSON but doesn't match HookFrame —
    // the dispatcher's serde layer drops it.
    let resp = cortex_adapter_claude_code::ipc::dispatch_inline(
        json!({ "garbage": true }),
        dispatcher,
    )
    .await;
    assert!(resp.additional_context.is_none());
    assert!(resp.permission_decision.is_none());
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn unknown_hook_kind_replies_empty() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(
        frame("ImaginaryHook", json!({ "k": "v" })),
        dispatcher,
    )
    .await;
    assert!(resp.additional_context.is_none());
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn session_correlation_pairs_pre_and_post_tool_events() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    dispatch_inline(
        frame("UserPromptSubmit", json!({ "prompt": "edit" })),
        dispatcher.clone(),
    )
    .await;
    dispatch_inline(
        frame(
            "PreToolUse",
            json!({ "tool_name": "Edit", "tool_use_id": "tu-7", "input": {} }),
        ),
        dispatcher.clone(),
    )
    .await;
    dispatch_inline(
        frame(
            "PostToolUse",
            json!({ "tool_name": "Edit", "tool_use_id": "tu-7", "output": {} }),
        ),
        dispatcher,
    )
    .await;
    let snap = publisher.snapshot().await;
    // Only Turn (UserPromptSubmit) and ToolCall (PostToolUse) publish.
    // PreToolUse is sync-only and does not appear here.
    let turn = snap.iter().find(|e| e.kind == Kind::Turn).unwrap();
    let tool_call = snap.iter().find(|e| e.kind == Kind::ToolCall).unwrap();
    assert_eq!(turn.session_id, tool_call.session_id);
    let tc_extras = cc_extras(tool_call);
    let turn_extras = cc_extras(turn);
    assert_eq!(turn_extras.turn_id, tc_extras.turn_id);
    assert!(tc_extras.tool_call_id.is_some());
    assert_eq!(tc_extras.tool_use_id.as_deref(), Some("tu-7"));
    assert!(!tc_extras.orphan);
}

#[tokio::test]
async fn redaction_strips_secrets_before_publish() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    // Open a turn first so the PreToolUse correlation isn't orphaned.
    dispatch_inline(
        frame("UserPromptSubmit", json!({ "prompt": "go" })),
        dispatcher.clone(),
    )
    .await;
    // PreToolUse opens the tool_call slot but doesn't publish.
    dispatch_inline(
        frame(
            "PreToolUse",
            json!({
                "tool_name": "Bash",
                "tool_use_id": "tu-secret",
                "input": { "command": "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000" }
            }),
        ),
        dispatcher.clone(),
    )
    .await;
    // PostToolUse carries the canonical record — also redacted.
    dispatch_inline(
        frame(
            "PostToolUse",
            json!({
                "tool_name": "Bash",
                "tool_use_id": "tu-secret",
                "input": { "command": "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE0000" },
                "output": { "stdout": "", "exit_code": 0 }
            }),
        ),
        dispatcher,
    )
    .await;
    let snap = publisher.snapshot().await;
    let evt = snap.iter().find(|e| e.kind == Kind::ToolCall).unwrap();
    let tc: ToolCall = serde_json::from_value(evt.payload.clone()).unwrap();
    let body = tc.input["command"].as_str().unwrap_or_default();
    assert!(!body.contains("AKIAIOSFODNN7EXAMPLE0000"));
    assert!(!evt.redactions.is_empty());
}

#[tokio::test]
async fn hook_response_serializes_to_protocol_shape() {
    use cortex_adapter_claude_code::HookResponse;
    let empty = serde_json::to_value(HookResponse::empty()).unwrap();
    assert_eq!(empty.as_object().unwrap().len(), 0);

    let bundle = serde_json::to_value(HookResponse::additional_context(json!({ "k": 1 }))).unwrap();
    assert_eq!(bundle["additional_context"]["k"], 1);

    let deny = serde_json::to_value(HookResponse::deny("LAW-007 ...")).unwrap();
    assert_eq!(deny["permission_decision"], "deny");
    assert_eq!(deny["permission_decision_reason"], "LAW-007 ...");
}
