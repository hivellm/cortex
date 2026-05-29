//! Newline-delimited JSON over stdin/stdout — the canonical MCP
//! transport.
//!
//! Each inbound line is one JSON-RPC frame. Each outbound response is
//! one line followed by a single `\n`. Notifications produce no
//! output so the framing stays one-line-per-message in both
//! directions.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::server::Server;

/// Drive the server with stdin/stdout. Returns `Ok(())` on graceful
/// EOF; propagates I/O errors otherwise.
pub async fn run(server: Arc<Server>) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    run_with(server, stdin, stdout).await
}

/// Variant the tests use — accepts arbitrary async readers / writers
/// so the transport can be exercised over an in-memory pipe.
pub async fn run_with<R, W>(server: Arc<Server>, reader: R, writer: W) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut lines = BufReader::new(reader).lines();
    let mut writer = writer;
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_frame(trimmed.as_bytes()).await {
            writer.write_all(&resp).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    #[tokio::test]
    async fn round_trips_initialize_and_tools_list_over_pipe() {
        let server = Arc::new(Server::new(ToolContext::new("http://127.0.0.1:1")));
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n\
                      {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{}}\n\
                      {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n";
        let mut output: Vec<u8> = Vec::new();

        run_with(server.clone(), &input[..], &mut output)
            .await
            .unwrap();

        let text = String::from_utf8(output).unwrap();
        let mut lines = text.lines();
        let init: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(init["id"], 1);
        let list: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(list["id"], 2);
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 30);
        assert!(lines.next().is_none(), "no extra response for notification");
    }

    #[tokio::test]
    async fn blank_lines_are_ignored() {
        let server = Arc::new(Server::new(ToolContext::new("http://127.0.0.1:1")));
        let input =
            b"\n   \n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n";
        let mut output: Vec<u8> = Vec::new();
        run_with(server, &input[..], &mut output).await.unwrap();
        let text = String::from_utf8(output).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["id"], 1);
    }
}
