//! Hook dispatcher — the daemon's single per-frame entry point.
//!
//! Spec 10 §Hook ↔ daemon protocol: the daemon receives a `HookFrame`,
//! decides whether the hook needs a synchronous call, builds the
//! envelope, hands it to the async publisher, and replies in the
//! protocol-defined JSON shape. Any internal error short-circuits to
//! an empty `{}` reply so the session never breaks.

use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cortex_api::active_work::ActiveTaskRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Operator warning; the native shim deduplicates this by actual session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
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
            system_message: None,
        }
    }

    /// Build a `SessionStart` response from a Markdown bundle
    /// (phase30 §1.3). Mirrors [`HookResponse::additional_context`]
    /// but stamps `hookEventName: "SessionStart"` — Claude Code uses
    /// that field to route the `additionalContext` payload, so
    /// reusing the `UserPromptSubmit` literal here would silently
    /// drop the active-work block. An empty bundle short-circuits to
    /// [`HookResponse::empty`], same as the UserPromptSubmit path.
    pub fn session_start_context(bundle: String) -> Self {
        if bundle.is_empty() {
            return Self::empty();
        }
        Self {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "SessionStart".into(),
                additional_context: bundle,
            }),
            permission_decision: None,
            permission_decision_reason: None,
            system_message: None,
        }
    }

    /// Build a PreToolUse `deny` response.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: None,
            permission_decision: Some("deny".into()),
            permission_decision_reason: Some(reason.into()),
            system_message: None,
        }
    }

    fn with_memory_outcome(mut self, outcome: &str) -> Self {
        if matches!(
            outcome,
            "unavailable"
                | "timeout"
                | "invalid_response"
                | "scope_timeout"
                | "scope_identity_required"
                | "scope_unresolved"
        ) {
            self.system_message = Some(
                "Cortex: local memory is unavailable. This prompt continues without saved Cortex memory.".into(),
            );
        }
        self
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
        let envelope = if self.publisher.enabled() {
            // The effective hint must match the identity used by sync memory.
            let mut frame_for_build = frame.clone();
            if frame_for_build
                .session_id
                .as_deref()
                .map(|s| s.is_empty())
                .unwrap_or(true)
            {
                frame_for_build.session_id = resolved_hint;
            }
            build_event(kind, &frame_for_build, &self.sessions, self.pid)
        } else {
            // Memory deliveries still carry turn identity. Skip bulk payload
            // cloning/redaction, transcript reads, and tool/event correlation.
            match kind {
                HookKind::UserPromptSubmit => {
                    self.sessions.open_turn(&session_id);
                }
                HookKind::Stop => self.sessions.close_turn(&session_id),
                _ => {}
            }
            None
        };
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
                    .with_memory_outcome(result.memory_outcome)
            }
            HookKind::PreToolUse => {
                // The legacy /v1/laws/check route is not implemented. Keep
                // correlation, without a request or an implied policy decision.
                HookResponse::empty()
            }
            HookKind::SessionStart => {
                let local = self
                    .sync
                    .session_start_memory(session_id, frame.cwd.as_deref());
                let work = async {
                    let repo = task_repo(frame.cwd.as_deref()).await;
                    self.sync.active_work(repo.as_deref()).await
                };
                let (memory, work) = tokio::join!(local, work);
                let mut bundle = memory.bundle;
                bundle.push_str(&render_active_work(
                    &work.tasks,
                    self.sync.bundle_budget().saturating_sub(bundle.len()),
                ));
                HookResponse::session_start_context(bundle)
                    .with_memory_outcome(memory.memory_outcome)
            }
            _ => HookResponse::empty(),
        }
    }
}

async fn task_repo(cwd: Option<&str>) -> Option<String> {
    static SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
        LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
    let cwd = cwd.filter(|cwd| Path::new(cwd).is_absolute())?.to_owned();
    tokio::time::timeout(Duration::from_millis(100), async {
        let permit = SLOTS.clone().acquire_owned().await.ok()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            cortex_pre_thinking::repo_from_cwd(Path::new(&cwd)).map(|r| r.to_ascii_lowercase())
        })
        .await
        .ok()?
    })
    .await
    .ok()?
}

fn read_string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Phase30 (continuity §1.3) — hard cap on the number of active-task
/// rows rendered into the `SessionStart` Markdown block. The daemon's
/// `/v1/dashboard/active-work` endpoint (and the `cortex_active_work`
/// MCP wrapper) may carry far more rows; `SessionStart` only has
/// budget for a short "what's already in flight" reminder, not the
/// full operator dashboard.
const ACTIVE_WORK_ROW_CAP: usize = 8;

/// Render the `SessionStart` "active work" block. Deterministic:
/// preserves the daemon's own ordering (phase, then id — see
/// `cortex_api::active_work::sort_key`) and truncates to
/// [`ACTIVE_WORK_ROW_CAP`] rows. Returns an empty string when `tasks`
/// is empty so [`HookResponse::session_start_context`] short-circuits
/// to `HookResponse::empty()`.
fn render_active_work(tasks: &[ActiveTaskRow], budget: usize) -> String {
    if tasks.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Cortex — active work (rulebook)\n");
    let cap = budget.min(4096);
    if out.len() > cap {
        return String::new();
    }
    for task in tasks.iter().take(ACTIVE_WORK_ROW_CAP) {
        let start = out.len();
        out.push_str("- ");
        out.push_str(&task.id);
        out.push_str(" [");
        out.push_str(&task.status);
        out.push(']');
        match task.next_unchecked_item.as_deref() {
            Some(item) if !item.is_empty() => {
                out.push_str(" → next: ");
                out.push_str(item);
            }
            _ => out.push_str(" → next: (no unchecked items)"),
        }
        if task.status == "blocked" {
            if let Some(reason) = task.blocked_reason.as_deref() {
                out.push_str(" (blocked: ");
                out.push_str(reason);
                out.push(')');
            }
        }
        out.push('\n');
        if out.len() > cap {
            out.truncate(start);
            break;
        }
    }
    out
}
