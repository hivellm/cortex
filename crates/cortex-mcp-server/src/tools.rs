//! Tool implementations exposed to the MCP host.
//!
//! Spec 18 §Tool implementations:
//! - [`QueryTool`] — POST `<api_url>/v1/query`.
//! - [`PreThinkingTool`] — runs the spec-12 pipeline against the same
//!   `cortex-api`.
//! - [`StatusTool`] — best-effort daemon health snapshot.
//!
//! Every tool implements [`Tool`]: a name, an MCP descriptor, and an
//! async `call` that returns either a serialised `result.content`
//! payload or a structured [`ToolError`]. The dispatch loop in
//! [`crate::server`] catches panics so a buggy tool can't kill the
//! transport.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::{
    tool_descriptor as query_tool_descriptor, Intent, QueryRequest, QueryResponse, TOOL_NAME,
};
use cortex_pre_thinking::pipeline::{
    ClosureQueryFn, PreThinkingBudget, PreThinkingInput, PreThinkingOutput,
};
use cortex_pre_thinking::{Metrics as PreThinkingMetrics, RecentFile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Spec-11 reasons mirrored when an upstream call fails. Surfaced as
/// the JSON-RPC `error.data.reason` so callers can pattern-match.
pub mod reasons {
    /// `cortex-api` was unreachable.
    pub const API_UNREACHABLE: &str = "api_unreachable";
    /// `cortex-api` returned 4xx but the body did not have a usable
    /// reason field.
    pub const API_HTTP_ERROR: &str = "api_http_error";
    /// Tool input failed schema validation client-side.
    pub const INVALID_INPUT: &str = "invalid_input";
    /// Pre-thinking pipeline returned an empty bundle (fail-open).
    pub const EMPTY_BUNDLE: &str = "empty_bundle";
}

/// Caller name advertised on the `x-cortex-caller` header.
pub const CALLER: &str = "claude-code-plugin";

/// Shared dependencies passed to every tool invocation.
#[derive(Clone)]
pub struct ToolContext {
    /// Base URL for the Cortex daemon (`http://127.0.0.1:15011` by default).
    pub api_url: String,
    /// Reusable HTTP client.
    pub http: reqwest::Client,
    /// Pre-thinking pipeline metrics handle.
    pub pt_metrics: Arc<PreThinkingMetrics>,
    /// Server start instant — surfaced through `cortex_status`.
    pub started_at: std::time::Instant,
}

impl ToolContext {
    /// Build a default context with a 5-second per-call timeout.
    pub fn new(api_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            api_url: api_url.into(),
            http,
            pt_metrics: Arc::new(PreThinkingMetrics::new()),
            started_at: std::time::Instant::now(),
        }
    }

    /// Test-friendly constructor that takes an explicit client.
    pub fn with_client(api_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            api_url: api_url.into(),
            http,
            pt_metrics: Arc::new(PreThinkingMetrics::new()),
            started_at: std::time::Instant::now(),
        }
    }
}

/// Outcome the dispatcher serialises into a JSON-RPC response.
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    /// MCP `content` array — always a single text part carrying the
    /// JSON-encoded structured payload.
    pub content: Vec<Value>,
    /// MCP `isError` flag — `true` when the tool ran but returned an
    /// upstream-side failure the host should surface to the user.
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    /// Build a success result whose only content part is the
    /// JSON-encoded `payload`.
    pub fn ok(payload: Value) -> Self {
        let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
        Self {
            content: vec![json!({ "type": "text", "text": text })],
            is_error: false,
        }
    }

    /// Build a soft-error result — the dispatcher returns this as a
    /// JSON-RPC `result` (not an `error`) with `isError=true`, which
    /// matches the MCP convention for tool-side failures.
    pub fn soft_error(reason: &str, message: &str, extra: Value) -> Self {
        let body = json!({
            "reason": reason,
            "message": message,
            "details": extra,
        });
        let text = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
        Self {
            content: vec![json!({ "type": "text", "text": text })],
            is_error: true,
        }
    }
}

/// JSON-RPC-side error a tool can raise. Bubbles up to the dispatcher
/// which translates it into an `error.code` + `error.data` envelope.
#[derive(Debug, Clone, Serialize)]
pub struct ToolError {
    /// Spec-11 reason string.
    pub reason: String,
    /// Short human-readable message.
    pub message: String,
}

impl ToolError {
    /// `-32602` invalid params.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            reason: reasons::INVALID_INPUT.into(),
            message: message.into(),
        }
    }
}

/// Surface every tool implements.
#[async_trait]
pub trait Tool: Send + Sync {
    /// MCP tool name (e.g. `cortex_query`). Identifier-safe per the
    /// MCP 2024-11-05 contract — names with `.` are rejected by
    /// clients (Claude Code silently drops the descriptor).
    fn name(&self) -> &'static str;
    /// MCP descriptor advertised by `tools/list`.
    fn descriptor(&self) -> Value;
    /// Run the tool. Returns either a structured [`ToolResult`] (the
    /// dispatcher serialises it as a JSON-RPC `result`) or a
    /// [`ToolError`] (the dispatcher renders it as `-32602` /
    /// `-32603`).
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError>;
}

