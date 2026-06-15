//! Runtime configuration for the OpenCode adapter.
//!
//! Reads from the same `cortex_config::Config` as the rest of the
//! adapter stack (ADR-016 env map). The only adapter-specific value
//! is the HTTP bind address, which defaults to `127.0.0.1:17004` —
//! the same port the Claude Code HTTP path uses so both plugins can
//! share a single listener in multi-plugin setups.

/// Default HTTP bind address for the OpenCode hook listener.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:17004";

/// Environment variable that overrides [`DEFAULT_HTTP_BIND`].
pub const ENV_HTTP_BIND: &str = "CORTEX_ADAPTER_HTTP_BIND";

/// Environment variable that disables the OpenCode adapter entirely.
/// When set to any non-empty value the daemon skips spawning the
/// HTTP listener. Useful in CI where the port may be occupied.
pub const ENV_DISABLE: &str = "CORTEX_OPENCODE_DISABLE";

/// Resolved adapter configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the HTTP hook listener binds to.
    pub http_bind: String,
    /// When `true`, the adapter's HTTP listener is not started.
    pub disabled: bool,
}

impl Config {
    /// Load configuration from the environment, falling back to
    /// `cortex_config::Config` then to compiled-in defaults.
    pub fn load() -> Self {
        let http_bind = std::env::var(ENV_HTTP_BIND)
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                cortex_config::Config::load()
                    .ok()
                    .and_then(|c| c.adapter.http_bind)
            })
            .unwrap_or_else(|| DEFAULT_HTTP_BIND.to_string());

        let disabled = std::env::var(ENV_DISABLE)
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        Self {
            http_bind,
            disabled,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            http_bind: DEFAULT_HTTP_BIND.to_string(),
            disabled: false,
        }
    }
}
