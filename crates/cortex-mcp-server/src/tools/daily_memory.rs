//! Bounded local memory transport; API owns durable delivery and authorization.

use super::{reasons, Tool, ToolContext, ToolError, ToolResult, MCP_RESPONSE_HARD_CAP};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Read explicit local decisions/lessons through `POST /v1/memory/query`.
pub struct MemoryReadTool;

impl MemoryReadTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize, Serialize)]
struct ReadArgs {
    repo: String,
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    include_global: bool,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_budget")]
    budget_bytes: usize,
}

fn default_limit() -> usize {
    5
}

fn default_budget() -> usize {
    1536
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &'static str {
        "cortex_memory_read"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": self.name(),
            "description": "Read applicable accepted local decisions and lessons without optional indexing services. Returns bounded cited context, exact prepared IDs/versions and durable query_id/delivery_id for feedback; prepared does not prove model consumption. Repository rules remain higher authority.",
            "inputSchema": {
                "type": "object", "required": ["repo", "query"],
                "properties": {
                    "repo": { "type": "string", "description": "Exact repository identity; never inferred as global." },
                    "query": { "type": "string", "description": "Ordinary task text, at most 4 KiB; never FTS syntax." },
                    "branch": { "type": "string" },
                    "paths": { "type": "array", "maxItems": 32, "items": { "type": "string" }, "description": "Current repository-relative file paths, using / separators." },
                    "include_global": { "type": "boolean", "default": false, "description": "Explicit authorized global opt-in." },
                    "limit": { "type": "integer", "minimum": 0, "maximum": 5, "default": 5 },
                    "budget_bytes": { "type": "integer", "minimum": 0, "maximum": 1536, "default": 1536 }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let mut request: ReadArgs = serde_json::from_value(args)
            .map_err(|error| ToolError::invalid_input(format!("memory read args: {error}")))?;
        if request.repo.trim().is_empty()
            || request.repo.trim() != request.repo
            || request.repo.len() > 256
            || request.repo.chars().any(char::is_control)
        {
            return Err(ToolError::invalid_input(
                "repo must be nonempty, trimmed, at most 256 bytes and contain no control characters",
            ));
        }
        if request.query.len() > 4096 {
            return Err(ToolError::invalid_input("query exceeds 4 KiB"));
        }
        if request.paths.len() > 32 {
            return Err(ToolError::invalid_input(
                "at most 32 applicability paths are allowed",
            ));
        }
        request.limit = request.limit.min(default_limit());
        request.budget_bytes = request.budget_bytes.min(default_budget());
        let body = serde_json::to_value(request)
            .map_err(|error| ToolError::invalid_input(format!("memory read args: {error}")))?;
        post_memory(ctx, "/v1/memory/query", &body).await
    }
}

pub(super) async fn post_memory(
    ctx: &ToolContext,
    path: &str,
    body: &Value,
) -> Result<ToolResult, ToolError> {
    let url = format!("{}{path}", ctx.api_url.trim_end_matches('/'));
    // This identifies the adapter only. Agent labels live in request data and
    // never become authentication or an invented Claude/session identity.
    let mut request = ctx
        .http
        .post(&url)
        .header(cortex_api::CALLER_HEADER, "cortex-mcp-server")
        .json(body);
    if let Some(key) = &ctx.api_key {
        request = request.bearer_auth(key);
    }
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            return Ok(ToolResult::soft_error(
                reasons::API_UNREACHABLE,
                "cortex-api memory request failed",
                json!({ "path": path }),
            ))
        }
    };
    let status = response.status();
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > MCP_RESPONSE_HARD_CAP {
                    return Ok(over_budget());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => {
                return Ok(ToolResult::soft_error(
                    reasons::API_UNREACHABLE,
                    "cortex-api memory response interrupted",
                    json!({ "path": path }),
                ))
            }
        }
    }
    let upstream: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return Ok(ToolResult::soft_error(
                reasons::API_HTTP_ERROR,
                "cortex-api memory response was not valid JSON",
                json!({ "status": status.as_u16() }),
            ))
        }
    };
    let result = if status.is_success() {
        ToolResult::ok(upstream)
    } else {
        let reason = upstream
            .get("error")
            .or_else(|| upstream.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or(reasons::API_HTTP_ERROR);
        ToolResult::soft_error(
            reason,
            &format!("cortex-api returned {status} for {path}"),
            json!({ "status": status.as_u16(), "body": upstream }),
        )
    };
    // MCP embeds JSON inside a text part; count the escaped result as well.
    if serde_json::to_vec(&result).map_or(true, |bytes| {
        bytes.len() > MCP_RESPONSE_HARD_CAP.saturating_sub(1024)
    }) {
        return Ok(over_budget());
    }
    Ok(result)
}

fn over_budget() -> ToolResult {
    ToolResult::soft_error(
        reasons::BUDGET_EXCEEDED,
        "memory response exceeds MCP transport budget",
        json!({ "transport_cap_bytes": MCP_RESPONSE_HARD_CAP }),
    )
}
