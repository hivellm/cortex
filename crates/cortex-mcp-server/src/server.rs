//! MCP server state machine.
//!
//! Spec 18 §MCP wire protocol. Handles the four canonical methods:
//!
//! 1. `initialize` → returns capabilities + serverInfo + `protocolVersion`.
//! 2. `notifications/initialized` → acknowledges client readiness.
//! 3. `tools/list` → returns the descriptors from [`ToolRegistry`].
//! 4. `tools/call` → dispatches to the matching tool, catches panics.
//!
//! The dispatcher is transport-agnostic: it consumes raw JSON bytes
//! and emits raw JSON bytes. [`crate::transport_stdio`] glues it to
//! stdin/stdout.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::metrics::Metrics;
use crate::rpc::{
    Inbound, Notification, Request, Response, RpcError, JSONRPC_VERSION, MCP_PROTOCOL_VERSION,
};
use crate::tools::{ToolContext, ToolError, ToolRegistry, ToolResult};

/// Server name advertised in `initialize.serverInfo.name`.
pub const SERVER_NAME: &str = "cortex-mcp-server";
/// Server version — pinned to the crate version at compile time.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot returned in `initialize.result.serverInfo`.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Human-readable server name.
    pub name: &'static str,
    /// Crate version.
    pub version: &'static str,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: SERVER_NAME,
            version: SERVER_VERSION,
        }
    }
}

/// MCP server. Stateful only insofar as it tracks whether the
/// handshake completed; tool dispatch is otherwise stateless.
pub struct Server {
    ctx: ToolContext,
    registry: ToolRegistry,
    metrics: Arc<Metrics>,
}

impl Server {
    /// Build a server with the default tool registry.
    pub fn new(ctx: ToolContext) -> Self {
        Self {
            ctx,
            registry: ToolRegistry::default_set(),
            metrics: Arc::new(Metrics::new()),
        }
    }

    /// Build a server with an explicit registry — used by tests.
    pub fn with_registry(ctx: ToolContext, registry: ToolRegistry) -> Self {
        Self {
            ctx,
            registry,
            metrics: Arc::new(Metrics::new()),
        }
    }

    /// Number of tools registered.
    pub fn tool_count(&self) -> usize {
        self.registry.len()
    }

    /// Read-only metrics handle.
    pub fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }

    /// Handle one frame of raw JSON. Returns the bytes the transport
    /// should write back, or `None` for notifications (no response).
    pub async fn handle_frame(&self, raw: &[u8]) -> Option<Vec<u8>> {
        let inbound: Inbound = match serde_json::from_slice(raw) {
            Ok(v) => v,
            Err(e) => {
                let resp = Response::error(
                    Value::Null,
                    RpcError::parse_error(format!("invalid JSON-RPC frame: {e}")),
                );
                return serde_json::to_vec(&resp).ok();
            }
        };

        match inbound {
            Inbound::Request(req) => {
                let resp = self.dispatch_request(req).await;
                serde_json::to_vec(&resp).ok()
            }
            Inbound::Notification(n) => {
                self.handle_notification(n);
                None
            }
        }
    }

    fn handle_notification(&self, n: Notification) {
        if n.method == "notifications/initialized" {
            self.metrics.incr_handshake();
            info!(method = %n.method, "MCP client signalled ready");
        } else {
            warn!(method = %n.method, "ignoring unknown notification");
        }
    }

    async fn dispatch_request(&self, req: Request) -> Response {
        if req.jsonrpc != JSONRPC_VERSION {
            return Response::error(
                req.id,
                RpcError::invalid_request(format!("unexpected jsonrpc version: {}", req.jsonrpc)),
            );
        }

        let id = req.id.clone();
        match req.method.as_str() {
            "initialize" => self.handle_initialize(id, req.params),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, req.params).await,
            other => Response::error(id, RpcError::method_not_found(other)),
        }
    }

    fn handle_initialize(&self, id: Value, _params: Value) -> Response {
        let info = ServerInfo::default();
        Response::success(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": info.name, "version": info.version }
            }),
        )
    }

    fn handle_tools_list(&self, id: Value) -> Response {
        Response::success(id, json!({ "tools": self.registry.descriptors() }))
    }

    async fn handle_tools_call(&self, id: Value, params: Value) -> Response {
        let name = match params.get("name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            None => {
                return Response::error(
                    id,
                    RpcError::invalid_params("missing required field: name"),
                );
            }
        };
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);

        let tool = match self.registry.find(&name) {
            Some(t) => t,
            None => {
                self.metrics.incr_error(&name);
                return Response::error(id, RpcError::method_not_found(&name));
            }
        };

        self.metrics.incr_invocation(&name);
        let started = std::time::Instant::now();

        let outcome: Result<ToolResult, ToolError> =
            match tokio::task::spawn(run_tool(tool, self.ctx.clone(), args)).await {
                Ok(res) => res,
                Err(join_err) => {
                    self.metrics.incr_error(&name);
                    return Response::error(
                        id,
                        RpcError::internal_error(format!("tool task panicked: {join_err}")),
                    );
                }
            };

        let elapsed = started.elapsed().as_millis() as u64;
        self.metrics.observe_latency(&name, elapsed);

        match outcome {
            Ok(result) => {
                if result.is_error {
                    self.metrics.incr_error(&name);
                }
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                Response::success(id, value)
            }
            Err(err) => {
                self.metrics.incr_error(&name);
                let rpc_err = match err.reason.as_str() {
                    crate::tools::reasons::INVALID_INPUT => {
                        RpcError::invalid_params(err.message.clone())
                    }
                    _ => RpcError::internal_error(err.message.clone()),
                }
                .with_data(json!({ "reason": err.reason }));
                Response::error(id, rpc_err)
            }
        }
    }
}

