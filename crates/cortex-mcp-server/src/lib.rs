//! Cortex MCP server — JSON-RPC 2.0 stdio bridge from Claude Code
//! into `cortex-api` (spec 11) and `cortex-pre-thinking` (spec 12).
//!
//! See `docs/specs/18-claude-code-plugin.md` for the wire protocol +
//! tool contract. Three tools are exposed:
//!
//! - `cortex_query` — wraps `POST <api_url>/v1/query`.
//! - `cortex_pre_thinking` — runs the spec-12 pipeline against the
//!   same `cortex-api`.
//! - `cortex_status` — daemon health snapshot.
//!
//! The library half is reusable from tests; the binary half lives in
//! `main.rs` and only handles clap parsing + bootstrap.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod metrics;
pub mod rpc;
pub mod server;
pub mod tools;
pub mod transport_stdio;
pub mod validate;

pub use metrics::Metrics;
pub use rpc::{
    error_codes, Inbound, Notification, Request, Response, RpcError, JSONRPC_VERSION,
    MCP_PROTOCOL_VERSION,
};
pub use server::{Server, ServerInfo, SERVER_NAME, SERVER_VERSION};
pub use tools::{
    ConsolidationGetTool, ConsolidationLineageTool, ConsolidationsByEntityTool,
    ConsolidationsDiffTool, ConsolidationsRecentTool, ConsolidationsSearchTool, EventsByKindTool,
    FeedbackSignalsTool, FilesTouchedTool, ForgetTool, LawViolationsTool, PreThinkingTool,
    QueryTool, SessionTimelineTool, StatusTool, Tool, ToolCallsTool, ToolContext, ToolError,
    ToolRegistry, ToolResult, TopicSearchTool, MCP_TOOL_TIMEOUT_MS,
};
pub use validate::{validate_plugin, ValidationReport};

/// Default Cortex API URL — matches the env var fall-back used by
/// the binary entry point and the `.mcp.json` shipped with the
/// plugin. Port 17000 mirrors `.env CORTEX_API_PORT=17000`, the
/// canonical bind cortex-api uses across the workspace.
pub const DEFAULT_API_URL: &str = "http://127.0.0.1:17000";
