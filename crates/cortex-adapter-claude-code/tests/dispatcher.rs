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
    // Phase14e — fail-open bundles now carry the
    // `<!-- cortex: timeout reason=… -->` sentinel so the model
    // can distinguish outage from "no context matched". The
    // additionalContext field surfaces it verbatim; the
    // permission decision is still untouched on UserPromptSubmit.
    let hook = resp
        .hook_specific_output
        .expect("fail-open carries hookSpecificOutput with sentinel");
    assert!(
        hook.additional_context
            .contains("<!-- cortex: timeout reason="),
        "additionalContext missing fail-open sentinel: {:?}",
        hook.additional_context
    );
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
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "violations": [
                {
                    "law_id": "LAW-007",
                    "severity": "critical",
                    "message": "no --no-verify"
                }
            ]
        })))
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
    // Mock a real-shaped QueryResponse with one active law and one
    // decision. The pre-thinking pipeline formats it to Markdown and
    // hands the string back as `hookSpecificOutput.additionalContext`.
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/query"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "intent": "pre_change_context",
            "query_id": "01HX0DEMOQUERY0000000001",
            "scope_resolved": { "repos": [], "topics": [] },
            "results": {
                "snippets": [],
                "decisions": [
                    {
                        "rank": 1,
                        "id": "DEC-0042",
                        "title": "Use Meilisearch for keyword lane",
                        "status": "accepted",
                        "ts": 1_777_000_000_000_i64,
                        "score": 0.91,
                        "links": []
                    }
                ],
                "violations": [],
                "graph_neighbors": [],
                "similar_turns": []
            },
            "laws_active": [
                {
                    "id": "LAW-007",
                    "severity": "critical",
                    "title": "No --no-verify on commits"
                }
            ],
            "budget": { "used_ms": 12, "cap_ms": 600, "cache": "miss" },
            "debug": { "lanes": {}, "errors": {} }
        })))
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
    let bundle = resp
        .hook_specific_output
        .expect("hookSpecificOutput present");
    assert_eq!(bundle.hook_event_name, "UserPromptSubmit");
    let md = &bundle.additional_context;
    assert!(md.contains("LAW-007"), "bundle missing law: {md}");
    assert!(md.contains("DEC-0042"), "bundle missing decision: {md}");
    assert!(
        md.starts_with("<!-- cortex: pre_change_context"),
        "bundle missing cortex header: {md}"
    );
}

#[tokio::test]
async fn malformed_hook_input_replies_empty_and_publishes_nothing() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    // Send something that's valid JSON but doesn't match HookFrame —
    // the dispatcher's serde layer drops it.
    let resp =
        cortex_adapter_claude_code::ipc::dispatch_inline(json!({ "garbage": true }), dispatcher)
            .await;
    assert!(resp.hook_specific_output.is_none());
    assert!(resp.permission_decision.is_none());
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn unknown_hook_kind_replies_empty() {
    let cfg = AdapterSection::default();
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(frame("ImaginaryHook", json!({ "k": "v" })), dispatcher).await;
    assert!(resp.hook_specific_output.is_none());
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

// ── SessionStart active-work surfacing (phase30 §1.3) ───────────────────────

#[tokio::test]
async fn session_start_surfaces_active_work_from_mock_api() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/dashboard/active-work"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "active_tasks": [
                {
                    "id": "phase30_continuity-loop-verification",
                    "phase": "phase30",
                    "status": "in-progress",
                    "next_unchecked_item": "1.3 Wire cortex_active_work into SessionStart"
                },
                {
                    "id": "phase29d_synap-wipe-equality-gap",
                    "phase": "phase29d",
                    "status": "pending",
                    "next_unchecked_item": "1.2 Cortex-side interim mitigation"
                }
            ],
            "in_progress_count": 1,
            "blocked_count": 0,
            "recent_archives": []
        })))
        .mount(&mock_server)
        .await;

    let cfg = AdapterSection {
        api_endpoint: mock_server.uri(),
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(frame("SessionStart", json!({})), dispatcher).await;
    let hook = resp
        .hook_specific_output
        .expect("SessionStart with active tasks must carry hookSpecificOutput");
    // Phase30 §1.3 — the reply must self-identify as SessionStart, not
    // borrow the UserPromptSubmit literal, or Claude Code won't route
    // the additionalContext to the right hook.
    assert_eq!(hook.hook_event_name, "SessionStart");
    let md = &hook.additional_context;
    assert!(
        md.contains("phase30_continuity-loop-verification"),
        "missing first task id: {md}"
    );
    assert!(
        md.contains("phase29d_synap-wipe-equality-gap"),
        "missing second task id: {md}"
    );
    assert!(
        md.contains("next: 1.3 Wire cortex_active_work into SessionStart"),
        "missing first next-fragment: {md}"
    );
    assert!(
        md.contains("next: 1.2 Cortex-side interim mitigation"),
        "missing second next-fragment: {md}"
    );
    // SessionStart stays sync-only (spec 04) — no canonical event.
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn session_start_fails_open_on_unreachable_api() {
    let cfg = AdapterSection {
        api_endpoint: "http://127.0.0.1:1".into(),
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(frame("SessionStart", json!({})), dispatcher).await;
    assert!(
        resp.hook_specific_output.is_none(),
        "fail-open must not carry hookSpecificOutput"
    );
    assert!(resp.permission_decision.is_none());
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn session_start_returns_empty_when_no_active_tasks() {
    let mock_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/dashboard/active-work"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "active_tasks": [],
            "in_progress_count": 0,
            "blocked_count": 0,
            "recent_archives": []
        })))
        .mount(&mock_server)
        .await;
    let cfg = AdapterSection {
        api_endpoint: mock_server.uri(),
        ..Default::default()
    };
    let (dispatcher, publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(frame("SessionStart", json!({})), dispatcher).await;
    assert!(
        resp.hook_specific_output.is_none(),
        "zero active_tasks must short-circuit to an empty response"
    );
    assert_eq!(publisher.count().await, 0);
}

