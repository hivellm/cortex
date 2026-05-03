//! Claude Code CLI backend.
//!
//! Spawns `claude -p <prompt> --model <model> --output-format json
//! --max-tokens 4096`, parses the tool's JSON envelope, then parses the
//! inner model output as the classifier contract from `prompts/classifier.v1.txt`.
//!
//! The CLI binary is only required at deploy time — unit tests exercise the
//! parser and fall back to [`StaticClassifier`](crate::StaticClassifier) when the CLI is
//! unreachable.

use super::errors::ClassifierError;
use super::prompt::{PromptTemplate, PROMPT_V1};
use super::types::{
    Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput, PiiRisk, Severity,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Path to a per-process Claude Code config directory whose
/// `settings.json` declares no plugins / hooks. The classifier passes
/// this to every `claude` subprocess via `CLAUDE_CONFIG_DIR` so the
/// child does NOT load the host's `cortex-plugin` (which would fire
/// SessionStart / UserPromptSubmit / Stop / tool hooks straight back
/// at the live cortex-adapter, looping classification output back
/// through the bus as fresh turn / tool_call envelopes).
///
/// Why CLAUDE_CONFIG_DIR rather than a sentinel env var picked up by
/// the hooks themselves: empirically (2026-04-28), Claude Code does
/// not propagate arbitrary parent env vars (`CORTEX_ADAPTER_DISABLE`,
/// etc.) to the bash/pwsh shim it spawns for hook execution, so any
/// approach that depends on the hook seeing a marker env fails
/// silently. CLAUDE_CONFIG_DIR is part of the documented Claude Code
/// contract and IS honoured at config-load time, so pointing it at a
/// hook-free directory is the only mechanism that's guaranteed to
/// suppress the loopback.
fn hookless_config_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("cortex-classifier-hookless-config");
        // Recreate on each daemon boot so a stale settings.json (e.g.
        // an old binary that wrote hook entries here in error) cannot
        // contaminate the new run.
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                error = %e,
                dir = %dir.display(),
                "failed to create hookless CLAUDE_CONFIG_DIR; classifier subprocess hooks may loop back into the bus"
            );
            return dir;
        }
        // An empty `hooks` block disables every lifecycle hook a
        // marketplace plugin would otherwise register.
        let settings = dir.join("settings.json");
        if let Err(e) = std::fs::write(&settings, b"{\"hooks\":{}}\n") {
            tracing::warn!(
                error = %e,
                path = %settings.display(),
                "failed to write hookless settings.json"
            );
        }
        // claude-code authenticates via `~/.claude/.credentials.json`;
        // an empty config dir has no credentials, so the subprocess
        // exits with status 1 ("not logged in"). Copy the host's
        // credentials so the subprocess inherits the operator's
        // login but NOT the operator's plugins. We try a couple of
        // common locations: the home dir and the explicit
        // `CLAUDE_CONFIG_DIR` if it's already set on the parent.
        let creds_src = host_claude_credentials_path();
        if let Some(src) = creds_src {
            let dst = dir.join(".credentials.json");
            if let Err(e) = std::fs::copy(&src, &dst) {
                tracing::warn!(
                    error = %e,
                    src = %src.display(),
                    dst = %dst.display(),
                    "failed to copy claude credentials into hookless config dir; subprocess will fail to authenticate"
                );
            }
        } else {
            tracing::warn!(
                "could not locate host claude credentials; classifier subprocess will likely fail to authenticate"
            );
        }
        dir
    })
}

