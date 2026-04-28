//! Runtime configuration for the classifier worker, parsed from
//! `CORTEX_CLASSIFIER_*` environment variables.

use std::env;

/// Classifier backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierMode {
    /// Pure-rust deterministic fallback. No network.
    Static,
    /// Spawn `claude -p ...` (Claude Code CLI) for each batch.
    Cli,
}

impl ClassifierMode {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cli" | "haiku" | "haiku_cli" => ClassifierMode::Cli,
            _ => ClassifierMode::Static,
        }
    }
}

/// Worker configuration derived from env vars (with defaults).
#[derive(Debug, Clone)]
pub struct ClassifierWorkerConfig {
    /// Synap base URL.
    pub synap_url: String,
    /// Classifier backend mode.
    pub mode: ClassifierMode,
    /// Number of concurrent worker tasks (one Synap consumer pull per task).
    pub workers: usize,
    /// Maximum messages fetched per pull.
    pub batch_size: usize,
    /// Daily budget in US cents (used by `BudgetTracker`).
    pub daily_limit_cents: u64,
    /// Prompt template version label stamped on outputs.
    pub prompt_version: String,
    /// `claude` binary path (only consulted in Cli mode).
    pub claude_bin: String,
    /// Optional model identifier override (Cli mode).
    pub model: String,
    /// Per-batch CLI timeout in seconds. Sonnet-class models with
    /// 32-event batches and a cold subprocess regularly need >30s,
    /// so the previous hard-coded 30s default produced 100% timeout
    /// rates and every classification fell back to the static path.
    /// Override with `CORTEX_CLASSIFIER_CLI_TIMEOUT_SECS`.
    pub cli_timeout_secs: u64,
}

impl Default for ClassifierWorkerConfig {
    fn default() -> Self {
        Self {
            synap_url: "http://127.0.0.1:17003".to_string(),
            mode: ClassifierMode::Static,
            workers: 2,
            batch_size: 32,
            daily_limit_cents: 2000,
            prompt_version: "static-v1".to_string(),
            claude_bin: "claude".to_string(),
            model: "claude-haiku-4-5".to_string(),
            cli_timeout_secs: 90,
        }
    }
}

impl ClassifierWorkerConfig {
    /// Read config from `CORTEX_CLASSIFIER_*` env vars, falling back to defaults.
    pub fn from_env() -> Self {
        let def = Self::default();
        Self {
            synap_url: env::var("CORTEX_CLASSIFIER_SYNAP_URL")
                .or_else(|_| env::var("SYNAP_URL"))
                .unwrap_or(def.synap_url),
            mode: env::var("CORTEX_CLASSIFIER_MODE")
                .map(|s| ClassifierMode::parse(&s))
                .unwrap_or(def.mode),
            workers: parse_usize("CORTEX_CLASSIFIER_WORKERS", def.workers),
            batch_size: parse_usize("CORTEX_CLASSIFIER_BATCH", def.batch_size),
            daily_limit_cents: parse_u64(
                "CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS",
                def.daily_limit_cents,
            ),
            prompt_version: env::var("CORTEX_CLASSIFIER_PROMPT_VERSION")
                .unwrap_or(def.prompt_version),
            claude_bin: env::var("CLAUDE_CODE_BIN").unwrap_or(def.claude_bin),
            model: env::var("CORTEX_CLASSIFIER_MODEL").unwrap_or(def.model),
            cli_timeout_secs: parse_u64(
                "CORTEX_CLASSIFIER_CLI_TIMEOUT_SECS",
                def.cli_timeout_secs,
            ),
        }
    }
}

fn parse_usize(key: &str, fallback: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(fallback)
}

fn parse_u64(key: &str, fallback: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_static() {
        let cfg = ClassifierWorkerConfig::default();
        assert_eq!(cfg.mode, ClassifierMode::Static);
        assert_eq!(cfg.workers, 2);
        assert_eq!(cfg.batch_size, 32);
    }

    #[test]
    fn mode_parse_handles_aliases() {
        assert_eq!(ClassifierMode::parse("cli"), ClassifierMode::Cli);
        assert_eq!(ClassifierMode::parse("CLI"), ClassifierMode::Cli);
        assert_eq!(ClassifierMode::parse("haiku"), ClassifierMode::Cli);
        assert_eq!(ClassifierMode::parse("static"), ClassifierMode::Static);
        assert_eq!(ClassifierMode::parse(""), ClassifierMode::Static);
        assert_eq!(ClassifierMode::parse("garbage"), ClassifierMode::Static);
    }
}
