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

use crate::events::{build_event, HookFrame, HookKind};
use crate::metrics::Metrics;
use crate::publisher::Publisher;
use crate::session::SessionManager;
use crate::sync_paths::SyncClient;

/// `hookSpecificOutput` payload for `UserPromptSubmit`. Claude Code
/// looks up `additionalContext` (a string) under this object and
/// injects it as a `<additional-context>` block in the model's
/// system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    /// Echoed event name. Claude Code uses this to disambiguate the
    /// payload when multiple hook kinds share a struct.
    pub hook_event_name: String,
    /// Markdown bundle to inject into the model context. Empty
    /// string is allowed; Claude Code drops empty bundles silently.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub additional_context: String,
}

/// Protocol response printed to stdout by the hook shim. Field
/// names follow Claude Code's hook contract verbatim
/// (`hookSpecificOutput`, `permissionDecision`,
/// `permissionDecisionReason`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HookResponse {
    /// `hookSpecificOutput.additionalContext` (UserPromptSubmit only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
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

    /// Build a UserPromptSubmit response from a Markdown bundle. An
    /// empty bundle short-circuits to [`HookResponse::empty`] so we
    /// never emit a `hookSpecificOutput` block with no content.
    pub fn additional_context(bundle: String) -> Self {
        if bundle.is_empty() {
            return Self::empty();
        }
        Self {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "UserPromptSubmit".into(),
                additional_context: bundle,
            }),
            permission_decision: None,
            permission_decision_reason: None,
        }
    }

    /// Build a PreToolUse `deny` response.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: None,
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
    /// Phase8b — optional metrics registry. The dispatcher stamps
    /// `frames_parsed_total{hook}` + `envelopes_built_total{kind}` +
    /// `last_frame_ts{hook}` + `last_envelope_ts{kind}` here so the
    /// freshness aggregator can pivot per stage. Tests construct the
    /// dispatcher without metrics — the increments are no-ops in that
    /// case.
    metrics: Option<Arc<Metrics>>,
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
            metrics: None,
        }
    }

    /// Phase8b — attach a metrics registry so per-hook /
    /// per-kind counters land alongside the dispatch path. Returns
    /// `self` so the caller can chain the construction.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Phase8b — read-only access to the dispatcher's metrics
    /// registry. Used by the IPC handler to bump
    /// `frames_received_total{hook}` once the JSON parses without
    /// re-threading the registry through every entrypoint.
    pub fn metrics(&self) -> Option<&Arc<Metrics>> {
        self.metrics.as_ref()
    }

    /// Handle one hook frame end-to-end. Always returns a response —
    /// internal errors degrade to [`HookResponse::empty`].
    pub async fn dispatch(&self, frame: HookFrame) -> HookResponse {
        // Phase11x — record an invocation line in
        // `~/.cortex/hook-invocations.log`. The legacy `.sh` / `.ps1`
        // shims used to do this themselves; now that `cortex-hook` is
        // a thin native binary that does no I/O beyond the IPC round-
        // trip, the dispatcher owns hook-side logging.
        crate::hook_log::record_invocation(&frame, self.pid);

        // Phase8b — bump `frames_parsed_total{hook}` + stamp the per-
        // hook freshness ts. The IPC handler bumped
        // `frames_received_total{hook}` upstream when the JSON
        // deserialised; this counter pairs with it so a divergence
        // (received > parsed for one hook) localises a malformed
        // payload to the hook label.
        if let Some(m) = self.metrics.as_ref() {
            m.incr_frames_parsed(&frame.hook);
        }

        let kind = match HookKind::parse(&frame.hook) {
            Some(k) => k,
            None => {
                tracing::warn!(hook = %frame.hook, "unknown hook kind");
                return HookResponse::empty();
            }
        };

        // Resolve correlation IDs before sync path so PreToolUse can
        // reference the in-flight turn even though it doesn't publish.
        // Claude Code passes the canonical Claude session id INSIDE
        // the hook stdin JSON (`payload.session_id`) — fall back to
        // it when the shim couldn't read CLAUDE_SESSION_ID from the
        // env. Without this fallback every concurrent Claude Code
        // window collapses onto the daemon's pid-keyed default and
        // sessions stop being distinguishable.
        let resolved_hint = match frame.session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => Some(s.to_string()),
            None => frame
                .payload
                .get("session_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from),
        };
        let session_id = self
            .sessions
            .resolve_or_synthesize(resolved_hint.as_deref(), self.pid);
        self.sessions.ensure(&session_id);

        // build_event sees the same effective hint via the same
        // SessionManager cache, so the envelope's session_id matches
        // the one we computed here.
        let mut frame_for_build = frame.clone();
        if frame_for_build
            .session_id
            .as_deref()
            .map(|s| s.is_empty())
            .unwrap_or(true)
        {
            frame_for_build.session_id = resolved_hint;
        }

        // build_event updates correlation tables (open turn / open tool
        // call) and returns Some(envelope) only for the publishable
        // hooks (UserPromptSubmit, PostToolUse, SubagentStop).
        let envelope = build_event(kind, &frame_for_build, &self.sessions, self.pid);
        let turn_id = self.sessions.current_turn(&session_id);

        let response = self
            .maybe_sync_path(kind, &frame, &session_id, turn_id.as_deref())
            .await;

        // Only publishable hooks reach the queue. Spec 04 has no
        // canonical kind for PreToolUse / Stop / SessionStart /
        // Notification, so they're sync-only.
        if let Some(env) = envelope {
            // Phase8b — stamp `envelopes_built_total{kind}` +
            // `last_envelope_ts{kind}` BEFORE handing to the
            // publisher. The publisher's success path stamps
            // `envelopes_publish_ok_total{kind}`; the gap between
            // built vs publish_ok is the "queue dropped or WAL
            // spilled" signal.
            if let Some(m) = self.metrics.as_ref() {
                m.incr_envelopes_built(env.kind.schema_stem());
            }
            self.publisher.publish(env).await;
        }

        response
    }

    async fn maybe_sync_path(
        &self,
        kind: HookKind,
        frame: &HookFrame,
        session_id: &str,
        turn_id: Option<&str>,
    ) -> HookResponse {
        match kind {
            HookKind::UserPromptSubmit => {
                let prompt = read_string_field(&frame.payload, "prompt")
                    .or_else(|| read_string_field(&frame.payload, "user_message"))
                    .unwrap_or_default();
                let result = self
                    .sync
                    .pre_thinking(&prompt, session_id, turn_id, frame.cwd.as_deref())
                    .await;
                HookResponse::additional_context(result.bundle)
            }
            HookKind::PreToolUse => {
                let tool_name =
                    read_string_field(&frame.payload, "tool_name").unwrap_or_default();
                let input = frame.payload.get("input").cloned().unwrap_or(json!({}));
                let result = self
                    .sync
                    .law_check(&tool_name, &input, session_id, turn_id)
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
