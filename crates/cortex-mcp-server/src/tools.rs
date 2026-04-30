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
    /// Base URL for the Cortex daemon (`http://127.0.0.1:17000` by default).
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
    /// Build a registry with the six Cortex tools wired up. The
    /// retrieval/health trio (`cortex_query`, `cortex_pre_thinking`,
    /// `cortex_status`) plus the phase10j audit/capture/replay trio
    /// (`cortex_audit`, `cortex_capture_memory`,
    /// `cortex_session_replay`).
    pub fn default_set() -> Self {
        Self {
            tools: vec![
                Arc::new(QueryTool::new()),
                Arc::new(PreThinkingTool::new()),
                Arc::new(StatusTool::new()),
                Arc::new(AuditTool::new()),
                Arc::new(CaptureMemoryTool::new()),
                Arc::new(SessionReplayTool::new()),
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
        // Phase6a — inject `x-cortex-cwd` so the daemon's
        // `resolve_scope` can derive `scope.repo` from the
        // basename when the caller did not supply one explicitly.
        // Errors reading `current_dir` are non-fatal — the request
        // still goes out, the daemon then falls back to its own
        // resolution lanes (explicit body, `x-cortex-repo` header,
        // legacy `CORTEX_ALLOW_UNKNOWN_SCOPE` hatch).
        let cwd_header = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string));
        let mut request = ctx
            .http
            .post(&url)
            .header(cortex_api::CALLER_HEADER, CALLER);
        if let Some(cwd) = &cwd_header {
            request = request.header("x-cortex-cwd", cwd);
        }
        let resp = match request.json(&req).send().await {
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

        // When the upstream `QueryResponse` carried a structural
        // notice (the `repo_not_indexed` path added for issue
        // hivellm/cortex#1), surface it as the MCP soft-error reason
        // so the caller can distinguish "scope was never indexed"
        // from "scope is fine but no signal" — both used to land
        // here as `empty_bundle`.
        if out.bundle.is_empty() {
            if let Some(n) = &out.notice {
                return Ok(ToolResult::soft_error(
                    &n.code,
                    &n.message,
                    json!({
                        "intent": out.intent.label(),
                        "fail_open": out.fail_open,
                        "latency_ms": out.latency_ms,
                        "hint": n.hint,
                    }),
                ));
            }
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

        let mut payload = json!({
            "bundle": out.bundle,
            "intent": out.intent.label(),
            "query_id": out.query_id,
            "steps_applied": out.steps_applied,
            "latency_ms": out.latency_ms,
            "fail_open": out.fail_open,
        });
        if let Some(n) = out.notice {
            payload["notice"] = json!({
                "code": n.code,
                "message": n.message,
                "hint": n.hint,
            });
        }
        Ok(ToolResult::ok(payload))
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

// ---------------------------------------------------------------------
// cortex_audit (phase10j)
// ---------------------------------------------------------------------

/// Phase10j tool — wraps `GET <api_url>/v1/audit/{query_id}`. The
/// agent calls this when it suspects a retrieval lane returned zero
/// hits and wants to confirm which one. The HTTP endpoint serves the
/// envelope from cortex-api's in-memory ring buffer; a 404 means the
/// daemon was restarted, the envelope aged out, or the request hit a
/// different replica.
pub struct AuditTool;

impl AuditTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AuditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AuditArgs {
    query_id: String,
    #[serde(default)]
    include_samples: bool,
}

#[async_trait]
impl Tool for AuditTool {
    fn name(&self) -> &'static str {
        "cortex_audit"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_audit",
            "description": "Return the audit envelope for a previous cortex_query / cortex_pre_thinking call. Useful when the agent wants to know which retrieval lane returned zero hits before tweaking the query.",
            "inputSchema": {
                "type": "object",
                "required": ["query_id"],
                "properties": {
                    "query_id": {
                        "type": "string",
                        "description": "ULID echoed back on the QueryResponse / pre-thinking bundle."
                    },
                    "include_samples": {
                        "type": "boolean",
                        "description": "Reserved for future use — when true the envelope carries up to 3 sample hits per lane. Today it is a no-op; the in-memory store does not retain samples.",
                        "default": false
                    }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let parsed: AuditArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid_input(format!("audit args: {e}")))?;
        let qid = parsed.query_id.trim();
        if qid.is_empty() {
            return Err(ToolError::invalid_input("query_id must not be empty"));
        }

        let url = format!(
            "{}/v1/audit/{}",
            ctx.api_url.trim_end_matches('/'),
            urlencode(qid),
        );
        let resp = match ctx
            .http
            .get(&url)
            .header(cortex_api::CALLER_HEADER, CALLER)
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
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            let mut payload = body;
            if !parsed.include_samples {
                if let Some(obj) = payload.as_object_mut() {
                    obj.remove("samples");
                }
            }
            return Ok(ToolResult::ok(payload));
        }
        let reason = body
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or(reasons::API_HTTP_ERROR)
            .to_string();
        Ok(ToolResult::soft_error(
            &reason,
            &format!("cortex-api returned {status} for /v1/audit"),
            json!({ "status": status.as_u16(), "body": body, "query_id": qid }),
        ))
    }
}

// ---------------------------------------------------------------------
// cortex_capture_memory (phase10j)
// ---------------------------------------------------------------------

/// Phase10j tool — POSTs `<api_url>/v1/ingest` with a canonical
/// `kind=memory|knowledge|learning` envelope so the live retrieval
/// lane sees the body on the next pre-thinking call.
pub struct CaptureMemoryTool;

impl CaptureMemoryTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CaptureMemoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CaptureArgs {
    kind: String,
    body: String,
    repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    severity: Option<String>,
}

/// MCP-side ceiling for the body. Mirrors
/// [`cortex_api::ingest_proxy::MAX_BODY_BYTES`] so the tool can reject
/// oversized payloads before paying the network round-trip.
const MAX_CAPTURE_BODY_BYTES: usize = 8 * 1024;

#[async_trait]
impl Tool for CaptureMemoryTool {
    fn name(&self) -> &'static str {
        "cortex_capture_memory"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_capture_memory",
            "description": "Capture an in-session fact (memory / knowledge / learning) into the live Cortex retrieval lane via /v1/ingest. The captured body becomes queryable on the next cortex_query free-text search.",
            "inputSchema": {
                "type": "object",
                "required": ["kind", "body", "repo"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["memory", "knowledge", "learning"],
                        "description": "Operator-curated kind. Use `memory` for ad-hoc session facts, `knowledge` for reusable patterns / anti-patterns, `learning` for implementation insights."
                    },
                    "body": {
                        "type": "string",
                        "description": "Free-form body text. Capped at 8 KiB; oversize payloads are rejected with a structured error so the caller can retry with a shorter body."
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repo slug (lowercase per phase10d). Required so the live freshness lane can attribute the envelope back to the correct project."
                    },
                    "topic": {
                        "type": "string",
                        "description": "Optional topic from the canonical taxonomy (e.g. `retrieval`, `auth`, `migration`)."
                    },
                    "severity": {
                        "type": "string",
                        "enum": ["info", "notable"],
                        "description": "Optional severity hint. Defaults to `info` server-side."
                    }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let parsed: CaptureArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid_input(format!("capture args: {e}")))?;
        if parsed.body.is_empty() {
            return Err(ToolError::invalid_input("body must not be empty"));
        }
        if parsed.body.as_bytes().len() > MAX_CAPTURE_BODY_BYTES {
            return Ok(ToolResult::soft_error(
                "body_too_large",
                "capture body exceeds the 8 KiB ceiling",
                json!({
                    "max_bytes": MAX_CAPTURE_BODY_BYTES,
                    "received": parsed.body.as_bytes().len(),
                }),
            ));
        }
        let kind = parsed.kind.to_ascii_lowercase();
        if !matches!(kind.as_str(), "memory" | "knowledge" | "learning") {
            return Err(ToolError::invalid_input(format!(
                "unsupported kind `{}` (expected memory / knowledge / learning)",
                parsed.kind
            )));
        }
        let repo = parsed.repo.trim();
        if repo.is_empty() {
            return Err(ToolError::invalid_input("repo must not be empty"));
        }
        if repo != repo.to_ascii_lowercase() {
            return Err(ToolError::invalid_input(format!(
                "repo `{}` must be lowercase per phase10d",
                repo
            )));
        }

        let url = format!("{}/v1/ingest", ctx.api_url.trim_end_matches('/'));
        let body = serde_json::to_value(&parsed).unwrap_or(Value::Null);
        let resp = match ctx
            .http
            .post(&url)
            .header(cortex_api::CALLER_HEADER, CALLER)
            .json(&body)
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
        let upstream_body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(ToolResult::ok(upstream_body));
        }
        let reason = upstream_body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or(reasons::API_HTTP_ERROR)
            .to_string();
        Ok(ToolResult::soft_error(
            &reason,
            &format!("cortex-api returned {status} for /v1/ingest"),
            json!({ "status": status.as_u16(), "body": upstream_body }),
        ))
    }
}

