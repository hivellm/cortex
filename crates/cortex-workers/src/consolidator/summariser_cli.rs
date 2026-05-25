//! Phase11p ad-hoc — `claude` CLI subprocess implementation of [`Summariser`].
//!
//! Mirrors `crate::classifier::haiku_cli::HaikuCliClassifier` for the
//! consolidation path: pipes the rendered prompt through `claude -p -
//! --model <id> --output-format json --bare` and parses the
//! `{"result": "<text>", "usage": {...}}` envelope back into a
//! [`SummariserResult`]. Used when the operator does not have an
//! `ANTHROPIC_API_KEY` but does have a logged-in Claude Code CLI.
//!
//! The CLI path costs $0 from the consolidator's accounting POV (the
//! billing happens against the user's CLI subscription), so
//! `cost_cents` is reported as the same value the API path would
//! have charged, computed from the usage block when present and
//! estimated from prompt/output bytes otherwise. This keeps the
//! ledger comparable across runs.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::summariser::{
    cost_cents, Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
    DEFAULT_MAX_OUTPUT_TOKENS,
};

/// Heuristic bytes-per-token used when the CLI does not surface a
/// `usage` block (older `claude` versions). 4 bytes/token matches
/// Anthropic's published rule of thumb for English prose.
const BYTES_PER_TOKEN_FALLBACK: u64 = 4;

/// `claude -p` based summariser. Spawns one process per call.
pub struct ClaudeCliSummariser {
    /// Path to the `claude` binary.
    claude_bin: PathBuf,
    /// Which model to request from the CLI (drives the `--model` flag).
    kind: SummariserKind,
    /// Hard wall-clock cap. The producer's per-grain prompts are
    /// small enough that 5 min is more than enough headroom even
    /// for Opus.
    timeout: Duration,
}

impl ClaudeCliSummariser {
    /// Build a CLI summariser. `claude_bin` defaults to `claude`
    /// (PATH lookup) when callers want the standard install.
    pub fn new(claude_bin: PathBuf, kind: SummariserKind) -> Self {
        Self {
            claude_bin,
            kind,
            timeout: Duration::from_secs(300),
        }
    }

    /// Override the wall-clock cap (test wiring).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Deserialize)]
struct CliResponse {
    /// `claude -p --output-format json` returns the assistant text in
    /// the `result` field for the simple non-streaming case.
    result: Option<String>,
    /// Usage block — present in modern CLIs, absent in older ones.
    usage: Option<CliUsage>,
}

#[derive(Deserialize)]
struct CliUsage {
    /// Input tokens reported by the CLI's accounting.
    input_tokens: Option<u64>,
    /// Output tokens reported by the CLI's accounting.
    output_tokens: Option<u64>,
}

#[async_trait::async_trait]
impl Summariser for ClaudeCliSummariser {
    fn kind(&self) -> SummariserKind {
        self.kind
    }

    async fn summarise(
        &self,
        request: SummariserRequest,
    ) -> Result<SummariserResult, SummariserError> {
        let max_tokens = request
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);

        // 2026-05-19 — `claude -p` does not expose a `--max-tokens`
        // flag. Default effort emits ~1k-token responses, which
        // truncates the consolidation `summary_markdown` mid-sentence
        // (the producer's wire-format target is ~200..2000 bytes
        // for Session, ~200..4000 for TopicCard). `--effort high`
        // raises the thinking + output budget so the JSON wrap
        // closes cleanly and the rendered markdown is a complete
        // paragraph.
        let effort = match self.kind {
            SummariserKind::Haiku45 => "high",
            SummariserKind::Opus47 => "max",
        };
        let mut cmd = Command::new(&self.claude_bin);
        cmd.args([
            "-p",
            "-",
            "--model",
            self.kind.model_id(),
            "--output-format",
            "json",
            "--effort",
            effort,
            "--dangerously-skip-permissions",
        ]);
        // Match the classifier's CLI subprocess hygiene: disable the
        // adapter so the child doesn't dispatch hooks back at the
        // running cortex daemon, and let it use the user's existing
        // keychain / OAuth login (which `--bare` would have blocked).
        cmd.env("CORTEX_ADAPTER_DISABLE", "1");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| SummariserError::Transport(format!("spawn claude: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request.prompt.as_bytes())
                .await
                .map_err(|e| SummariserError::Transport(format!("write stdin: {e}")))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| SummariserError::Transport(format!("close stdin: {e}")))?;
        }

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| SummariserError::Transport("claude cli timeout".into()))?
            .map_err(|e| SummariserError::Transport(format!("claude cli wait: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            return Err(SummariserError::Upstream {
                status: code as u16,
                body: stderr.into_owned(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: CliResponse = serde_json::from_str(stdout.trim()).map_err(|e| {
            SummariserError::Transport(format!(
                "decode claude json (first 200 bytes: {}): {e}",
                stdout.chars().take(200).collect::<String>()
            ))
        })?;

        let text = parsed.result.unwrap_or_default();
        if text.trim().is_empty() {
            return Err(SummariserError::Transport(
                "claude cli returned empty result".into(),
            ));
        }

        let prompt_fallback = request.prompt.len() as u64 / BYTES_PER_TOKEN_FALLBACK;
        let text_fallback = text.len() as u64 / BYTES_PER_TOKEN_FALLBACK;
        let (input_tokens, output_tokens) = match parsed.usage {
            Some(u) => (
                u.input_tokens.unwrap_or(prompt_fallback),
                u.output_tokens.unwrap_or(text_fallback),
            ),
            None => (prompt_fallback, text_fallback),
        };
        let _ = max_tokens;

        let cost = cost_cents(self.kind, input_tokens, output_tokens);

        Ok(SummariserResult {
            text,
            cost_cents: cost,
            kind: self.kind,
            input_tokens,
            output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_summariser_advertises_kind() {
        let s = ClaudeCliSummariser::new(PathBuf::from("claude"), SummariserKind::Haiku45);
        assert_eq!(s.kind(), SummariserKind::Haiku45);
        let s = ClaudeCliSummariser::new(PathBuf::from("claude"), SummariserKind::Opus47);
        assert_eq!(s.kind(), SummariserKind::Opus47);
    }

    #[test]
    fn cli_summariser_with_timeout_overrides_default() {
        let s = ClaudeCliSummariser::new(PathBuf::from("claude"), SummariserKind::Haiku45)
            .with_timeout(Duration::from_secs(7));
        assert_eq!(s.timeout, Duration::from_secs(7));
    }

    #[tokio::test]
    async fn cli_summariser_surfaces_spawn_failure_as_transport_error() {
        let s = ClaudeCliSummariser::new(
            PathBuf::from("claude-binary-that-does-not-exist-zzz"),
            SummariserKind::Haiku45,
        );
        let req = SummariserRequest {
            prompt: "hi".into(),
            max_output_tokens: None,
        };
        let err = s
            .summarise(req)
            .await
            .expect_err("missing binary should error");
        match err {
            SummariserError::Transport(msg) => assert!(msg.contains("spawn"), "unexpected: {msg}"),
            other => panic!("expected Transport, got {other:?}"),
        }
    }
}
