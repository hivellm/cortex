//! MCP tool binding for `cortex_query`.
//!
//! Sibling modules live under `crates/cortex-api/src/mcp/*.rs`
//! (currently `topic_card` for the topic-card MCP tool surface).
//!
//! Spec 11 §MCP tool binding: the same JSON schema is exposed as an
//! MCP tool so agent hosts can call the orchestrator without
//! speaking HTTP. The full MCP-runtime wire protocol lands with the
//! spec-17 multi-adapter rollout; for now the binding exposes:
//!
//! - [`tool_descriptor`] — the JSON document an MCP host advertises
//!   (`name`, `description`, `inputSchema`, `outputSchema`) per the
//!   MCP 2024-11-05 contract: tool names are identifier-safe (no
//!   dots) and schema fields are camelCase.
//! - [`invoke`] — a uniform handler the runtime calls with the
//!   parsed input and returns the parsed output. Identical
//!   surface-by-construction to the HTTP path so behaviour is
//!   guaranteed to stay aligned.

pub mod topic_card;

use std::sync::Arc;

use serde_json::{json, Value};
use thiserror::Error;

use crate::service::{QueryService, ServiceOutcome};
use crate::types::{QueryRequest, QueryResponse};

/// Tool name surfaced to MCP hosts. Identifier-safe per MCP
/// 2024-11-05; clients reject names containing `.`.
pub const TOOL_NAME: &str = "cortex_query";

/// MCP-side error shape. Translates the spec-11 outcomes into JSON
/// the runtime can serialise as a tool error.
#[derive(Debug, Error)]
pub enum McpError {
    /// 400-equivalent.
    #[error("empty query")]
    EmptyQuery,
    /// 422-equivalent — phase6a: scope.repo could not be resolved.
    #[error("scope.repo is required (set scope.repo or pass cwd)")]
    ScopeRepoRequired,
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
        "inputSchema": input_schema(),
        "outputSchema": output_schema(),
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
            "budget_bytes": {
                "type": "integer",
                "minimum": 1024,
                "maximum": 262144,
                "default": 32768,
                "description": "Max serialised JSON bytes for the response. Phase11c — keeps the bundle under the MCP transport's per-tool-result cap. Caller can tighten / loosen; omit for the 32 KiB default.",
            },
            "as_of": {
                "type": "string",
                "description": "Phase18 §4.4 — render the response as it would be believed at this point in valid time (ADR-018 second-precision). Accepts RFC-3339 or `YYYY-MM-DD`. Missing defaults to wall-clock now.",
            },
            "branch": {
                "type": "string",
                "description": "Phase18 §4.4 — branch composite id `<project>:<branch>` (ADR-019). Missing defaults to `<scope.repo>:main`.",
            },
            "projects": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Phase18 §4.4 — cross-project axis activation list (ADR-020). Default off; explicit list unions facts from those projects into the candidate set.",
            },
            "include_history": {
                "type": "boolean",
                "default": false,
                "description": "Phase18 §4.4 — demote SUPERSEDED/EXPIRED candidates instead of dropping them. Default off.",
            },
            "include_future": {
                "type": "boolean",
                "default": false,
                "description": "Phase18 §4.4 — keep NOT_YET_VALID candidates (planning queries). Default off.",
            },
            "include_branches": {
                "type": "boolean",
                "default": false,
                "description": "Phase18 §4.4 — keep ABANDONED branch hits when the caller scopes a specific branch. Default off.",
            },
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
    let req: QueryRequest =
        serde_json::from_value(input).map_err(|e| McpError::Invalid(e.to_string()))?;
    match service.handle(caller, req).await {
        ServiceOutcome::Ok(resp) => Ok(*resp),
        ServiceOutcome::EmptyQuery => Err(McpError::EmptyQuery),
        ServiceOutcome::EmptyScope => Err(McpError::ScopeRepoRequired),
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
        assert_eq!(d["name"], "cortex_query");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        let input = &d["inputSchema"];
        assert_eq!(input["type"], "object");
        assert!(input["properties"]["intent"]["enum"]
            .as_array()
            .unwrap()
            .contains(&Value::String("pre_change_context".into())));
        assert_eq!(input["required"], json!(["intent", "query"]));
    }

    #[test]
    fn input_schema_exposes_budget_bytes_for_phase11c() {
        // Phase11c spec scenario "MCP schema exposes budget_bytes".
        let d = tool_descriptor();
        let props = d["inputSchema"]["properties"]
            .as_object()
            .expect("properties is an object");
        let bb = props
            .get("budget_bytes")
            .expect("budget_bytes must be advertised");
        assert_eq!(bb["type"], "integer");
        assert_eq!(bb["default"], 32768);
    }

    #[test]
    fn budget_bytes_round_trips_through_query_request_serde() {
        // Phase11c spec scenario "MCP forwards budget_bytes verbatim"
        // — the request body POSTed to /v1/query carries the same
        // value the MCP caller supplied, no truncation, no rename.
        let raw = json!({
            "intent": "free_search",
            "query": "x",
            "budget_bytes": 8192,
        });
        let req: QueryRequest = serde_json::from_value(raw).expect("schema-compatible");
        assert_eq!(req.budget_bytes, Some(8192));
        let back = serde_json::to_value(&req).expect("serialisable");
        assert_eq!(back["budget_bytes"], 8192);
    }

    #[test]
    fn output_schema_lists_every_response_top_level_field() {
        let d = tool_descriptor();
        assert!(
            d.get("output_schema").is_none(),
            "snake_case output_schema must not be emitted"
        );
        let props = d["outputSchema"]["properties"].as_object().unwrap();
        for k in [
            "intent",
            "query_id",
            "scope_resolved",
            "results",
            "budget",
            "debug",
        ] {
            assert!(props.contains_key(k), "missing {k}");
        }
    }
}