// ---------------------------------------------------------------------
// cortex_session_replay (phase10j)
// ---------------------------------------------------------------------

/// Phase10j tool — wraps the dashboard conversation detail endpoint
/// so the agent can pull an ordered transcript of an earlier session
/// without leaving the MCP transport.
pub struct SessionReplayTool;

impl SessionReplayTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SessionReplayTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ReplayArgs {
    session_id: String,
    #[serde(default)]
    max_turns: Option<u32>,
    #[serde(default)]
    include_tool_calls: bool,
}

/// Default value the tool applies when `max_turns` is omitted.
const DEFAULT_REPLAY_TURNS: u32 = 20;
/// Hard cap so a misconfigured caller cannot burn the context budget.
const MAX_REPLAY_TURNS: u32 = 200;

#[async_trait]
impl Tool for SessionReplayTool {
    fn name(&self) -> &'static str {
        "cortex_session_replay"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_session_replay",
            "description": "Return the ordered turns for a previous Cortex session. Backed by /v1/dashboard/conversations/{session_id}; max_turns defaults to 20 and is capped at 200 so the bundle stays under context budget.",
            "inputSchema": {
                "type": "object",
                "required": ["session_id"],
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Canonical 26-char ULID echoed by /v1/dashboard/conversations."
                    },
                    "max_turns": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "default": 20
                    },
                    "include_tool_calls": {
                        "type": "boolean",
                        "default": false,
                        "description": "When true, each turn carries a `tool_calls` array (best-effort — the dashboard endpoint exposes the user / assistant text only today)."
                    }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let parsed: ReplayArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid_input(format!("replay args: {e}")))?;
        let sid = parsed.session_id.trim();
        if sid.is_empty() {
            return Err(ToolError::invalid_input("session_id must not be empty"));
        }
        let max_turns = parsed
            .max_turns
            .unwrap_or(DEFAULT_REPLAY_TURNS)
            .clamp(1, MAX_REPLAY_TURNS) as usize;

        let url = format!(
            "{}/v1/dashboard/conversations/{}",
            ctx.api_url.trim_end_matches('/'),
            urlencode(sid),
        );
        let resp = match ctx
            .http
            .get(&url)
            .header(cortex_api::CALLER_HEADER, CALLER)
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
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Ok(ToolResult::soft_error(
                reasons::API_HTTP_ERROR,
                &format!("cortex-api returned {status} for /v1/dashboard/conversations"),
                json!({ "status": status.as_u16(), "body": body }),
            ));
        }

        let session_id = body
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or(sid)
            .to_string();
        let turns_in: Vec<Value> = body
            .get("turns")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let total_turns = turns_in.len();
        let started_at_ms = turns_in
            .first()
            .and_then(|t| t.get("started_at_ms"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let ended_at_ms = turns_in
            .last()
            .and_then(|t| {
                t.get("completed_at_ms")
                    .and_then(Value::as_i64)
                    .or_else(|| t.get("started_at_ms").and_then(Value::as_i64))
            })
            .unwrap_or(0);

        // Cap turns and reshape into the spec-20 envelope.
        let turns: Vec<Value> = turns_in
            .into_iter()
            .take(max_turns)
            .flat_map(|raw| reshape_turn(raw, parsed.include_tool_calls))
            .collect();

        let payload = json!({
            "session_id": session_id,
            "started_at_ms": started_at_ms,
            "ended_at_ms": ended_at_ms,
            "total_turns": total_turns,
            "returned_turns": turns.len(),
            "turns": turns,
        });
        Ok(ToolResult::ok(payload))
    }
}