#[tokio::test]
async fn session_start_caps_rendered_rows_at_eight() {
    let mock_server = wiremock::MockServer::start().await;
    // Zero-padded row ids so no id is a prefix of another — keeps the
    // presence/absence assertions below unambiguous.
    let tasks: Vec<Value> = (0..12)
        .map(|i| {
            json!({
                "id": format!("phase-row-{i:02}"),
                "status": "pending",
                "next_unchecked_item": format!("{i}.1 work item")
            })
        })
        .collect();
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/dashboard/active-work"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "active_tasks": tasks,
            "in_progress_count": 0,
            "blocked_count": 0,
            "recent_archives": []
        })))
        .mount(&mock_server)
        .await;
    let cfg = AdapterSection {
        api_endpoint: mock_server.uri(),
        ..Default::default()
    };
    let (dispatcher, _publisher) = build_dispatcher(cfg);
    let resp = dispatch_inline(frame("SessionStart", json!({})), dispatcher).await;
    let hook = resp
        .hook_specific_output
        .expect("hookSpecificOutput present");
    let md = &hook.additional_context;
    let rendered_rows = md.lines().filter(|l| l.starts_with("- phase-row-")).count();
    assert_eq!(
        rendered_rows, 8,
        "row cap must trim to 8 regardless of the daemon returning more: {md}"
    );
    for i in 0..8 {
        assert!(
            md.contains(&format!("phase-row-{i:02}")),
            "row {i} within the cap must render: {md}"
        );
    }
    for i in 8..12 {
        assert!(
            !md.contains(&format!("phase-row-{i:02}")),
            "row {i} beyond the cap must be trimmed: {md}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn hook_response_serializes_to_protocol_shape() {
    use cortex_adapter_claude_code::HookResponse;
    let empty = serde_json::to_value(HookResponse::empty()).unwrap();
    assert_eq!(empty.as_object().unwrap().len(), 0);

    let bundle = serde_json::to_value(HookResponse::additional_context(
        "## Active laws\n- LAW-007: blah\n".to_string(),
    ))
    .unwrap();
    assert_eq!(
        bundle["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert!(bundle["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("LAW-007"));
    assert!(
        bundle.get("additional_context").is_none(),
        "snake_case field must not be emitted"
    );

    let empty_bundle =
        serde_json::to_value(HookResponse::additional_context(String::new())).unwrap();
    assert_eq!(
        empty_bundle.as_object().unwrap().len(),
        0,
        "empty bundle must short-circuit to {{}}"
    );

    let deny = serde_json::to_value(HookResponse::deny("LAW-007 ...")).unwrap();
    assert_eq!(deny["permissionDecision"], "deny");
    assert_eq!(deny["permissionDecisionReason"], "LAW-007 ...");
    assert!(
        deny.get("permission_decision").is_none(),
        "snake_case field must not be emitted"
    );
}
