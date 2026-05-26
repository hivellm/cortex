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
    /// Phase11c — the parsed response would have exceeded the MCP
    /// transport's hard cap. Adapter returns this instead of letting
    /// the transport reject the call (which dumps to a side-file).
    pub const BUDGET_EXCEEDED: &str = "budget_exceeded";
    /// Phase14i §2.2 — the tool's `call` body exceeded the
    /// configured timeout (default `MCP_TOOL_TIMEOUT_MS = 5_000`).
    /// `error.data` carries `{ elapsed_ms, tool, request_id? }`.
    pub const TOOL_TIMEOUT: &str = "tool_timeout";
}

/// Phase14i §2.3 — default per-call timeout for every MCP tool.
/// Override per-tool via `cortex_config::McpConfig::tool_timeouts`.
pub const MCP_TOOL_TIMEOUT_MS: u64 = 5_000;

/// Caller name advertised on the `x-cortex-caller` header.
pub const CALLER: &str = "claude-code-plugin";

/// Phase11c — hard cap the MCP transport enforces for a single
/// tool result before it dumps the payload to a side-file. Sized
/// generously above `cortex-api`'s default `budget_bytes` so a
/// daemon clipped to 32 KiB still fits with margin for the JSON-RPC
/// envelope and audit headers added downstream. Bumped together
/// with the upstream transport cap if it ever changes.
pub const MCP_RESPONSE_HARD_CAP: usize = 48 * 1024;

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
    /// Phase14i §2.3 — default per-call timeout used when the
    /// per-tool override is absent. Defaults to
    /// [`MCP_TOOL_TIMEOUT_MS`].
    pub tool_timeout_default: std::time::Duration,
    /// Phase14i §2.3 — per-tool timeout overrides keyed by MCP
    /// tool name. Populated from the env var
    /// `CORTEX_MCP_TOOL_TIMEOUT_MS_{TOOL_NAME}` (the name is
    /// upper-cased + non-alnum collapsed to `_`).
    pub tool_timeouts: std::collections::BTreeMap<String, std::time::Duration>,
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
            tool_timeout_default: std::time::Duration::from_millis(MCP_TOOL_TIMEOUT_MS),
            tool_timeouts: std::collections::BTreeMap::new(),
        }
    }

    /// Phase14i §2.3 — resolve the timeout for `tool_name`. Falls
    /// back to [`Self::tool_timeout_default`] when no override is
    /// configured.
    pub fn tool_timeout(&self, tool_name: &str) -> std::time::Duration {
        self.tool_timeouts
            .get(tool_name)
            .copied()
            .unwrap_or(self.tool_timeout_default)
    }

    /// Phase14i §2.3 — register a per-tool timeout override. Tests
    /// use this to inject short timeouts; production wiring reads
    /// from `cortex_config::McpConfig` once it ships.
    pub fn with_tool_timeout(
        mut self,
        tool_name: impl Into<String>,
        dur: std::time::Duration,
    ) -> Self {
        self.tool_timeouts.insert(tool_name.into(), dur);
        self
    }

    /// Phase14i §2.3 — override the default timeout. Builder form
    /// so production wiring chains it after `new`.
    pub fn with_default_tool_timeout(mut self, dur: std::time::Duration) -> Self {
        self.tool_timeout_default = dur;
        self
    }

    /// Test-friendly constructor that takes an explicit client.
    pub fn with_client(api_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            api_url: api_url.into(),
            http,
            pt_metrics: Arc::new(PreThinkingMetrics::new()),
            started_at: std::time::Instant::now(),
            tool_timeout_default: std::time::Duration::from_millis(MCP_TOOL_TIMEOUT_MS),
            tool_timeouts: std::collections::BTreeMap::new(),
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

    /// Phase14i §2.2 — the tool's `call` body exceeded the
    /// configured timeout. The dispatcher renders the wire shape as
    /// `error.code = -32603` + `error.data = { reason: "tool_timeout",
    /// elapsed_ms, tool, request_id? }`. `tool` MUST match the
    /// MCP tool name so callers can pattern-match per-tool.
    pub fn timeout(tool: impl Into<String>, elapsed_ms: u64) -> Self {
        let tool = tool.into();
        Self {
            reason: reasons::TOOL_TIMEOUT.into(),
            message: format!("tool `{tool}` exceeded timeout after {elapsed_ms} ms"),
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
                Arc::new(ForgetTool::new()),
                Arc::new(KeywordSearchTool::new()),
                Arc::new(VectorSearchTool::new()),
                Arc::new(GraphQueryTool::new()),
                Arc::new(ActiveWorkTool::new()),
                Arc::new(SimilarSessionsTool::new()),
                Arc::new(DecisionChainTool::new()),
                Arc::new(EventsByKindTool::new()),
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
            // Phase11c — even after the daemon-side clipper, the
            // serialised payload could exceed the MCP transport's
            // hard cap (the runtime today rejects ~30k chars). Count
            // the wire bytes and, on overflow, return a structured
            // `budget_exceeded` soft-error carrying enough context
            // for the caller to retry with a smaller `budget_bytes`
            // — instead of letting the transport drop the call to
            // a side-file the caller has to re-read by hand.
            let payload = serde_json::to_value(&parsed).unwrap_or(Value::Null);
            let payload_bytes = serde_json::to_vec(&payload).map(|v| v.len()).unwrap_or(0);
            if payload_bytes > MCP_RESPONSE_HARD_CAP {
                let total_hits = parsed.results.snippets.len()
                    + parsed.results.decisions.len()
                    + parsed.results.violations.len()
                    + parsed.results.similar_turns.len()
                    + parsed.results.graph_neighbors.len();
                let suggested = MCP_RESPONSE_HARD_CAP.saturating_mul(2) / 3;
                return Ok(ToolResult::soft_error(
                    reasons::BUDGET_EXCEEDED,
                    &format!(
                        "cortex_query response is {payload_bytes} bytes — \
                         exceeds the MCP transport cap of {MCP_RESPONSE_HARD_CAP} bytes. \
                         Retry with a smaller `budget_bytes` (suggested: {suggested})."
                    ),
                    json!({
                        "total_hits": total_hits,
                        "payload_bytes": payload_bytes,
                        "transport_cap_bytes": MCP_RESPONSE_HARD_CAP,
                        "suggested_budget_bytes": suggested,
                    }),
                ));
            }
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
        if parsed.body.len() > MAX_CAPTURE_BODY_BYTES {
            return Ok(ToolResult::soft_error(
                "body_too_large",
                "capture body exceeds the 8 KiB ceiling",
                json!({
                    "max_bytes": MAX_CAPTURE_BODY_BYTES,
                    "received": parsed.body.len(),
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

// ---------------------------------------------------------------------
// Phase11t §2 — cortex_forget
// ---------------------------------------------------------------------

/// Wraps `POST <api_url>/v1/admin/forget` (phase11t §1.4) so an MCP
/// host can drive the irreversible cascade across Vectorizer +
/// Meili + Nexus + Parquet for one event id. Requires the caller
/// to echo the canonical confirmation token from
/// [`cortex_workers::pruner::purge::REQUIRED_CONFIRMATION_TOKEN`].
pub struct ForgetTool;

impl ForgetTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ForgetTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ForgetTool {
    fn name(&self) -> &'static str {
        "cortex_forget"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_forget",
            "description": "Hard-purge one event id across Vectorizer + Meili + Nexus + Parquet archive. Irreversible. Requires the caller to echo the canonical confirmation token (see REQUIRED_CONFIRMATION_TOKEN) — a missing or wrong token returns invalid_input. Pass dry_run=true to preview the cascade without invoking it.",
            "inputSchema": {
                "type": "object",
                "required": ["event_id", "confirmation_token"],
                "properties": {
                    "event_id": {
                        "type": "string",
                        "description": "Canonical 26-char ULID of the event to forget."
                    },
                    "confirmation_token": {
                        "type": "string",
                        "description": "Must match REQUIRED_CONFIRMATION_TOKEN verbatim."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "When true, returns the projected cascade plan without invoking it. Defaults to false.",
                        "default": false
                    }
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let url = format!("{}/v1/admin/forget", ctx.api_url.trim_end_matches('/'));
        let resp = match ctx
            .http
            .post(&url)
            .header(cortex_api::CALLER_HEADER, CALLER)
            .json(&args)
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
            let payload: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
            return Ok(ToolResult::ok(payload));
        }
        // Map the cortex-api 400 (missing/wrong confirmation token)
        // to a `ToolError::invalid_input` so the MCP host surfaces
        // the irreversibility advisory at the same layer it would
        // reject any other malformed call.
        if status.as_u16() == 400 {
            let err: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("missing or wrong confirmation_token");
            return Err(ToolError::invalid_input(msg.to_string()));
        }
        // Any other non-success surfaces as a soft-error so the
        // MCP host renders it without aborting the session.
        Ok(ToolResult::soft_error(
            "purge_cascade_failed",
            &format!("cortex-api {url} returned HTTP {status}"),
            serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({})),
        ))
    }
}

// ----------------------------------------------------------------------
// Phase11v — fine-grained per-backend search tools.
// ----------------------------------------------------------------------

/// MCP wrapper for `POST <api_url>/v1/search/keyword`. Forwards the
/// body verbatim and returns the raw cortex-api response payload.
pub struct KeywordSearchTool;

impl KeywordSearchTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for KeywordSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for KeywordSearchTool {
    fn name(&self) -> &'static str {
        "cortex_keyword_search"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_keyword_search",
            "description": "Raw Meilisearch keyword search against a named index. Bypasses the cortex_query fusion lane — returns the literal Meili hits with `processing_time_ms` and `estimated_total_hits`. Useful for inspecting what a specific index actually contains, debugging recall, or grepping decisions/consolidations/turns directly.",
            "inputSchema": {
                "type": "object",
                "required": ["index"],
                "properties": {
                    "index": {
                        "type": "string",
                        "description": "Meili index uid, e.g. `cortex_consolidations`, `cortex_decisions`, `cortex-cortex-turns`."
                    },
                    "q": {"type": "string", "description": "Free-text query. Empty string returns the first `limit` documents."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10},
                    "filter": {"type": "string", "description": "Raw Meili filter expression (e.g. `repo = \"cortex\"`)."},
                    "sort": {"type": "array", "items": {"type": "string"}, "description": "Meili sort expression list."},
                    "attributes_to_retrieve": {"type": "array", "items": {"type": "string"}, "description": "When set, restrict per-hit fields to this list."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        proxy_search(ctx, "/v1/search/keyword", args).await
    }
}

/// MCP wrapper for `POST <api_url>/v1/search/vector`.
pub struct VectorSearchTool;

impl VectorSearchTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for VectorSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for VectorSearchTool {
    fn name(&self) -> &'static str {
        "cortex_vector_search"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_vector_search",
            "description": "Raw Vectorizer cosine search against a named collection. Bypasses the cortex_query fusion lane — returns the upstream hits as Vectorizer surfaces them. v1 accepts `query_vector` (caller-supplied embedding); server-side embedding via `query_text` lands in a follow-up.",
            "inputSchema": {
                "type": "object",
                "required": ["collection", "query_vector"],
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Vectorizer collection name, e.g. `cortex.consolidation.fp32`, `cortex-cortex-turns`."
                    },
                    "query_vector": {
                        "type": "array",
                        "items": {"type": "number"},
                        "description": "Raw f32 embedding."
                    },
                    "k": {"type": "integer", "minimum": 1, "maximum": 200, "default": 10},
                    "score_threshold": {"type": "number", "description": "Optional cosine-score floor — drops hits below this value."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        proxy_search(ctx, "/v1/search/vector", args).await
    }
}

/// MCP wrapper for `POST <api_url>/v1/search/graph`.
pub struct GraphQueryTool;

impl GraphQueryTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for GraphQueryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn name(&self) -> &'static str {
        "cortex_graph_query"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_graph_query",
            "description": "Direct Nexus graph query. Two modes: `neighbors` walks 1..5 hops from a seed node id with optional edge-kind filter; `cypher` runs a raw Cypher statement (gated on `CORTEX_GRAPH_CYPHER_ENABLED=1` server-side). Returns nodes + edges as JSON. Use this to inspect relationships and graph topology that the fused cortex_query hides.",
            "inputSchema": {
                "type": "object",
                "required": ["mode"],
                "properties": {
                    "mode": {"type": "string", "enum": ["neighbors", "cypher"]},
                    "node_id": {"type": "string", "description": "Required for `neighbors` mode."},
                    "depth": {"type": "integer", "minimum": 1, "maximum": 5, "default": 1, "description": "neighbors hop limit."},
                    "edge_kinds": {"type": "array", "items": {"type": "string"}, "description": "Optional edge-kind filter for `neighbors` (e.g. [\"EMITTED\"])."},
                    "statement": {"type": "string", "description": "Required for `cypher` mode."},
                    "parameters": {"type": "object", "description": "Optional Cypher bind parameters."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        proxy_search(ctx, "/v1/search/graph", args).await
    }
}

/// MCP wrapper for `GET <api_url>/v1/dashboard/active-work` (phase13g
/// §1.4). Reads `.rulebook/tasks/` + `.rulebook/archive/` via the
/// `cortex-api` daemon and exposes the operator-state snapshot so
/// the agent can ground its session on what is already in-flight
/// without re-walking the filesystem.
pub struct ActiveWorkTool;

impl ActiveWorkTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ActiveWorkTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Server-side hard cap on the number of `active_tasks` rows the
/// MCP tool surfaces. The daemon may return more; the MCP wrapper
/// trims to keep the envelope under [`MCP_RESPONSE_HARD_CAP`]
/// without forcing the dashboard endpoint to also clamp.
pub const ACTIVE_WORK_TASK_CAP: usize = 50;

#[async_trait]
impl Tool for ActiveWorkTool {
    fn name(&self) -> &'static str {
        "cortex_active_work"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_active_work",
            "description": "Operator-state snapshot — returns the rulebook tasks under `.rulebook/tasks/` plus a recent-archives tail. Each row carries id, phase, status, the next unchecked checklist item, and (when status='blocked') a blocked_reason. Optional `repo` filter scopes the snapshot to one workspace. Reads are cached for 60 s; the response is capped server-side to keep the MCP envelope small.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": {"type": "string", "description": "Optional repo slug (case-insensitive) — when set, only tasks whose workspace parent matches are returned."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let repo = args
            .get("repo")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        // Validate `repo` is slug-safe before splicing into the URL —
        // dashboard slugs are lowercase alnum + `-` / `_`; anything
        // else is a client mistake the MCP layer rejects.
        if let Some(slug) = repo.as_deref() {
            if slug.is_empty()
                || !slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(ToolError::invalid_input(
                    "`repo` must be a non-empty slug of [a-zA-Z0-9_-]",
                ));
            }
        }
        let mut path = String::from("/v1/dashboard/active-work");
        if let Some(ref slug) = repo {
            path.push_str("?repo=");
            path.push_str(slug);
        }
        let mut value = match proxy_get(ctx, &path).await? {
            ToolResult {
                content,
                is_error: false,
            } => {
                let text = content
                    .first()
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string();
                serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({}))
            }
            err @ ToolResult { is_error: true, .. } => return Ok(err),
        };
        // Cap active_tasks at ACTIVE_WORK_TASK_CAP. The daemon side
        // already truncated `next_unchecked_item` to 256 bytes.
        if let Some(arr) = value.get_mut("active_tasks").and_then(|v| v.as_array_mut()) {
            if arr.len() > ACTIVE_WORK_TASK_CAP {
                arr.truncate(ACTIVE_WORK_TASK_CAP);
            }
        }
        Ok(ToolResult::ok(value))
    }
}

/// MCP wrapper for `POST <api_url>/v1/search/similar-sessions`
/// (phase13g §2.4). Returns up to `k` consolidation hits whose
/// score clears the supplied confidence floor. Server-side enforces
/// `1 <= k <= 10`.
pub struct SimilarSessionsTool;

impl SimilarSessionsTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimilarSessionsTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Hard cap on `k` enforced by the MCP layer in addition to the
/// daemon-side clamp. Mirrors the spec's "max 10".
pub const SIMILAR_SESSIONS_MAX_K: u64 = 10;

#[async_trait]
impl Tool for SimilarSessionsTool {
    fn name(&self) -> &'static str {
        "cortex_similar_sessions"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_similar_sessions",
            "description": "Vector search restricted to the `cortex-<repo>-consolidations` collection. Returns top-K past sessions whose summary embeds close to `query`. Use this when the agent needs deterministic grounding on prior work (e.g. \"show me sessions that already tackled rework analysis\") instead of constructing a fusion query. Confidence floor defaults to 0.6 — sub-threshold hits drop server-side.",
            "inputSchema": {
                "type": "object",
                "required": ["query", "repo"],
                "properties": {
                    "query": {"type": "string", "description": "Free-text query the lane embeds + searches against the consolidation collection."},
                    "repo": {"type": "string", "description": "Repo slug (case-insensitive). Scopes the search to `cortex-<repo>-consolidations`."},
                    "k": {"type": "integer", "minimum": 1, "maximum": 10, "default": 5, "description": "Top-K. Clamped server-side."},
                    "confidence_floor": {"type": "number", "minimum": 0.0, "maximum": 1.0, "default": 0.6, "description": "Score cutoff — hits below this drop before the response is built."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        // Mirror the MCP-side k cap. The daemon also clamps; this
        // gate catches obvious caller bugs before the round-trip.
        if let Some(k) = args.get("k").and_then(|v| v.as_u64()) {
            if k == 0 || k > SIMILAR_SESSIONS_MAX_K {
                return Err(ToolError::invalid_input(format!(
                    "`k` must be in [1, {SIMILAR_SESSIONS_MAX_K}]"
                )));
            }
        }
        if let Some(floor) = args.get("confidence_floor").and_then(|v| v.as_f64()) {
            if !(0.0..=1.0).contains(&floor) {
                return Err(ToolError::invalid_input(
                    "`confidence_floor` must be in [0.0, 1.0]",
                ));
            }
        }
        proxy_search(ctx, "/v1/search/similar-sessions", args).await
    }
}

/// MCP wrapper for `GET <api_url>/v1/search/decision-chain`
/// (phase13g §3.4). Walks `:SUPERSEDES` edges in both directions
/// from a `Decision` ULID, returning the merged chain with
/// per-direction hop counts. Server-side enforces `max_hops <= 16`
/// and ULID-shaped `event_id`.
pub struct DecisionChainTool;

impl DecisionChainTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DecisionChainTool {
    fn default() -> Self {
        Self::new()
    }
}

/// MCP-side ceiling on `max_hops`. Mirrors the daemon-side cap so
/// obvious caller bugs surface before the round-trip.
pub const DECISION_CHAIN_MAX_HOPS: u64 = 16;

#[async_trait]
impl Tool for DecisionChainTool {
    fn name(&self) -> &'static str {
        "cortex_decision_chain"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_decision_chain",
            "description": "Walk `:SUPERSEDES` edges from a Decision node in both directions and return the chronological chain. Use this when the agent needs to know which ADR a current ADR replaces, or which later ADRs replace it. `event_id` must be a 26-char ULID; `max_hops` bounds the walk (default 16, max 16).",
            "inputSchema": {
                "type": "object",
                "required": ["event_id"],
                "properties": {
                    "event_id": {"type": "string", "description": "Decision node ULID — 26 chars from [0-9A-Z] (Crockford-safe subset)."},
                    "max_hops": {"type": "integer", "minimum": 1, "maximum": 16, "default": 16, "description": "Path-length cap per direction."}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        let event_id = args
            .get("event_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| ToolError::invalid_input("`event_id` is required"))?;
        if !is_ulid_safe(&event_id) {
            return Err(ToolError::invalid_input(
                "`event_id` must match [0-9A-Z]{26}",
            ));
        }
        if let Some(h) = args.get("max_hops").and_then(|v| v.as_u64()) {
            if h == 0 || h > DECISION_CHAIN_MAX_HOPS {
                return Err(ToolError::invalid_input(format!(
                    "`max_hops` must be in [1, {DECISION_CHAIN_MAX_HOPS}]"
                )));
            }
        }
        let mut path = format!("/v1/search/decision-chain?event_id={event_id}");
        if let Some(h) = args.get("max_hops").and_then(|v| v.as_u64()) {
            path.push_str(&format!("&max_hops={h}"));
        }
        proxy_get(ctx, &path).await
    }
}

/// 26-char Crockford-safe ULID regex, hand-rolled so the MCP layer
/// doesn't pull a new dep.
fn is_ulid_safe(s: &str) -> bool {
    if s.len() != 26 {
        return false;
    }
    s.bytes().all(|b| {
        matches!(
            b,
            b'0'..=b'9'
                | b'A'..=b'H'
                | b'J'
                | b'K'
                | b'M'
                | b'N'
                | b'P'..=b'T'
                | b'V'..=b'Z'
        )
    })
}

// ---------------------------------------------------------------------
// phase19 §1.1 — cortex_events_by_kind
// ---------------------------------------------------------------------

/// MCP wrapper for `POST <api_url>/v1/search/events`.
///
/// Granular envelope listing filtered by `kind` (Turn / ToolCall /
/// Consolidation / Decision / KnowledgeNote / LearningNote /
/// TopicCard / Law / Memory / Analysis). Reads from the kind-routed
/// Meili index so the response carries the native envelope shape —
/// no fusion projection.
pub struct EventsByKindTool;

impl EventsByKindTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for EventsByKindTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EventsByKindTool {
    fn name(&self) -> &'static str {
        "cortex_events_by_kind"
    }

    fn descriptor(&self) -> Value {
        json!({
            "name": "cortex_events_by_kind",
            "description": "List envelopes filtered by `kind` (turn / tool_call / consolidation / decision / analysis / memory / law / knowledge / learning / topic_card). Reads directly from the kind-routed Meili index — returns the native envelope shape, NOT a fusion projection. Use this when `cortex_query` would post-filter and drop candidates.",
            "inputSchema": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": [
                            "turn", "tool_call", "consolidation", "decision",
                            "analysis", "memory", "law", "violation",
                            "knowledge", "learning", "topic_card"
                        ],
                        "description": "Envelope kind discriminator. Accepts both snake_case (`tool_call`) and PascalCase (`ToolCall`)."
                    },
                    "repo": {"type": "string", "description": "Optional `source.repo` filter."},
                    "session_id": {"type": "string", "description": "Optional `session_id` filter."},
                    "since": {"type": "string", "description": "RFC3339 lower bound on `occurred_at`."},
                    "until": {"type": "string", "description": "RFC3339 upper bound on `occurred_at`."},
                    "q": {"type": "string", "description": "Free-text Meili query. Empty string returns every match."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20}
                }
            }
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult, ToolError> {
        proxy_search(ctx, "/v1/search/events", args).await
    }
}

/// GET-mode sibling of [`proxy_search`]. Reads the response body
/// verbatim into a `ToolResult`.
async fn proxy_get(ctx: &ToolContext, path: &str) -> Result<ToolResult, ToolError> {
    let url = format!("{}{}", ctx.api_url.trim_end_matches('/'), path);
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
    let body_bytes = resp.bytes().await.unwrap_or_default();
    if status.is_success() {
        let payload: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
        return Ok(ToolResult::ok(payload));
    }
    let err: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
    let reason = err
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or(reasons::API_HTTP_ERROR)
        .to_string();
    Ok(ToolResult::soft_error(
        &reason,
        &format!("cortex-api {url} returned HTTP {status}"),
        err,
    ))
}

/// Shared helper — POST `<api_url><path>` with the args body, fold
/// the cortex-api response into a `ToolResult`. 2xx → ok with the
/// JSON body; 400/403/404 → soft-error keyed on the upstream `reason`;
/// other failures → `ToolResult::soft_error`.
async fn proxy_search(ctx: &ToolContext, path: &str, args: Value) -> Result<ToolResult, ToolError> {
    let url = format!("{}{}", ctx.api_url.trim_end_matches('/'), path);
    let resp = match ctx
        .http
        .post(&url)
        .header(cortex_api::CALLER_HEADER, CALLER)
        .json(&args)
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
        let payload: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
        return Ok(ToolResult::ok(payload));
    }
    let err: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| json!({}));
    let reason = err
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or(reasons::API_HTTP_ERROR)
        .to_string();
    Ok(ToolResult::soft_error(
        &reason,
        &format!("cortex-api {url} returned HTTP {status}"),
        err,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_returns_fourteen_tools_with_unique_names() {
        let reg = ToolRegistry::default_set();
        assert_eq!(
            reg.len(),
            14,
            "phase19 §1.1 adds cortex_events_by_kind (13 -> 14)"
        );
        let names: Vec<&str> = reg.tools.iter().map(|t| t.name()).collect();
        for expected in [
            "cortex_query",
            "cortex_pre_thinking",
            "cortex_status",
            "cortex_audit",
            "cortex_capture_memory",
            "cortex_session_replay",
            "cortex_forget",
            "cortex_keyword_search",
            "cortex_vector_search",
            "cortex_graph_query",
            "cortex_active_work",
            "cortex_similar_sessions",
            "cortex_decision_chain",
            "cortex_events_by_kind",
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
    fn active_work_descriptor_advertises_optional_repo_filter() {
        let t = ActiveWorkTool::new();
        let d = t.descriptor();
        assert_eq!(d["name"], "cortex_active_work");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        let props = d["inputSchema"]["properties"].as_object().unwrap();
        assert!(props.contains_key("repo"));
        // `repo` is optional — no `required` array on this tool.
        assert!(d["inputSchema"].get("required").is_none());
    }

    #[tokio::test]
    async fn active_work_rejects_repo_with_special_chars() {
        let t = ActiveWorkTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(&ctx, json!({ "repo": "../etc/passwd" }))
            .await
            .expect_err("invalid slug must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[test]
    fn similar_sessions_descriptor_requires_query_and_repo() {
        let t = SimilarSessionsTool::new();
        let d = t.descriptor();
        assert_eq!(d["name"], "cortex_similar_sessions");
        assert!(
            d.get("input_schema").is_none(),
            "snake_case input_schema must not be emitted"
        );
        let req = d["inputSchema"]["required"].as_array().unwrap();
        let names: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"query"));
        assert!(names.contains(&"repo"));
        // k + confidence_floor stay optional with documented defaults.
        let props = d["inputSchema"]["properties"].as_object().unwrap();
        assert_eq!(props["k"]["maximum"].as_u64().unwrap(), 10);
        assert_eq!(props["k"]["default"].as_u64().unwrap(), 5);
    }

    #[tokio::test]
    async fn similar_sessions_rejects_k_above_max() {
        let t = SimilarSessionsTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(&ctx, json!({ "query": "x", "repo": "cortex", "k": 50 }))
            .await
            .expect_err("k > 10 must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn similar_sessions_rejects_zero_k() {
        let t = SimilarSessionsTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(&ctx, json!({ "query": "x", "repo": "cortex", "k": 0 }))
            .await
            .expect_err("k=0 must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[test]
    fn decision_chain_descriptor_requires_event_id_only() {
        let t = DecisionChainTool::new();
        let d = t.descriptor();
        assert_eq!(d["name"], "cortex_decision_chain");
        assert!(d.get("input_schema").is_none());
        let req = d["inputSchema"]["required"].as_array().unwrap();
        let names: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["event_id"]);
        assert_eq!(
            d["inputSchema"]["properties"]["max_hops"]["maximum"]
                .as_u64()
                .unwrap(),
            16
        );
    }

    #[tokio::test]
    async fn decision_chain_rejects_invalid_ulid() {
        let t = DecisionChainTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(&ctx, json!({ "event_id": "not-a-ulid" }))
            .await
            .expect_err("non-ULID must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn decision_chain_rejects_max_hops_over_ceiling() {
        let t = DecisionChainTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(
                &ctx,
                json!({
                    "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                    "max_hops": 100,
                }),
            )
            .await
            .expect_err("max_hops > 16 must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn similar_sessions_rejects_floor_outside_unit_interval() {
        let t = SimilarSessionsTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(
                &ctx,
                json!({ "query": "x", "repo": "cortex", "confidence_floor": 1.5 }),
            )
            .await
            .expect_err("floor > 1.0 must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
    }

    #[tokio::test]
    async fn active_work_rejects_empty_repo_string() {
        let t = ActiveWorkTool::new();
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let err = t
            .call(&ctx, json!({ "repo": "" }))
            .await
            .expect_err("empty slug must be rejected");
        assert_eq!(err.reason, reasons::INVALID_INPUT);
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

    // ---------------------------------------------------------------
    // Phase11t §3 — ForgetTool
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn forget_tool_rejects_missing_confirmation_token() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/admin/forget"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "reason": "missing_confirmation_token",
                "message": "missing or wrong confirmation_token (expected \"I-UNDERSTAND-FORGET-IS-IRREVERSIBLE\")",
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = ForgetTool::new();
        let err = tool
            .call(
                &ctx,
                json!({
                    "event_id": "EVT",
                    "confirmation_token": "wrong",
                }),
            )
            .await
            .expect_err("must reject");
        assert!(
            err.message.contains("confirmation_token"),
            "expected token-mismatch message; got {:?}",
            err.message
        );
    }

    // -- Phase 11v: fine-grained search MCP wrappers ---------------

    #[tokio::test]
    async fn keyword_search_tool_round_trips_meili_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/keyword"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "index": "cortex_consolidations",
                "hits": [
                    { "id": "01H1", "title": "consolidation a" },
                    { "id": "01H2", "title": "consolidation b" }
                ],
                "processing_time_ms": 4,
                "estimated_total_hits": 12
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = KeywordSearchTool::new();
        let res = tool
            .call(
                &ctx,
                json!({ "index": "cortex_consolidations", "q": "phase11v", "limit": 5 }),
            )
            .await
            .expect("happy-path must succeed");
        assert!(!res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["index"], "cortex_consolidations");
        assert_eq!(payload["hits"].as_array().unwrap().len(), 2);
        assert_eq!(payload["processing_time_ms"], 4);
        assert_eq!(payload["estimated_total_hits"], 12);
    }

    #[tokio::test]
    async fn keyword_search_tool_surfaces_index_not_found_as_soft_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/keyword"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "reason": "index_not_found",
                "message": "index `cortex_missing` not found in Meili",
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = KeywordSearchTool::new();
        let res = tool
            .call(&ctx, json!({ "index": "cortex_missing", "q": "x" }))
            .await
            .expect("404 must arrive as soft-error, not panic");
        assert!(res.is_error, "non-2xx must be a soft-error");
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["reason"], "index_not_found");
    }

    #[tokio::test]
    async fn keyword_search_tool_returns_api_unreachable_when_no_listener() {
        // No server: bind to a port that has nothing listening so
        // reqwest fails connect and the tool produces the canonical
        // `cortex-api unreachable` soft-error envelope.
        let ctx = ToolContext::new("http://127.0.0.1:1");
        let tool = KeywordSearchTool::new();
        let res = tool
            .call(&ctx, json!({ "index": "cortex_consolidations", "q": "x" }))
            .await
            .expect("transport failure must arrive as soft-error, not panic");
        assert!(res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["reason"], reasons::API_UNREACHABLE);
    }

    #[tokio::test]
    async fn vector_search_tool_round_trips_vectorizer_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/vector"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "collection": "cortex.consolidation.fp32",
                "hits": [
                    { "id": "01V1", "score": 0.91, "payload": {"kind":"consolidation"} }
                ],
                "upstream_latency_ms": 23
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = VectorSearchTool::new();
        let res = tool
            .call(
                &ctx,
                json!({
                    "collection": "cortex.consolidation.fp32",
                    "query_vector": [0.1, 0.2, 0.3],
                    "k": 5
                }),
            )
            .await
            .expect("happy path must succeed");
        assert!(!res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["collection"], "cortex.consolidation.fp32");
        assert_eq!(payload["hits"].as_array().unwrap().len(), 1);
        assert_eq!(payload["upstream_latency_ms"], 23);
    }

    #[tokio::test]
    async fn vector_search_tool_surfaces_bad_input_as_soft_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/vector"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "reason": "bad_input",
                "message": "exactly one of `query_vector` or `query_text` is required",
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = VectorSearchTool::new();
        let res = tool
            .call(&ctx, json!({ "collection": "cortex.consolidation.fp32" }))
            .await
            .expect("400 must arrive as soft-error");
        assert!(res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["reason"], "bad_input");
    }

    #[tokio::test]
    async fn vector_search_tool_surfaces_budget_exceeded_envelope() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // The cortex-api `ok_capped` helper returns HTTP 413 with an
        // `error` field rather than a `reason` field. The MCP wrapper
        // surfaces the envelope as a soft-error keyed on the
        // upstream HTTP status (no `reason` → fallback label).
        Mock::given(method("POST"))
            .and(path("/v1/search/vector"))
            .respond_with(ResponseTemplate::new(413).set_body_json(json!({
                "error": "budget_exceeded",
                "payload_bytes": 41201,
                "transport_cap_bytes": 30720,
                "suggested_limit": "reduce `k`"
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = VectorSearchTool::new();
        let res = tool
            .call(
                &ctx,
                json!({
                    "collection": "cortex.consolidation.fp32",
                    "query_vector": [0.1, 0.2],
                    "k": 200
                }),
            )
            .await
            .expect("413 must arrive as soft-error");
        assert!(res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        // soft_error wraps the upstream body under `details`; the
        // MCP wrapper falls back to `API_HTTP_ERROR` for the soft-
        // error `reason` because the cortex-api 413 envelope uses
        // `error` instead of `reason`.
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["reason"], reasons::API_HTTP_ERROR);
        assert_eq!(payload["details"]["error"], "budget_exceeded");
        assert_eq!(payload["details"]["payload_bytes"], 41201);
    }

    #[tokio::test]
    async fn graph_query_tool_round_trips_neighbors_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/graph"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "mode": "neighbors",
                "nodes": [
                    { "node_id": "N1", "labels": ["Turn"], "properties": {} },
                    { "node_id": "N2", "labels": ["Decision"], "properties": {} }
                ],
                "edges": [
                    { "from": "N1", "to": "N2", "kind": "REMEMBERS", "properties": {} }
                ]
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = GraphQueryTool::new();
        let res = tool
            .call(
                &ctx,
                json!({ "mode": "neighbors", "node_id": "N1", "depth": 2 }),
            )
            .await
            .expect("neighbors happy path must succeed");
        assert!(!res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["mode"], "neighbors");
        assert_eq!(payload["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(payload["edges"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn graph_query_tool_surfaces_cypher_disabled_as_soft_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/graph"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "reason": "cypher_disabled",
                "message": "raw Cypher mode requires `CORTEX_GRAPH_CYPHER_ENABLED=1`",
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = GraphQueryTool::new();
        let res = tool
            .call(
                &ctx,
                json!({
                    "mode": "cypher",
                    "statement": "MATCH (n) RETURN n LIMIT 1"
                }),
            )
            .await
            .expect("403 must arrive as soft-error");
        assert!(res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["reason"], "cypher_disabled");
    }

    #[tokio::test]
    async fn graph_query_tool_round_trips_cypher_payload_when_enabled_upstream() {
        // The MCP wrapper does NOT inspect the env var — it simply
        // forwards the request and surfaces whatever cortex-api
        // returns. This test pins that contract by mocking a
        // successful Cypher-mode response.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search/graph"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "mode": "cypher",
                "nodes": [],
                "edges": []
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = GraphQueryTool::new();
        let res = tool
            .call(
                &ctx,
                json!({
                    "mode": "cypher",
                    "statement": "MATCH (n:Decision) RETURN n LIMIT 1"
                }),
            )
            .await
            .expect("cypher happy path must succeed");
        assert!(!res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["mode"], "cypher");
        assert!(payload["nodes"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn forget_tool_dry_run_round_trips_projection() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/admin/forget"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "event_id": "EVT",
                "vectorizer_collections": [
                    "cortex.consolidation.fp32",
                    "cortex.consolidation.pq",
                    "cortex.cold.binary",
                ],
                "dry_run": true,
                "vectors_removed": 0,
                "meili_deleted": false,
                "nexus_deleted": false,
                "archive_rewritten": false,
            })))
            .mount(&server)
            .await;
        let ctx = ToolContext::new(server.uri());
        let tool = ForgetTool::new();
        let res = tool
            .call(
                &ctx,
                json!({
                    "event_id": "EVT",
                    "confirmation_token": "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE",
                    "dry_run": true,
                }),
            )
            .await
            .expect("dry-run must succeed");
        assert!(!res.is_error);
        let text = res.content[0].get("text").and_then(|v| v.as_str()).unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["meili_deleted"], false);
        assert_eq!(payload["nexus_deleted"], false);
        assert_eq!(payload["archive_rewritten"], false);
    }
}
