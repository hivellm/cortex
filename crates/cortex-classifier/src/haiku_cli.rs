//! Claude Code CLI backend.
//!
//! Spawns `claude -p <prompt> --model <model> --output-format json
//! --max-tokens 4096`, parses the tool's JSON envelope, then parses the
//! inner model output as the classifier contract from `prompts/classifier.v1.txt`.
//!
//! The CLI binary is only required at deploy time — unit tests exercise the
//! parser and fall back to [`StaticClassifier`](crate::StaticClassifier) when the CLI is
//! unreachable.

use crate::errors::ClassifierError;
use crate::prompt::{PromptTemplate, PROMPT_V1};
use crate::types::{
    Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput, PiiRisk, Severity,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Config for [`HaikuCliClassifier`].
#[derive(Debug, Clone)]
pub struct HaikuCliConfig {
    /// Path to the `claude` binary.
    pub claude_bin: PathBuf,
    /// Model id (`claude-haiku-4-5`).
    pub model: String,
    /// Upper bound for the CLI to return; on timeout the process is killed.
    pub timeout: Duration,
    /// Optional environment overrides (e.g. `CLAUDE_CODE_PROJECT`).
    pub envs: Vec<(String, String)>,
}

impl Default for HaikuCliConfig {
    fn default() -> Self {
        Self {
            claude_bin: PathBuf::from("claude"),
            model: "claude-haiku-4-5".into(),
            timeout: Duration::from_secs(30),
            envs: Vec::new(),
        }
    }
}

/// Haiku invoked through the Claude Code CLI.
pub struct HaikuCliClassifier {
    cfg: HaikuCliConfig,
    prompt: PromptTemplate,
}

impl HaikuCliClassifier {
    /// Build a new CLI-backed classifier.
    pub fn new(cfg: HaikuCliConfig) -> Self {
        Self {
            cfg,
            prompt: PROMPT_V1,
        }
    }
}

#[async_trait]
impl Classifier for HaikuCliClassifier {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = self.prompt.render(events)?;
        let started = Instant::now();

        let mut cmd = Command::new(&self.cfg.claude_bin);
        cmd.args([
            "-p",
            "-",
            "--model",
            &self.cfg.model,
            "--output-format",
            "json",
            "--max-tokens",
            "4096",
        ]);
        for (k, v) in &self.cfg.envs {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ClassifierError::Backend(format!("spawn: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(ClassifierError::Io)?;
            stdin.shutdown().await.map_err(ClassifierError::Io)?;
        }

        let output = tokio::time::timeout(self.cfg.timeout, child.wait_with_output())
            .await
            .map_err(|_| ClassifierError::Backend("cli timeout".into()))?
            .map_err(ClassifierError::Io)?;

        if !output.status.success() {
            return Err(ClassifierError::Backend(format!(
                "claude exit {}: {}",
                output.status.code().unwrap_or_default(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|e| ClassifierError::Backend(format!("stdout utf8: {e}")))?;

        let outer: ClaudeJsonResponse = serde_json::from_str(stdout.trim())?;
        let inner_text = outer
            .text
            .ok_or_else(|| ClassifierError::Backend("claude response has no text body".into()))?;
        let inner: ClassifierOutputBatch = serde_json::from_str(inner_text.trim())?;

        if inner.events.len() != events.len() {
            return Err(ClassifierError::LengthMismatch {
                expected: events.len(),
                actual: inner.events.len(),
            });
        }

        let latency_ms = started.elapsed().as_millis() as u32;
        let tokens_in = outer.tokens.as_ref().and_then(|t| t.input).unwrap_or(0);
        let tokens_out = outer.tokens.as_ref().and_then(|t| t.output).unwrap_or(0);

        Ok(events
            .iter()
            .zip(inner.events)
            .map(|(input, rec)| ClassifierOutput {
                event_id: input.event_id.clone(),
                kind_refinement: rec.kind_refinement,
                topics: normalise_topics(rec.topics),
                severity: rec.severity,
                pii_risk: rec.pii_risk,
                redaction_suggestions: rec.redaction_suggestions,
                summary: rec.summary,
                source: ClassifierSource::HaikuCli,
                prompt_version: self.prompt.version.into(),
                model: self.cfg.model.clone(),
                latency_ms,
                tokens_in,
                tokens_out,
            })
            .collect())
    }
}

/// Outer Claude Code CLI response (shape of `--output-format json`).
#[derive(Debug, Deserialize)]
pub struct ClaudeJsonResponse {
    /// Model output body (may already be JSON-encoded).
    #[serde(default)]
    pub text: Option<String>,
    /// Token usage if reported.
    #[serde(default)]
    pub tokens: Option<ClaudeTokens>,
    /// Any free-form fields we don't care about.
    #[serde(flatten)]
    pub rest: std::collections::BTreeMap<String, Value>,
}

/// Token usage subsection.
#[derive(Debug, Deserialize)]
pub struct ClaudeTokens {
    /// Input tokens.
    #[serde(default)]
    pub input: Option<u32>,
    /// Output tokens.
    #[serde(default)]
    pub output: Option<u32>,
}

/// Model-facing classifier output shape.
#[derive(Debug, Deserialize)]
pub struct ClassifierOutputBatch {
    /// One record per input event, in order.
    pub events: Vec<ClassifierRecord>,
}

/// Per-event classifier record.
#[derive(Debug, Deserialize)]
pub struct ClassifierRecord {
    /// Echo of the input event id.
    #[serde(default)]
    pub event_id: Option<String>,
    /// Refined kind (e.g. `git_push`).
    #[serde(default)]
    pub kind_refinement: Option<String>,
    /// Multi-label topics (filtered server-side to the controlled vocab).
    #[serde(default)]
    pub topics: Vec<String>,
    /// Severity label.
    pub severity: Severity,
    /// PII risk.
    pub pii_risk: PiiRisk,
    /// Extra redaction suggestions.
    #[serde(default)]
    pub redaction_suggestions: Vec<crate::types::RedactionSuggestion>,
    /// Short summary.
    #[serde(default)]
    pub summary: Option<String>,
}

/// Normalize free-form classifier topics against the controlled vocab —
/// drops anything not in [`TOPIC_VOCAB_V1`](crate::prompt::TOPIC_VOCAB_V1),
/// sorts, and dedups. Exposed as `#[doc(hidden)] pub` so the integration
/// tests can exercise the helper directly.
#[doc(hidden)]
pub fn normalise_topics(mut topics: Vec<String>) -> Vec<String> {
    use crate::prompt::TOPIC_VOCAB_V1;
    topics.retain(|t| TOPIC_VOCAB_V1.iter().any(|v| v == &t.as_str()));
    topics.sort();
    topics.dedup();
    topics
}
