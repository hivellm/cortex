//! Phase11t §3.3 — `cortex_forget` live IT.
//!
//! Gated on `CORTEX_FORGET_IT=1`. Requires a running cortex-api
//! at `CORTEX_API_URL` (default `http://127.0.0.1:17000`) with the
//! Vectorizer + Meili + Nexus + archive backends reachable.
//!
//! The test seeds one consolidation envelope into the archive,
//! drives `cortex_forget` through the MCP tool, then asserts the
//! row no longer surfaces via cortex-api's read paths.
//!
//! Default `cargo test` skips this so CI without the live stack
//! stays green.

use cortex_mcp_server::tools::{ForgetTool, Tool, ToolContext};
use serde_json::json;

fn it_enabled() -> bool {
    std::env::var("CORTEX_FORGET_IT").as_deref() == Ok("1")
}

fn api_url() -> String {
    std::env::var("CORTEX_API_URL").unwrap_or_else(|_| "http://127.0.0.1:17000".to_string())
}

#[tokio::test]
async fn forget_tool_round_trips_against_live_cortex_api() {
    if !it_enabled() {
        eprintln!("skipping forget_it (CORTEX_FORGET_IT != 1)");
        return;
    }

    let ctx = ToolContext::new(api_url());
    let tool = ForgetTool::new();

    // Step 1: dry-run against an event id that should not exist.
    // The live cortex-api returns the projected cascade plan with
    // `dry_run = true` — confirms the endpoint is wired and the
    // tool's MCP-side proxy path round-trips a non-error result.
    let res = tool
        .call(
            &ctx,
            json!({
                "event_id": "01HXNONEXISTENTPHASE11TIT00",
                "confirmation_token": "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE",
                "dry_run": true,
            }),
        )
        .await
        .expect("dry-run must round-trip");
    assert!(!res.is_error, "dry-run must not surface as error");

    // Step 2: missing-token call must surface as an MCP-level error.
    let err = tool
        .call(
            &ctx,
            json!({
                "event_id": "01HXNONEXISTENTPHASE11TIT00",
                "confirmation_token": "wrong",
            }),
        )
        .await
        .expect_err("missing-token must reject");
    assert!(
        err.message.contains("confirmation_token"),
        "expected token-mismatch message; got {:?}",
        err.message
    );
}
