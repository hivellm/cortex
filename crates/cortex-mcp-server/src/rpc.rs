//! JSON-RPC 2.0 framing — request / response / notification / error.
//!
//! Spec 18 §MCP wire protocol. The MCP host emits one JSON object per
//! line; the server replies in the same envelope. Error codes follow
//! JSON-RPC 2.0 (`-32700` parse error, `-32600` invalid request,
//! `-32601` method not found, `-32602` invalid params, `-32603`
//! internal error) plus a custom `data` field that carries the
//! spec-11 reason string when the underlying API rejects a call.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 fixed version tag.
pub const JSONRPC_VERSION: &str = "2.0";

/// MCP protocol revision the server speaks.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Standard JSON-RPC error codes.
pub mod error_codes {
    /// `-32700` parse error.
    pub const PARSE_ERROR: i32 = -32700;
    /// `-32600` invalid request shape.
    pub const INVALID_REQUEST: i32 = -32600;
    /// `-32601` method not found.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// `-32602` invalid params for the method.
    pub const INVALID_PARAMS: i32 = -32602;
    /// `-32603` internal error.
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// Inbound message — request or notification. Untagged so the parser
/// routes by presence of `id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Inbound {
    /// Request with an `id` — must be answered.
    Request(Request),
    /// Notification (no `id`).
    Notification(Notification),
}

/// JSON-RPC request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Numeric or string id; mirrored in the response.
    pub id: Value,
    /// Method name (e.g. `"initialize"`, `"tools/list"`, `"tools/call"`).
    pub method: String,
    /// Method-specific parameters.
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC notification envelope (no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method-specific parameters.
    #[serde(default)]
    pub params: Value,
}

/// Outbound envelope — either a result or an error, always tagged
/// with the request's `id`.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Mirrored id.
    pub id: Value,
    /// Method-specific result body. Mutually exclusive with `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error envelope. Mutually exclusive with `result`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Build a success response.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }
    /// Build an error response.
    pub fn error(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC error envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Numeric error code.
    pub code: i32,
    /// Short human-readable message.
    pub message: String,
    /// Optional structured payload — used to surface the spec-11
    /// `reason` so callers can pattern-match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// `-32700` parse error.
    pub fn parse_error(detail: impl Into<String>) -> Self {
        Self {
            code: error_codes::PARSE_ERROR,
            message: detail.into(),
            data: None,
        }
    }
    /// `-32600` invalid request.
    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self {
            code: error_codes::INVALID_REQUEST,
            message: detail.into(),
            data: None,
        }
    }
    /// `-32601` method not found.
    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self {
            code: error_codes::METHOD_NOT_FOUND,
            message: format!("method not found: {}", method.into()),
            data: None,
        }
    }
    /// `-32602` invalid params.
    pub fn invalid_params(detail: impl Into<String>) -> Self {
        Self {
            code: error_codes::INVALID_PARAMS,
            message: detail.into(),
            data: None,
        }
    }
    /// `-32603` internal error.
    pub fn internal_error(detail: impl Into<String>) -> Self {
        Self {
            code: error_codes::INTERNAL_ERROR,
            message: detail.into(),
            data: None,
        }
    }
    /// Attach a structured `data` payload.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips_through_json() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let req: Request = serde_json::from_str(raw).expect("parse");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tools/list");
    }

    #[test]
    fn untagged_inbound_routes_to_request_when_id_present() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"x","params":{}}"#;
        let inbound: Inbound = serde_json::from_str(raw).expect("parse");
        assert!(matches!(inbound, Inbound::Request(_)));
    }

    #[test]
    fn untagged_inbound_routes_to_notification_when_id_absent() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let inbound: Inbound = serde_json::from_str(raw).expect("parse");
        assert!(matches!(inbound, Inbound::Notification(_)));
    }

    #[test]
    fn response_serializes_with_either_result_or_error() {
        let ok = Response::success(json!(1), json!({"ok":true}));
        let bytes = serde_json::to_string(&ok).unwrap();
        assert!(bytes.contains("\"result\""));
        assert!(!bytes.contains("\"error\""));

        let err = Response::error(
            json!(1),
            RpcError::method_not_found("nope").with_data(json!({"reason":"x"})),
        );
        let bytes = serde_json::to_string(&err).unwrap();
        assert!(bytes.contains("\"error\""));
        assert!(!bytes.contains("\"result\""));
        assert!(bytes.contains("\"reason\""));
    }
}