/// Convert one dashboard turn into two flat rows (user + assistant)
/// so the replay payload stays self-describing for the agent. The
/// dashboard pairs the two halves into one struct; the replay
/// surface flattens them back out so each row carries a `role` the
/// agent can switch on without inferring it from field presence.
fn reshape_turn(raw: Value, include_tool_calls: bool) -> Vec<Value> {
    let turn_id = raw
        .get("turn_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let user_message = raw
        .get("user_message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let assistant_message = raw
        .get("assistant_message")
        .and_then(Value::as_str)
        .map(String::from);
    let started_at_ms = raw
        .get("started_at_ms")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completed_at_ms = raw.get("completed_at_ms").and_then(Value::as_i64);

    let mut rows = Vec::with_capacity(2);
    if !user_message.is_empty() {
        let mut row = json!({
            "turn_id": turn_id,
            "role": "user",
            "occurred_at_ms": started_at_ms,
            "summary": user_message,
        });
        if include_tool_calls {
            row["tool_calls"] = Value::Array(Vec::new());
        }
        rows.push(row);
    }
    if let Some(a) = assistant_message {
        let mut row = json!({
            "turn_id": turn_id,
            "role": "assistant",
            "occurred_at_ms": completed_at_ms.unwrap_or(started_at_ms),
            "summary": a,
        });
        if include_tool_calls {
            row["tool_calls"] = Value::Array(Vec::new());
        }
        rows.push(row);
    }
    rows
}

/// Minimal path-segment encoder. The session_id / query_id values we
/// pass through are ULIDs in practice, but the tool surface accepts
/// arbitrary strings — encode the small set of characters that would
/// otherwise break the URL parser.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_returns_six_tools_with_unique_names() {
        let reg = ToolRegistry::default_set();
        assert_eq!(reg.len(), 6, "phase10j adds audit/capture/replay");
        let names: Vec<&str> = reg.tools.iter().map(|t| t.name()).collect();
        for expected in [
            "cortex_query",
            "cortex_pre_thinking",
            "cortex_status",
            "cortex_audit",
            "cortex_capture_memory",
            "cortex_session_replay",
        ] {
            assert!(
                names.contains(&expected),
                "registry must include {expected}; got {names:?}"
            );
        }
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
    fn audit_tool_descriptor_requires_query_id() {
        let d = AuditTool::new().descriptor();
        assert_eq!(d["name"], "cortex_audit");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        let req = d["inputSchema"]["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "query_id"));
    }

    #[test]
    fn capture_memory_descriptor_lists_required_kind_body_repo() {
        let d = CaptureMemoryTool::new().descriptor();
        assert_eq!(d["name"], "cortex_capture_memory");
        let req: Vec<&str> = d["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(req.contains(&"kind"));
        assert!(req.contains(&"body"));
        assert!(req.contains(&"repo"));
        let kinds = d["inputSchema"]["properties"]["kind"]["enum"]
            .as_array()
            .unwrap();
        for k in ["memory", "knowledge", "learning"] {
            assert!(
                kinds.iter().any(|v| v == k),
                "kind enum must include {k}; got {kinds:?}"
            );
        }
    }

    #[test]
    fn session_replay_descriptor_caps_max_turns() {
        let d = SessionReplayTool::new().descriptor();
        assert_eq!(d["name"], "cortex_session_replay");
        let mt = &d["inputSchema"]["properties"]["max_turns"];
        assert_eq!(mt["minimum"], 1);
        assert_eq!(mt["maximum"], 200);
        assert_eq!(mt["default"], 20);
    }

    #[tokio::test]
    async fn capture_memory_rejects_oversized_body_without_network_call() {
        let tool = CaptureMemoryTool::new();
        // No HTTP listener — unreachable api_url is fine because we
        // expect short-circuit before any send().
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let big = "x".repeat(MAX_CAPTURE_BODY_BYTES + 1);
        let res = tool
            .call(
                &ctx,
                json!({
                    "kind": "memory",
                    "body": big,
                    "repo": "cortex",
                }),
            )
            .await
            .expect("oversize is a soft error, not an invalid_input error");
        assert!(res.is_error);
        let txt = res.content[0]["text"].as_str().unwrap();
        assert!(
            txt.contains("body_too_large"),
            "structured reason should be `body_too_large`; got {txt}"
        );
    }

    #[tokio::test]
    async fn capture_memory_rejects_uppercase_repo() {
        let tool = CaptureMemoryTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = tool
            .call(
                &ctx,
                json!({
                    "kind": "memory",
                    "body": "x",
                    "repo": "Cortex",
                }),
            )
            .await
            .expect_err("uppercase repo must be invalid_input");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn capture_memory_rejects_unsupported_kind() {
        let tool = CaptureMemoryTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = tool
            .call(
                &ctx,
                json!({
                    "kind": "decision",
                    "body": "x",
                    "repo": "cortex",
                }),
            )
            .await
            .expect_err("decision is not in the curated set");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn audit_tool_rejects_empty_query_id() {
        let tool = AuditTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = tool
            .call(&ctx, json!({ "query_id": "  " }))
            .await
            .expect_err("blank query_id must be invalid_input");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn session_replay_rejects_empty_session_id() {
        let tool = SessionReplayTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = tool
            .call(&ctx, json!({ "session_id": "" }))
            .await
            .expect_err("blank session_id must be invalid_input");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[test]
    fn urlencode_round_trips_safe_chars_and_escapes_unsafe_ones() {
        assert_eq!(urlencode("abc-123_~."), "abc-123_~.");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("a b"), "a%20b");
    }

    #[test]
    fn reshape_turn_emits_two_rows_when_both_messages_present() {
        let raw = json!({
            "turn_id": "t1",
            "user_message": "hi",
            "assistant_message": "hello",
            "started_at_ms": 100,
            "completed_at_ms": 200,
        });
        let rows = reshape_turn(raw, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["occurred_at_ms"], 100);
        assert_eq!(rows[1]["role"], "assistant");
        assert_eq!(rows[1]["occurred_at_ms"], 200);
    }

    #[test]
    fn reshape_turn_skips_empty_user_message() {
        // Stop envelopes with empty user_message must NOT inject a
        // blank user row — the agent renders it as a confusing
        // empty bubble.
        let raw = json!({
            "turn_id": "t2",
            "user_message": "",
            "assistant_message": "hello",
            "started_at_ms": 100,
            "completed_at_ms": 200,
        });
        let rows = reshape_turn(raw, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["role"], "assistant");
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