/// Best-effort lookup for the operator's existing `.credentials.json`
/// so the classifier subprocess can authenticate without re-running
/// `claude /login`. Honours `CLAUDE_CONFIG_DIR` when set; otherwise
/// falls back to `$HOME/.claude/.credentials.json` (Unix and Windows).
fn host_claude_credentials_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(dir).join(".credentials.json");
        if p.exists() {
            return Some(p);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let p = PathBuf::from(home)
        .join(".claude")
        .join(".credentials.json");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

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
        ]);
        for (k, v) in &self.cfg.envs {
            cmd.env(k, v);
        }
        // Point the subprocess at a hook-free Claude Code config dir
        // so it doesn't load the cortex-plugin and dispatch hooks
        // back at the live adapter. See `hookless_config_dir` above
        // for the full incident write-up. Belt-and-suspenders:
        // CORTEX_ADAPTER_DISABLE is also set so any hook that
        // accidentally still loads (custom user-installed plugins,
        // for example) can opt out via its own guard.
        cmd.env("CLAUDE_CONFIG_DIR", hookless_config_dir());
        cmd.env("CORTEX_ADAPTER_DISABLE", "1");
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
            .body()
            .ok_or_else(|| ClassifierError::Backend("claude response has no text body".into()))?
            .to_string();
        // The model wraps JSON in ```json … ``` fences with some
        // frequency. Strip them so the inner parse doesn't choke
        // on the markdown.
        let cleaned = strip_code_fence(&inner_text);
        let inner: ClassifierOutputBatch = serde_json::from_str(cleaned.trim())?;

        // The model is supposed to return exactly one record per input
        // in arrival order, but sonnet occasionally emits extra
        // entries (an echoed example or a duplicate). When that
        // happens, fall back to matching by `event_id` so a single
        // bad record doesn't drop the whole batch into the static
        // fallback path. Strict zip is kept for the equal-length
        // happy path because it's the canonical contract and indexes
        // are cheaper than HashMap lookups.
        let records = if inner.events.len() == events.len() {
            inner.events
        } else {
            tracing::warn!(
                expected = events.len(),
                actual = inner.events.len(),
                "classifier returned non-canonical batch size; matching by event_id"
            );
            let mut by_id: std::collections::HashMap<String, ClassifierRecord> = inner
                .events
                .into_iter()
                .filter_map(|rec| rec.event_id.clone().map(|id| (id, rec)))
                .collect();
            let mut aligned: Vec<ClassifierRecord> = Vec::with_capacity(events.len());
            for input in events {
                match by_id.remove(&input.event_id) {
                    Some(rec) => aligned.push(rec),
                    None => {
                        return Err(ClassifierError::LengthMismatch {
                            expected: events.len(),
                            actual: aligned.len(),
                        });
                    }
                }
            }
            aligned
        };

        let latency_ms = started.elapsed().as_millis() as u32;
        let tokens_in = outer.tokens.as_ref().and_then(|t| t.input).unwrap_or(0);
        let tokens_out = outer.tokens.as_ref().and_then(|t| t.output).unwrap_or(0);

        Ok(events
            .iter()
            .zip(records)
            .map(|(input, rec)| ClassifierOutput {
                event_id: input.event_id.clone(),
                kind_refinement: rec.kind_refinement,
                topics: normalise_topics(rec.topics),
                severity: rec.severity,
                pii_risk: rec.pii_risk,
                redaction_suggestions: rec.redaction_suggestions,
                summary: rec.summary,
                // Pass through Sonnet's typed entities + relations
                // so the graph mapper can hoist them into Nexus
                // nodes/edges. Empty when the model emitted nothing
                // (or when the fallback static path runs).
                entities: rec.entities,
                relations: rec.relations,
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
/// CLI 2.x renamed the body field from `text` to `result`; we
/// accept both so the parser keeps working across CLI versions
/// (and so tests that fixture the older `text` field still pass).
#[derive(Debug, Deserialize)]
pub struct ClaudeJsonResponse {
    /// Body in CLI 2.x. Wins when present.
    #[serde(default)]
    pub result: Option<String>,
    /// Body in CLI 1.x. Legacy fallback.
    #[serde(default)]
    pub text: Option<String>,
    /// Token usage if reported.
    #[serde(default)]
    pub tokens: Option<ClaudeTokens>,
    /// Any free-form fields we don't care about.
    #[serde(flatten)]
    pub rest: std::collections::BTreeMap<String, Value>,
}

impl ClaudeJsonResponse {
    /// `result` (CLI 2.x) → fallback to `text` (CLI 1.x).
    pub fn body(&self) -> Option<&str> {
        self.result.as_deref().or(self.text.as_deref())
    }
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
    pub redaction_suggestions: Vec<super::types::RedactionSuggestion>,
    /// Short summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Typed entities the model identified inside the event.
    /// Each becomes a Nexus node in the graph mapper.
    #[serde(default)]
    pub entities: Vec<super::types::ExtractedEntity>,
    /// Semantic relations between this event and the listed
    /// entities. Each becomes a typed edge in Nexus.
    #[serde(default)]
    pub relations: Vec<super::types::ExtractedRelation>,
}

/// Normalize free-form classifier topics against the controlled vocab —
/// drops anything not in [`TOPIC_VOCAB_V1`](crate::prompt::TOPIC_VOCAB_V1),
/// sorts, and dedups. Exposed as `#[doc(hidden)] pub` so the integration
/// tests can exercise the helper directly.
#[doc(hidden)]
pub fn normalise_topics(mut topics: Vec<String>) -> Vec<String> {
    use super::prompt::TOPIC_VOCAB_V1;
    topics.retain(|t| TOPIC_VOCAB_V1.iter().any(|v| v == &t.as_str()));
    topics.sort();
    topics.dedup();
    topics
}

/// Strip ```json … ``` markdown fences. Sonnet sometimes wraps
/// its JSON output in fences despite explicit instructions; this
/// keeps the inner parse from failing.
fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let stripped = stripped.strip_suffix("```").unwrap_or(stripped);
    stripped.trim().to_string()
}