async fn run_tool(
    tool: Arc<dyn crate::tools::Tool>,
    ctx: ToolContext,
    args: Value,
) -> Result<ToolResult, ToolError> {
    tool.call(&ctx, args).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::error_codes;

    fn make_server() -> Server {
        Server::new(ToolContext::new("http://127.0.0.1:1"))
    }

    #[tokio::test]
    async fn initialize_returns_protocol_version_and_tools_capability() {
        let s = make_server();
        let req =
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["jsonrpc"], JSONRPC_VERSION);
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(v["result"]["capabilities"]["tools"].is_object());
        assert_eq!(v["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[tokio::test]
    async fn tools_list_returns_eleven_descriptors() {
        let s = make_server();
        let req = br#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        let arr = v["result"]["tools"].as_array().expect("tools array");
        assert_eq!(
            arr.len(),
            11,
            "phase13g §1 adds cortex_active_work (10 -> 11)"
        );
        let names: Vec<&str> = arr.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "cortex_query",
            "cortex_pre_thinking",
            "cortex_status",
            "cortex_audit",
            "cortex_capture_memory",
            "cortex_session_replay",
            "cortex_forget",
            "cortex_active_work",
        ] {
            assert!(
                names.contains(&expected),
                "tools/list must advertise {expected}; got {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let s = make_server();
        let req = br#"{"jsonrpc":"2.0","id":3,"method":"does/not/exist","params":{}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["error"]["code"], error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn malformed_frame_returns_parse_error_and_server_stays_alive() {
        let s = make_server();
        let raw = s.handle_frame(b"not json at all").await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["error"]["code"], error_codes::PARSE_ERROR);

        // Subsequent frame still works.
        let req = br#"{"jsonrpc":"2.0","id":99,"method":"tools/list","params":{}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert!(v["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn notifications_dont_emit_a_response() {
        let s = make_server();
        let req = br#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        assert!(s.handle_frame(req).await.is_none());
        assert_eq!(s.metrics.snapshot().handshakes, 1);
    }

    #[tokio::test]
    async fn unknown_tool_returns_method_not_found_with_metric() {
        let s = make_server();
        let req = br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cortex_unknown","arguments":{}}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["error"]["code"], error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn tools_call_missing_name_returns_invalid_params() {
        let s = make_server();
        let req = br#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{}}"#;
        let raw = s.handle_frame(req).await.expect("response");
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["error"]["code"], error_codes::INVALID_PARAMS);
    }
}