/// Read-only registry the dispatcher iterates over.
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Build a registry with the three Cortex tools wired up.
    pub fn default_set() -> Self {
        Self {
            tools: vec![
                Arc::new(QueryTool::new()),
                Arc::new(PreThinkingTool::new()),
                Arc::new(StatusTool::new()),
            ],
        }
    }

    /// Empty registry — used by tests + `validate` mode.
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool.
    pub fn push(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Iterator over the descriptors `tools/list` returns.
    pub fn descriptors(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.descriptor()).collect()
    }

    /// Look a tool up by name.
    pub fn find(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// `true` when no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::default_set()
    }
}

// ---------------------------------------------------------------------
// cortex_query
// ---------------------------------------------------------------------

/// Wraps `POST <api_url>/v1/query` (spec 11).
pub struct QueryTool;

impl QueryTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for QueryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for QueryTool {
    fn name(&self) -> &'static str {
        TOOL_NAME
    }

    fn descriptor(&self) -> Value {
        // Source-of-truth lives in cortex-api: spec 18 Decision 3.
        query_tool_descriptor()
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let req: QueryRequest = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid_input(format!("query request: {e}")))?;

        let url = format!("{}/v1/query", ctx.api_url.trim_end_matches('/'));
        let resp = match ctx
            .http
            .post(&url)
            .header(cortex_api::CALLER_HEADER, CALLER)
            .json(&req)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::soft_error(
                    reasons::API_UNREACHABLE,
                    &format!("cortex-api unreachable at {url}: {e}"),
                    json!({ "url": url }),
                ));
            }
        };

        let status = resp.status();
        let body_bytes = resp.bytes().await.unwrap_or_default();

        if status.is_success() {
            let parsed: QueryResponse = serde_json::from_slice(&body_bytes).map_err(|e| {
                ToolError::invalid_input(format!("upstream response not valid: {e}"))
            })?;
            let payload = serde_json::to_value(&parsed).unwrap_or(Value::Null);
            return Ok(ToolResult::ok(payload));
        }

        // 4xx — pull the spec-11 reason out of the body.
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
        let reason = body_json
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or(reasons::API_HTTP_ERROR);
        Ok(ToolResult::soft_error(
            reason,
            &format!("cortex-api returned {status}"),
            json!({ "status": status.as_u16(), "body": body_json }),
        ))
    }
}

// ---------------------------------------------------------------------
// cortex_pre_thinking
// ---------------------------------------------------------------------

/// Wraps the spec-12 pipeline backed by `cortex-api`.
pub struct PreThinkingTool;

impl PreThinkingTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PreThinkingTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Inbound shape for `cortex_pre_thinking`.
#[derive(Debug, Clone, Deserialize)]
struct PreThinkingArgs {
    user_prompt: String,
    cwd: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    budget_bytes: Option<u32>,
    #[serde(default)]
    budget_ms: Option<u32>,
    #[serde(default)]
    recent_files: Vec<RecentFile>,
}

#[async_trait]
impl Tool for PreThinkingTool {
    fn name(&self) -> &'static str {
        "cortex_pre_thinking"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_pre_thinking",
            "description": "Run the spec-12 pre-thinking pipeline against the configured cortex-api and return the deterministic Markdown bundle.",
            "inputSchema": {
                "type": "object",
                "required": ["user_prompt", "cwd"],
                "properties": {
                    "user_prompt": { "type": "string" },
                    "cwd": { "type": "string" },
                    "session_id": { "type": "string" },
                    "turn_id": { "type": "string" },
                    "budget_bytes": { "type": "integer", "default": 32768 },
                    "budget_ms": { "type": "integer", "default": 600 },
                    "recent_files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "status", "age_seconds"],
                            "properties": {
                                "path": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["modified", "staged", "untracked"]
                                },
                                "age_seconds": { "type": "integer" }
                            }
                        }
                    }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let parsed: PreThinkingArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid_input(format!("pre_thinking args: {e}")))?;

        let cwd = PathBuf::from(&parsed.cwd);
        let session_id = parsed.session_id.unwrap_or_else(|| "session".into());
        let turn_id = parsed.turn_id.unwrap_or_else(|| "turn".into());
        let budget = PreThinkingBudget {
            bundle_bytes: parsed.budget_bytes.unwrap_or(32 * 1024),
            time_ms: parsed.budget_ms.unwrap_or(600),
        };

        let api_url = ctx.api_url.clone();
        let http = ctx.http.clone();
        let query_fn = Arc::new(ClosureQueryFn(move |req: QueryRequest| {
            let api_url = api_url.clone();
            let http = http.clone();
            async move { post_query(&http, &api_url, req).await }
        }));

        let input = PreThinkingInput {
            session_id: &session_id,
            turn_id: &turn_id,
            user_prompt: &parsed.user_prompt,
            cwd: &cwd,
            recent_files: &parsed.recent_files,
            budget,
        };

        let out: PreThinkingOutput =
            cortex_pre_thinking::pipeline::run(&input, query_fn, ctx.pt_metrics.clone()).await;

        if out.bundle.is_empty() {
            return Ok(ToolResult::soft_error(
                reasons::EMPTY_BUNDLE,
                "pre-thinking produced an empty bundle (fail-open path)",
                json!({
                    "intent": out.intent.label(),
                    "fail_open": out.fail_open,
                    "latency_ms": out.latency_ms,
                }),
            ));
        }

        Ok(ToolResult::ok(json!({
            "bundle": out.bundle,
            "intent": out.intent.label(),
            "query_id": out.query_id,
            "steps_applied": out.steps_applied,
            "latency_ms": out.latency_ms,
            "fail_open": out.fail_open,
        })))
    }
}

