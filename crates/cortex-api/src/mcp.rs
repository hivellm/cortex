//! MCP tool binding for `cortex.query`.
//!
//! Spec 11 §MCP tool binding: the same JSON schema is exposed as an
//! MCP tool so agent hosts can call the orchestrator without
//! speaking HTTP. The full MCP-runtime wire protocol lands with the
//! spec-17 multi-adapter rollout; for now the binding exposes:
//!
//! - [`tool_descriptor`] — the JSON document an MCP host advertises
//!   (`name`, `description`, `input_schema`, `output_schema`).
//! - [`invoke`] — a uniform handler the runtime calls with the
//!   parsed input and returns the parsed output. Identical
//!   surface-by-construction to the HTTP path so behaviour is
//!   guaranteed to stay aligned.

use std::sync::Arc;

use serde_json::{json, Value};
use thiserror::Error;

use crate::service::{QueryService, ServiceOutcome};
use crate::types::{QueryRequest, QueryResponse};

/// Tool name surfaced to MCP hosts.
pub const TOOL_NAME: &str = "cortex.query";

/// MCP-side error shape. Translates the spec-11 outcomes into JSON
/// the runtime can serialise as a tool error.
#[derive(Debug, Error)]
pub enum McpError {
    /// 400-equivalent.
    #[error("empty query")]
    EmptyQuery,
    /// 403-equivalent.
    #[error("scope forbidden")]
    ScopeForbidden,
    /// 429-equivalent. Carries the suggested wait window in
    /// milliseconds for hosts that respect `retry_after_ms`.
    #[error("rate limited (retry after {retry_after_ms} ms)")]
    RateLimited {
        /// Suggested retry window.
        retry_after_ms: u64,
    },
    /// Runtime-side serde failure.
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// Build the JSON descriptor an MCP host advertises to clients.
pub fn tool_descriptor() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Hybrid retrieval (vector + keyword + graph) over Cortex. \
                        Returns a structured context bundle for an agent prompt.",
        "input_schema": input_schema(),
        "output_schema": output_schema(),
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["intent", "query"],
        "properties": {
            "intent": {
                "type": "string",
                "enum": [
                    "pre_change_context",
                    "decision_lookup",
                    "similar_problems",
                    "law_check",
                    "free_search",
                ],
            },
            "scope": {
                "type": "object",
                "properties": {
                    "repo": { "type": "string" },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "topics": { "type": "array", "items": { "type": "string" } },
                    "since": { "type": "string" },
                },
            },
            "query": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
            "k": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 },
            "include": {
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": [
                        "snippets",
                        "decisions",
                        "violations",
                        "graph_neighbors",
                        "similar_turns",
                    ],
                },
            },
            "budget_ms": { "type": "integer", "minimum": 1, "default": 500 },
        },
    })
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["intent", "query_id", "results", "budget"],
        "properties": {
            "intent": { "type": "string" },
            "query_id": { "type": "string" },
            "scope_resolved": { "type": "object" },
            "results": { "type": "object" },
            "laws_active": { "type": "array" },
            "budget": { "type": "object" },
            "debug": { "type": "object" },
        },
    })
}

/// Parse the MCP-supplied input + dispatch through the service.
pub async fn invoke(
    service: Arc<QueryService>,
    caller: &str,
    input: Value,
) -> Result<QueryResponse, McpError> {
    let req: QueryRequest = serde_json::from_value(input)
        .map_err(|e| McpError::Invalid(e.to_string()))?;
    match service.handle(caller, req).await {
        ServiceOutcome::Ok(resp) => Ok(*resp),
        ServiceOutcome::EmptyQuery => Err(McpError::EmptyQuery),
        ServiceOutcome::Denied => Err(McpError::ScopeForbidden),
        ServiceOutcome::RateLimited(d) => Err(McpError::RateLimited {
            retry_after_ms: d.as_millis() as u64,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_descriptor_advertises_cortex_query_with_required_fields() {
        let d = tool_descriptor();
        assert_eq!(d["name"], "cortex.query");
        let input = &d["input_schema"];
        assert_eq!(input["type"], "object");
        assert!(input["properties"]["intent"]["enum"]
            .as_array()
            .unwrap()
            .contains(&Value::String("pre_change_context".into())));
        assert_eq!(input["required"], json!(["intent", "query"]));
    }

    #[test]
    fn output_schema_lists_every_response_top_level_field() {
        let d = tool_descriptor();
        let props = d["output_schema"]["properties"].as_object().unwrap();
        for k in ["intent", "query_id", "scope_resolved", "results", "budget", "debug"] {
            assert!(props.contains_key(k), "missing {k}");
        }
    }
}
