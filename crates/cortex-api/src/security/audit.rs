//! Query-audit publisher. Spec 11 §Security / privacy: every
//! request emits one envelope onto `cortex.events.query_audit`
//! carrying caller, intent, scope, result counts, and latency.
//!
//! The trait is publisher-shaped so tests use a recording fake; the
//! live wiring against `synap-sdk` mirrors the publisher pattern
//! shared by the Phase-1 worker crates.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::fusion::FusionConfig;
use crate::query_rewrite::RewrittenQuery;
use crate::types::QueryResponse;

/// Stream name spec 11 declares for the audit envelope.
pub const STREAM_QUERY_AUDIT: &str = "cortex.events.query_audit";

/// Audit publisher trait.
#[async_trait]
pub trait AuditPublisher: Send + Sync {
    /// Publish one envelope.
    async fn publish(&self, envelope: Value);
}

/// In-memory recording publisher (tests).
#[derive(Default)]
pub struct MemoryAuditPublisher {
    /// Recorded envelopes.
    pub events: Mutex<Vec<Value>>,
}

impl MemoryAuditPublisher {
    /// Fresh recorder.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot the recorded list.
    pub fn snapshot(&self) -> Vec<Value> {
        self.events.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl AuditPublisher for MemoryAuditPublisher {
    async fn publish(&self, envelope: Value) {
        if let Ok(mut g) = self.events.lock() {
            g.push(envelope);
        }
    }
}

/// Build the audit envelope from the response + caller context.
///
/// Older callers without phase6a's resolution-lane awareness fall
/// through this entry; the envelope omits the `scope_resolution`
/// field so the wire shape stays compatible with v1 consumers.
pub fn build_envelope(caller: &str, intent: &str, response: &QueryResponse) -> Value {
    json!({
        "kind": "query_audit",
        "caller": caller,
        "query_id": response.query_id,
        "intent": intent,
        "scope": response.scope_resolved,
        "counts": {
            "snippets": response.results.snippets.len(),
            "decisions": response.results.decisions.len(),
            "violations": response.results.violations.len(),
            "graph_neighbors": response.results.graph_neighbors.len(),
            "similar_turns": response.results.similar_turns.len(),
            "laws_active": response.laws_active.len(),
        },
        "latency_ms": response.budget.used_ms,
        "cache": response.budget.cache,
    })
}

/// Phase6a — build the audit envelope and stamp the
/// `scope_resolution` field (`"explicit"` / `"header"` / `"cwd"` /
/// `"rejected_legacy"`) so the dashboard can flag misconfigured
/// callers before relevance scores degrade.
pub fn build_envelope_with_scope_resolution(
    caller: &str,
    intent: &str,
    response: &QueryResponse,
    scope_resolution: &str,
) -> Value {
    let mut env = build_envelope(caller, intent, response);
    if let Some(obj) = env.as_object_mut() {
        obj.insert(
            "scope_resolution".to_string(),
            Value::String(scope_resolution.to_string()),
        );
    }
    env
}

/// Phase6c — build the audit envelope with the phase6a
/// scope-resolution field AND the fusion-tuning fields
/// (`fusion_alpha`, `fusion_k`). The harness in phase6e relies on
/// these to attribute relevance regressions to fusion-config
/// changes; without them, an A/B that flips `alpha` from 0.7 →
/// 0.5 looks like a model regression instead of a tuning move.
///
/// Phase6d — also accepts an optional `intent_trigger` (the
/// keyword that fired the intent selector). `None` means the
/// prompt fell through to the safe default (`pre_change_context`)
/// with no explicit rule match. Stamped on every envelope so the
/// dashboard can attribute routing changes to the specific rule
/// that fired.
pub fn build_envelope_with_audit_context(
    caller: &str,
    intent: &str,
    response: &QueryResponse,
    scope_resolution: &str,
    fusion: &FusionConfig,
    intent_trigger: Option<&str>,
) -> Value {
    let mut env = build_envelope_with_scope_resolution(caller, intent, response, scope_resolution);
    if let Some(obj) = env.as_object_mut() {
        // `fusion_alpha` is serialised as the f32 value the
        // operator set (or the clamped default); JSON's number
        // type loses the f32-vs-f64 distinction so consumers
        // round-trip through f64 without surprise.
        obj.insert("fusion_alpha".to_string(), json!(fusion.alpha as f64));
        obj.insert("fusion_k".to_string(), json!(fusion.k));
        // Phase6d — explicit Null on fallback so the dashboard can
        // tell "no rule matched" apart from "this envelope was
        // emitted by an older daemon that didn't record the
        // trigger". The selector returns `None` only on the
        // safe-default path.
        obj.insert(
            "intent_trigger".to_string(),
            match intent_trigger {
                Some(k) => Value::String(k.to_string()),
                None => Value::Null,
            },
        );
    }
    env
}

/// Phase6f — extends [`build_envelope_with_audit_context`] with the
/// query-rewriter context (`query_rewrite_strategy`, `vector_query`,
/// `keyword_query`). The harness in phase6e attributes uplift to
/// the rewriter via these fields; operators read them when a
/// specific query routed somewhere unexpected.
pub fn build_envelope_with_rewrite_context(
    caller: &str,
    intent: &str,
    response: &QueryResponse,
    scope_resolution: &str,
    fusion: &FusionConfig,
    intent_trigger: Option<&str>,
    rewritten: &RewrittenQuery,
) -> Value {
    let mut env = build_envelope_with_audit_context(
        caller,
        intent,
        response,
        scope_resolution,
        fusion,
        intent_trigger,
    );
    if let Some(obj) = env.as_object_mut() {
        obj.insert(
            "query_rewrite_strategy".to_string(),
            Value::String(rewritten.strategy.to_string()),
        );
        obj.insert(
            "vector_query".to_string(),
            Value::String(rewritten.vector_query.clone()),
        );
        obj.insert(
            "keyword_query".to_string(),
            Value::String(rewritten.keyword_query.clone()),
        );
    }
    env
}

/// Phase11r §4.1 — build the audit envelope for a single topic-card
/// MCP tool call. Every §4 tool publishes one envelope per
/// invocation so the dashboard can surface the operator's drill /
/// neighbor / synthesize traffic alongside the existing `cortex_query`
/// audit lane. Envelope shape stays close to [`build_envelope`] —
/// `caller`, `tool`, `scope`, plus a tool-specific `result_summary`
/// the caller fills in (e.g. `{"hit": "auth-rewrite", "confidence":
/// 0.82}` for `cortex_topic_get`). The `kind` discriminator is
/// `"topic_card_mcp_audit"` so consumers can filter cleanly without
/// re-parsing the `tool` field.
pub fn build_topic_card_envelope(
    caller: &str,
    tool: &str,
    scope_repo: Option<&str>,
    result_summary: Value,
) -> Value {
    json!({
        "kind": "topic_card_mcp_audit",
        "caller": caller,
        "tool": tool,
        "scope": {
            "repo": scope_repo.unwrap_or(""),
        },
        "result": result_summary,
    })
}

/// Phase11r §4.1 — publish one topic-card audit envelope through the
/// supplied [`AuditPublisher`]. Equivalent to the `record_call` shape
/// the §4 brief named; the helper keeps the call site terse so each
/// tool's `invoke_*` body adds one line at the tail rather than
/// re-rolling the envelope shape.
pub async fn record_topic_card_call(
    publisher: &dyn AuditPublisher,
    caller: &str,
    tool: &str,
    scope_repo: Option<&str>,
    result_summary: Value,
) {
    let env = build_topic_card_envelope(caller, tool, scope_repo, result_summary);
    publisher.publish(env).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{empty_response, IncludeField, Intent, QueryRequest, Scope};

    #[tokio::test]
    async fn memory_publisher_records_published_envelopes() {
        let pub_ = MemoryAuditPublisher::new();
        pub_.publish(json!({ "kind": "x" })).await;
        let snap = pub_.snapshot();
        assert_eq!(snap.len(), 1);
    }

    #[tokio::test]
    async fn record_topic_card_call_publishes_envelope_with_tool_and_scope() {
        // Phase11r §4.1 — the topic-card audit envelope MUST carry
        // the canonical `kind = "topic_card_mcp_audit"`, the tool
        // name, the repo scope, and an opaque result_summary the
        // caller fills in. Pin the shape so a future tool addition
        // does not silently rename a top-level key.
        let pub_ = MemoryAuditPublisher::new();
        record_topic_card_call(
            &pub_,
            "claude-code",
            "cortex_topic_get",
            Some("cortex"),
            json!({ "hit": "auth-rewrite", "confidence": 0.82, "via": "search" }),
        )
        .await;
        let envs = pub_.snapshot();
        assert_eq!(envs.len(), 1);
        let env = &envs[0];
        assert_eq!(env["kind"], "topic_card_mcp_audit");
        assert_eq!(env["caller"], "claude-code");
        assert_eq!(env["tool"], "cortex_topic_get");
        assert_eq!(env["scope"]["repo"], "cortex");
        assert_eq!(env["result"]["hit"], "auth-rewrite");
        assert_eq!(env["result"]["via"], "search");
    }

    #[test]
    fn build_topic_card_envelope_handles_absent_repo() {
        // A failed call (e.g. `ScopeRepoRequired`) still emits an
        // envelope so the dashboard can surface the rejection. The
        // missing repo lands as an empty string rather than `null`
        // so consumers reading `scope.repo` as a string don't trip.
        let env = build_topic_card_envelope(
            "claude-code",
            "cortex_topic_get",
            None,
            json!({ "error": "scope_repo_required" }),
        );
        assert_eq!(env["scope"]["repo"], "");
        assert_eq!(env["result"]["error"], "scope_repo_required");
    }

    #[test]
    fn build_envelope_carries_intent_and_counts() {
        let req = QueryRequest {
            intent: Intent::PreChangeContext,
            scope: Scope::default(),
            query: "x".into(),
            limit: 20,
            k: 50,
            include: vec![IncludeField::Snippets],
            budget_ms: 500,
            budget_bytes: None,
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
            principal: None,
        };
        let resp = empty_response(&req);
        let env = build_envelope("dash", "pre_change_context", &resp);
        assert_eq!(env["caller"], "dash");
        assert_eq!(env["intent"], "pre_change_context");
        assert_eq!(env["counts"]["snippets"], 0);
    }
}
