//! Phase14i §2.4 — MCP tool-timeout integration test.
//!
//! Stubs a tool whose `call` body sleeps 10s, configures a 200ms
//! per-tool timeout, fires `tools/call`, and asserts:
//!
//! 1. The MCP response carries `error.data.reason = "tool_timeout"`.
//! 2. The response surfaces within 5.5s (well under the 10s sleep).
//! 3. `error.data.elapsed_ms` is non-zero and `error.data.tool`
//!    matches the requested tool name.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_mcp_server::{Server, Tool, ToolContext, ToolError, ToolRegistry, ToolResult};
use serde_json::{json, Value};

struct SlowTool;

#[async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &'static str {
        "slow_tool"
    }
    fn descriptor(&self) -> Value {
        json!({
            "name": "slow_tool",
            "description": "Sleeps 10s to exercise the timeout contract.",
            "inputSchema": { "type": "object" }
        })
    }
    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<ToolResult, ToolError> {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(ToolResult {
            content: vec![json!({"type": "text", "text": "never reached"})],
            is_error: false,
        })
    }
}

#[tokio::test]
async fn tools_call_slow_tool_surfaces_tool_timeout_error_within_budget() {
    let ctx = ToolContext::new("http://127.0.0.1:1")
        .with_tool_timeout("slow_tool", Duration::from_millis(200));
    let mut registry = ToolRegistry::new();
    registry.push(Arc::new(SlowTool));
    let server = Server::with_registry(ctx, registry);

    let frame = br#"{
        "jsonrpc": "2.0",
        "id": 42,
        "method": "tools/call",
        "params": { "name": "slow_tool", "arguments": {} }
    }"#;

    let started = Instant::now();
    let raw = server.handle_frame(frame).await.expect("response");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(5_500),
        "tool timeout must surface within 5.5s, got {elapsed:?}"
    );

    let v: Value = serde_json::from_slice(&raw).expect("json");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 42);
    let error = v.get("error").expect("response must carry an error");
    let data = error.get("data").expect("error.data");
    assert_eq!(data["reason"], "tool_timeout");
    assert_eq!(data["tool"], "slow_tool");
    let elapsed_ms = data["elapsed_ms"].as_u64().expect("elapsed_ms u64");
    assert!(
        (150..5_500).contains(&elapsed_ms),
        "elapsed_ms must reflect the configured timeout (~200ms), got {elapsed_ms}"
    );
    assert_eq!(data["request_id"], json!(42));
}

#[tokio::test]
async fn default_timeout_applies_when_no_per_tool_override_is_registered() {
    // Default is MCP_TOOL_TIMEOUT_MS = 5_000. Override the
    // *default* via the builder to 150ms so the test stays
    // snappy without per-tool wiring.
    let ctx = ToolContext::new("http://127.0.0.1:1")
        .with_default_tool_timeout(Duration::from_millis(150));
    let mut registry = ToolRegistry::new();
    registry.push(Arc::new(SlowTool));
    let server = Server::with_registry(ctx, registry);

    let frame = br#"{
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": { "name": "slow_tool", "arguments": {} }
    }"#;

    let started = Instant::now();
    let raw = server.handle_frame(frame).await.expect("response");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(5_500),
        "default-timeout path must surface within 5.5s, got {elapsed:?}"
    );

    let v: Value = serde_json::from_slice(&raw).expect("json");
    assert_eq!(v["error"]["data"]["reason"], "tool_timeout");
    assert_eq!(v["error"]["data"]["tool"], "slow_tool");
}

#[tokio::test]
async fn fast_tool_below_timeout_returns_normal_result() {
    struct FastTool;
    #[async_trait]
    impl Tool for FastTool {
        fn name(&self) -> &'static str {
            "fast_tool"
        }
        fn descriptor(&self) -> Value {
            json!({"name": "fast_tool", "description": "fast", "inputSchema": {"type": "object"}})
        }
        async fn call(
            &self,
            _ctx: &ToolContext,
            _args: Value,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult {
                content: vec![json!({"type": "text", "text": "ok"})],
                is_error: false,
            })
        }
    }
    let ctx = ToolContext::new("http://127.0.0.1:1")
        .with_tool_timeout("fast_tool", Duration::from_millis(500));
    let mut registry = ToolRegistry::new();
    registry.push(Arc::new(FastTool));
    let server = Server::with_registry(ctx, registry);

    let frame = br#"{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "fast_tool", "arguments": {} }
    }"#;
    let raw = server.handle_frame(frame).await.expect("response");
    let v: Value = serde_json::from_slice(&raw).expect("json");
    assert!(v.get("error").is_none(), "fast tool must not error: {v:?}");
    assert_eq!(v["result"]["content"][0]["text"], "ok");
}
