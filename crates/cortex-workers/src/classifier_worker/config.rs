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
    /// Worker is disabled — `main` logs an INFO line and exits 0
    /// without consuming the input streams. Operator escape hatch
    /// for "the LLM-backed classifier isn't earning its cost
    /// today, but I don't want to delete the deployment plumbing".
    /// Set `CORTEX_CLASSIFIER_MODE=disabled` (or `off`) in the
    /// environment.
    Disabled,
}

impl ClassifierMode {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cli" | "haiku" | "haiku_cli" => ClassifierMode::Cli,
            "disabled" | "off" | "none" => ClassifierMode::Disabled,
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
        let cfg = cortex_config::Config::load().unwrap_or_default();
        Self {
            synap_url: cfg
                .classifier
                .synap_url
                .clone()
                .or_else(|| env::var("SYNAP_URL").ok())
                .unwrap_or(def.synap_url),
            mode: cfg
                .classifier
                .mode
                .as_deref()
                .map(ClassifierMode::parse)
                .unwrap_or(def.mode),
            workers: parse_usize("CORTEX_CLASSIFIER_WORKERS", def.workers),
            batch_size: parse_usize("CORTEX_CLASSIFIER_BATCH", def.batch_size),
            daily_limit_cents: parse_u64(
                "CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS",
                def.daily_limit_cents,
            ),
            prompt_version: cfg
                .classifier
                .prompt_version
                .clone()
                .unwrap_or(def.prompt_version),
            claude_bin: env::var("CLAUDE_CODE_BIN").unwrap_or(def.claude_bin),
            model: cfg.classifier.model.clone().unwrap_or(def.model),
            cli_timeout_secs: parse_u64("CORTEX_CLASSIFIER_CLI_TIMEOUT_SECS", def.cli_timeout_secs),
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

    #[test]
    fn mode_parse_recognises_disabled_aliases() {
        // `disabled` is the canonical operator escape hatch
        // (`docs/ops/...`); `off` and `none` are friendlier
        // aliases since both spellings show up in operator
        // env-var checklists.
        assert_eq!(ClassifierMode::parse("disabled"), ClassifierMode::Disabled);
        assert_eq!(ClassifierMode::parse("DISABLED"), ClassifierMode::Disabled);
        assert_eq!(ClassifierMode::parse("off"), ClassifierMode::Disabled);
        assert_eq!(ClassifierMode::parse("none"), ClassifierMode::Disabled);
        assert_eq!(
            ClassifierMode::parse(" disabled "),
            ClassifierMode::Disabled
        );
    }

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const ALL_KEYS: &[&str] = &[
        "CORTEX_CLASSIFIER_SYNAP_URL",
        "SYNAP_URL",
        "CORTEX_CLASSIFIER_MODE",
        "CORTEX_CLASSIFIER_WORKERS",
        "CORTEX_CLASSIFIER_BATCH",
        "CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS",
        "CORTEX_CLASSIFIER_PROMPT_VERSION",
        "CLAUDE_CODE_BIN",
        "CORTEX_CLASSIFIER_MODEL",
        "CORTEX_CLASSIFIER_CLI_TIMEOUT_SECS",
    ];
    fn clear_all() {
        for k in ALL_KEYS {
            env::remove_var(k);
        }
    }

    #[test]
    fn from_env_returns_defaults_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        let cfg = ClassifierWorkerConfig::from_env();
        let def = ClassifierWorkerConfig::default();
        assert_eq!(cfg.synap_url, def.synap_url);
        assert_eq!(cfg.mode, def.mode);
        assert_eq!(cfg.workers, def.workers);
        assert_eq!(cfg.batch_size, def.batch_size);
        assert_eq!(cfg.cli_timeout_secs, def.cli_timeout_secs);
    }

    #[test]
    fn from_env_overrides_each_field() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("CORTEX_CLASSIFIER_SYNAP_URL", "http://syn:9");
        env::set_var("CORTEX_CLASSIFIER_MODE", "cli");
        env::set_var("CORTEX_CLASSIFIER_WORKERS", "4");
        env::set_var("CORTEX_CLASSIFIER_BATCH", "16");
        env::set_var("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS", "5000");
        env::set_var("CORTEX_CLASSIFIER_PROMPT_VERSION", "v9");
        env::set_var("CLAUDE_CODE_BIN", "/usr/local/bin/claude");
        env::set_var("CORTEX_CLASSIFIER_MODEL", "claude-haiku-test");
        env::set_var("CORTEX_CLASSIFIER_CLI_TIMEOUT_SECS", "120");

        let cfg = ClassifierWorkerConfig::from_env();
        assert_eq!(cfg.synap_url, "http://syn:9");
        assert_eq!(cfg.mode, ClassifierMode::Cli);
        assert_eq!(cfg.workers, 4);
        assert_eq!(cfg.batch_size, 16);
        assert_eq!(cfg.daily_limit_cents, 5000);
        assert_eq!(cfg.prompt_version, "v9");
        assert_eq!(cfg.claude_bin, "/usr/local/bin/claude");
        assert_eq!(cfg.model, "claude-haiku-test");
        assert_eq!(cfg.cli_timeout_secs, 120);

        clear_all();
    }

    #[test]
    fn synap_url_falls_back_to_synap_url_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_all();
        env::set_var("SYNAP_URL", "http://generic:9000");
        let cfg = ClassifierWorkerConfig::from_env();
        assert_eq!(cfg.synap_url, "http://generic:9000");
        // Specific var beats the generic one.
        env::set_var("CORTEX_CLASSIFIER_SYNAP_URL", "http://specific:9100");
        let cfg2 = ClassifierWorkerConfig::from_env();
        assert_eq!(cfg2.synap_url, "http://specific:9100");
        clear_all();
    }

    #[test]
    fn parse_usize_rejects_zero() {
        let _g = ENV_LOCK.lock().unwrap();
        env::set_var("CORTEX_CLASSIFIER_WORKERS", "0");
        // 0 is filtered out — falls back to default 2.
        assert_eq!(parse_usize("CORTEX_CLASSIFIER_WORKERS", 2), 2);
        env::set_var("CORTEX_CLASSIFIER_WORKERS", "abc");
        assert_eq!(parse_usize("CORTEX_CLASSIFIER_WORKERS", 7), 7);
        env::remove_var("CORTEX_CLASSIFIER_WORKERS");
    }

    #[test]
    fn parse_u64_rejects_garbage() {
        let _g = ENV_LOCK.lock().unwrap();
        env::set_var("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS", "??");
        assert_eq!(parse_u64("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS", 100), 100);
        env::set_var("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS", "999");
        assert_eq!(parse_u64("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS", 100), 999);
        env::remove_var("CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS");
    }
}
