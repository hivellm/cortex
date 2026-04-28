//! `~/.cortex/adapter.toml` schema. Mirrors `docs/specs/10-claude-
//! code-adapter.md` §Adapter config.
//!
//! Defaults are designed so a fresh `cortex-adapters install
//! claude-code` Just Works against a default-config `cortex-core` /
//! `cortex-api` deployment. Overrides only need to be set when an
//! operator runs the services on non-standard ports.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Top-level config — wraps the `[adapter]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterToml {
    /// `[adapter]` table.
    #[serde(default)]
    pub adapter: AdapterSection,
}

/// `[adapter]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterSection {
    /// `cortex-core` ingestion router base URL.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// `cortex-api` base URL (query + law-check).
    #[serde(default = "default_api_endpoint")]
    pub api_endpoint: String,
    /// Hard cap on every outbound HTTP request issued by the daemon.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Bound on the in-memory async publisher queue.
    #[serde(default = "default_queue")]
    pub queue_bounded: usize,
    /// Pre-thinking sub-section.
    #[serde(default)]
    pub pre_thinking: PreThinkingSection,
    /// Laws sub-section.
    #[serde(default)]
    pub laws: LawsSection,
    /// Redaction sub-section.
    #[serde(default)]
    pub redaction: RedactionSection,
    /// Logging sub-section.
    #[serde(default)]
    pub logging: LoggingSection,
}

impl Default for AdapterSection {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_endpoint: default_api_endpoint(),
            timeout_ms: default_timeout_ms(),
            queue_bounded: default_queue(),
            pre_thinking: PreThinkingSection::default(),
            laws: LawsSection::default(),
            redaction: RedactionSection::default(),
            logging: LoggingSection::default(),
        }
    }
}

fn default_endpoint() -> String {
    "http://127.0.0.1:17010".to_string()
}
fn default_api_endpoint() -> String {
    "http://127.0.0.1:17000".to_string()
}
fn default_timeout_ms() -> u64 {
    1500
}
fn default_queue() -> usize {
    2048
}

/// Pre-thinking knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreThinkingSection {
    /// Whether the synchronous pre-thinking call is wired up.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum bundle size returned to Claude Code, in KB.
    #[serde(default = "default_bundle_kb")]
    pub max_bundle_kb: u64,
    /// Hook-budget timeout. Claude Code's overall budget is 1 s; the
    /// adapter caps itself at 600 ms by default so the user-prompt
    /// hook returns in time even when the API is slow.
    #[serde(default = "default_pre_thinking_timeout")]
    pub timeout_ms: u64,
}

impl Default for PreThinkingSection {
    fn default() -> Self {
        Self {
            enabled: true,
            max_bundle_kb: 32,
            timeout_ms: default_pre_thinking_timeout(),
        }
    }
}

fn default_pre_thinking_timeout() -> u64 {
    600
}
fn default_bundle_kb() -> u64 {
    32
}

/// Laws-check knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LawsSection {
    /// Whether `severity=critical` violations should `permissionDecision: deny`
    /// the tool call. Operators can flip to false for a soft-block
    /// (capture only) deployment.
    #[serde(default = "default_true")]
    pub block_on_critical: bool,
    /// Hook-budget cap. Above this the daemon fails-open per spec 10
    /// §Synchronous paths.
    #[serde(default = "default_laws_timeout")]
    pub timeout_ms: u64,
}

impl Default for LawsSection {
    fn default() -> Self {
        Self {
            block_on_critical: true,
            timeout_ms: default_laws_timeout(),
        }
    }
}

fn default_laws_timeout() -> u64 {
    300
}

/// Redaction knobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedactionSection {
    /// Adapter-side extra patterns merged into the in-process
    /// redactor. `cortex-core` still runs the authoritative pass.
    #[serde(default)]
    pub extra_patterns: Vec<ExtraPattern>,
}

/// One redaction pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraPattern {
    /// Pattern name.
    pub name: String,
    /// Regex source.
    pub regex: String,
}

/// Logging knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingSection {
    /// `trace`/`debug`/`info`/`warn`/`error`.
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Where the daemon's log file lives.
    #[serde(default = "default_log_path")]
    pub path: String,
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            path: default_log_path(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_path() -> String {
    "~/.cortex/adapter.log".to_string()
}

fn default_true() -> bool {
    true
}

/// Failure modes raised while loading the adapter config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// File read failed.
    #[error("config read: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse failed.
    #[error("config parse: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Read the adapter config from `path`. Missing file returns
/// [`AdapterToml::default`] so first-launch is friction-free.
pub fn load_or_default(path: &Path) -> Result<AdapterToml, ConfigError> {
    if !path.exists() {
        return Ok(AdapterToml::default());
    }
    let body = fs::read_to_string(path)?;
    Ok(toml::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = load_or_default(&tmp.path().join("absent.toml")).expect("default");
        assert_eq!(cfg.adapter.timeout_ms, 1500);
        assert_eq!(cfg.adapter.pre_thinking.timeout_ms, 600);
        assert_eq!(cfg.adapter.laws.timeout_ms, 300);
        assert!(cfg.adapter.laws.block_on_critical);
    }

    #[test]
    fn parses_full_example_from_spec_10() {
        let body = r#"
[adapter]
endpoint = "http://127.0.0.1:17010"
api_endpoint = "http://127.0.0.1:17000"
timeout_ms = 1500
queue_bounded = 2048

[adapter.pre_thinking]
enabled = true
max_bundle_kb = 32
timeout_ms = 600

[adapter.laws]
block_on_critical = true
timeout_ms = 300

[adapter.redaction]
extra_patterns = [
  { name = "internal_token", regex = "HIVE_TOKEN_[A-Z0-9]{24}" }
]

[adapter.logging]
level = "info"
path = "~/.cortex/adapter.log"
"#;
        let cfg: AdapterToml = toml::from_str(body).expect("parse");
        assert_eq!(cfg.adapter.queue_bounded, 2048);
        assert_eq!(cfg.adapter.redaction.extra_patterns.len(), 1);
        assert_eq!(cfg.adapter.logging.level, "info");
    }
}
