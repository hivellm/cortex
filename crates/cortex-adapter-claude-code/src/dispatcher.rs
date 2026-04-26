//! Hook dispatcher — the daemon's single per-frame entry point.
//!
//! Spec 10 §Hook ↔ daemon protocol: the daemon receives a `HookFrame`,
//! decides whether the hook needs a synchronous call, builds the
//! envelope, hands it to the async publisher, and replies in the
//! protocol-defined JSON shape. Any internal error short-circuits to
//! an empty `{}` reply so the session never breaks.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::{build_event, ClaudeEvent, HookFrame, HookKind};
use crate::publisher::Publisher;
use crate::session::SessionManager;
use crate::sync_paths::SyncClient;

/// Protocol response printed to stdout by the hook shim.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookResponse {
    /// `additionalContext` bundle (UserPromptSubmit only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<Value>,
    /// `permissionDecision` (PreToolUse only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    /// Reason printed alongside a deny.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,
}

impl HookResponse {
    /// `{}` — Claude Code proceeds unmodified.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a UserPromptSubmit response.
    pub fn additional_context(value: Value) -> Self {
        Self {
            additional_context: Some(value),
            permission_decision: None,
            permission_decision_reason: None,
        }
    }

    /// Build a PreToolUse `deny` response.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            additional_context: None,
            permission_decision: Some("deny".into()),
            permission_decision_reason: Some(reason.into()),
        }
    }
}

/// Daemon dispatch context — owns the session manager, sync client,
/// publisher, and the daemon pid for fallback session synthesis.
pub struct Dispatcher {
    sessions: Arc<SessionManager>,
    publisher: Arc<dyn Publisher>,
    sync: Arc<SyncClient>,
    pid: u32,
}

impl Dispatcher {
    /// Build a new dispatcher.
    pub fn new(
        sessions: Arc<SessionManager>,
        publisher: Arc<dyn Publisher>,
        sync: Arc<SyncClient>,
        pid: u32,
    ) -> Self {
        Self {
            sessions,
            publisher,
            sync,
            pid,
        }
    }

    /// Handle one hook frame end-to-end. Always returns a response —
    /// internal errors degrade to [`HookResponse::empty`].
    pub async fn dispatch(&self, frame: HookFrame) -> HookResponse {
        let kind = match HookKind::parse(&frame.hook) {
            Some(k) => k,
            None => {
                tracing::warn!(hook = %frame.hook, "unknown hook kind");
                return HookResponse::empty();
            }
        };

        let event = build_event(kind, &frame, &self.sessions, self.pid);
        let response = self.maybe_sync_path(kind, &frame, &event).await;
        // Async publish always happens, regardless of sync outcome.
        self.publisher.publish(event).await;
        response
    }

    async fn maybe_sync_path(
        &self,
        kind: HookKind,
        frame: &HookFrame,
        event: &ClaudeEvent,
    ) -> HookResponse {
        match kind {
            HookKind::UserPromptSubmit => {
                let prompt = read_string_field(&frame.payload, "prompt")
                    .or_else(|| read_string_field(&frame.payload, "user_message"))
                    .unwrap_or_default();
                let result = self
                    .sync
                    .pre_thinking(&prompt, &event.session_id, frame.cwd.as_deref())
                    .await;
                if result.fail_open
                    && result
                        .additional_context
                        .as_object()
                        .map(|m| m.is_empty())
                        .unwrap_or(false)
                {
                    return HookResponse::empty();
                }
                HookResponse::additional_context(result.additional_context)
            }
            HookKind::PreToolUse => {
                let tool_name =
                    read_string_field(&frame.payload, "tool_name").unwrap_or_default();
                let input = frame.payload.get("input").cloned().unwrap_or(json!({}));
                let result = self
                    .sync
                    .law_check(
                        &tool_name,
                        &input,
                        &event.session_id,
                        event.turn_id.as_deref(),
                    )
                    .await;
                if result.deny {
                    return HookResponse::deny(result.reason);
                }
                HookResponse::empty()
            }
            _ => HookResponse::empty(),
        }
    }
}

fn read_string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}
