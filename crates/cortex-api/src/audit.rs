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
pub fn build_envelope(
    caller: &str,
    intent: &str,
    response: &QueryResponse,
) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{empty_response, Intent, QueryRequest, Scope, IncludeField};

    #[tokio::test]
    async fn memory_publisher_records_published_envelopes() {
        let pub_ = MemoryAuditPublisher::new();
        pub_.publish(json!({ "kind": "x" })).await;
        let snap = pub_.snapshot();
        assert_eq!(snap.len(), 1);
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
        };
        let resp = empty_response(&req);
        let env = build_envelope("dash", "pre_change_context", &resp);
        assert_eq!(env["caller"], "dash");
        assert_eq!(env["intent"], "pre_change_context");
        assert_eq!(env["counts"]["snippets"], 0);
    }
}