async fn post_query(
    http: &reqwest::Client,
    api_url: &str,
    req: QueryRequest,
) -> Option<QueryResponse> {
    let url = format!("{}/v1/query", api_url.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .header(cortex_api::CALLER_HEADER, CALLER)
        .json(&req)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<QueryResponse>().await.ok()
}

// ---------------------------------------------------------------------
// cortex_status
// ---------------------------------------------------------------------

/// Daemon health probe.
pub struct StatusTool;

impl StatusTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for StatusTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for StatusTool {
    fn name(&self) -> &'static str {
        "cortex_status"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_status",
            "description": "Cortex daemon health snapshot: pid, queue depth, recent publisher errors, overflow WAL bytes.",
            "inputSchema": { "type": "object", "properties": {} }
        })
    }

    async fn call(&self, ctx: &ToolContext, _args: Value) -> Result<ToolResult, ToolError> {
        let url = format!("{}/v1/status", ctx.api_url.trim_end_matches('/'));
        let mut payload = json!({
            "mcp_server": {
                "name": crate::server::SERVER_NAME,
                "version": crate::server::SERVER_VERSION,
                "pid": std::process::id(),
                "uptime_ms": ctx.started_at.elapsed().as_millis() as u64,
            },
            "api_url": ctx.api_url,
            "api_reachable": false,
            "daemon": Value::Null,
        });

        match ctx.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                payload["api_reachable"] = json!(true);
                if let Ok(body) = resp.json::<Value>().await {
                    payload["daemon"] = body;
                }
            }
            Ok(resp) => {
                payload["api_reachable"] = json!(false);
                payload["daemon"] = json!({
                    "reason": "status_endpoint_returned_non_2xx",
                    "status": resp.status().as_u16(),
                });
            }
            Err(e) => {
                payload["api_reachable"] = json!(false);
                payload["daemon"] = json!({
                    "reason": reasons::API_UNREACHABLE,
                    "message": e.to_string(),
                });
            }
        }

        // Make absolutely sure the canonical Intent::FreeSearch label is reachable —
        // forces the cortex-api enum into the binary even when only QueryTool runs.
        let _ = Intent::FreeSearch.label();

        Ok(ToolResult::ok(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_returns_three_tools_with_unique_names() {
        let reg = ToolRegistry::default_set();
        assert_eq!(reg.len(), 3);
        let names: Vec<&str> = reg.tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"cortex_query"));
        assert!(names.contains(&"cortex_pre_thinking"));
        assert!(names.contains(&"cortex_status"));
        for n in &names {
            assert!(
                !n.contains('.'),
                "tool name {n} contains '.' — MCP spec forbids dots"
            );
        }
    }

    #[test]
    fn query_tool_descriptor_matches_cortex_api_source_of_truth() {
        let t = QueryTool::new();
        assert_eq!(t.descriptor(), cortex_api::tool_descriptor());
    }

    #[test]
    fn pre_thinking_descriptor_lists_required_fields() {
        let t = PreThinkingTool::new();
        let d = t.descriptor();
        assert_eq!(d["name"], "cortex_pre_thinking");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        let req = d["inputSchema"]["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "user_prompt"));
        assert!(req.iter().any(|v| v == "cwd"));
    }

    #[test]
    fn status_descriptor_takes_no_input() {
        let t = StatusTool::new();
        let d = t.descriptor();
        assert_eq!(d["name"], "cortex_status");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        assert_eq!(d["inputSchema"]["type"], "object");
        let props = d["inputSchema"]["properties"].as_object().unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn tool_result_serialises_is_error_flag() {
        let ok = ToolResult::ok(json!({"a": 1}));
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["isError"], false);
        let bad = ToolResult::soft_error("x", "y", json!({}));
        let v = serde_json::to_value(&bad).unwrap();
        assert_eq!(v["isError"], true);
    }
}
