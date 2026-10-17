//! `cortex-config` — ADR-016 typed Config crate.
//!
//! Single source of truth for every `CORTEX_*` knob the workspace
//! reads. Replaces the 237 scattered `std::env::var("CORTEX_*")`
//! call sites with `Config::load() -> Result<Config>` + typed
//! field access (`cfg.embedder.vectorizer_url` instead of
//! `env::var("CORTEX_EMBEDDER_VECTORIZER_URL")?`).
//!
//! ## Resolution precedence
//!
//! Each field resolves in priority order (highest wins):
//!
//! 1. CLI flag (binaries that thread one through pass it to
//!    [`Config::load_with_cli`]).
//! 2. Environment variable — every field carries its legacy
//!    `CORTEX_*` name as a serde alias so existing operator
//!    scripts keep working byte-for-byte.
//! 3. Nonempty `CORTEX_CONFIG_FILE` selects the runtime TOML file.
//! 4. Otherwise, an existing user `~/.cortex/config.toml` (directory
//!    overridden by `CORTEX_HOME`). Windows uses `USERPROFILE`, falling back
//!    to `HOME`; other platforms use `HOME`. Empty values are unset.
//! 5. Otherwise, `./cortex.toml` relative to the current working directory.
//! 6. Built-in `Default` (every field documents its default).
//!
//! Discovery selects one file; it never merges repo values into a selected
//! user or explicit config. Missing explicit files use defaults plus env;
//! unreadable or malformed selected files return errors. [`Config::load_from`]
//! and [`Config::load_with_cli`] read only their explicit file path, keeping
//! tests and repo-specific callers isolated from user configuration. Repository
//! bootstrap `[cortex]` configuration uses its separate parser unchanged.
//!
//! ## Schema evolution
//!
//! Top-level [`Config`] is serde-versioned via a `schema_version`
//! discriminator (current `"v1"`). A future migration ships a `v2`
//! variant + a `migrate_v1_to_v2` transformer; pre-existing
//! `cortex.toml` files keep deserialising into `v1` and load
//! without operator action.
//!
//! ## Operator-facing env names
//!
//! Each sub-struct field carries the legacy env var as a
//! `#[serde(rename / alias)]` so the env layer (which feeds
//! serde) sees the same names operators have always used:
//! `CORTEX_EMBEDDER_VECTORIZER_URL`,
//! `CORTEX_FULLTEXT_MEILI_URL`, `CORTEX_GRAPH_NEXUS_URL`,
//! etc.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod audit;
mod config;
mod env_map;
mod load;
mod runtime;
mod sub;

pub use audit::{audit, EnvVarUsage};
pub use config::{Config, ConfigError, SCHEMA_VERSION};
pub use env_map::{env_name_for, KNOWN_ENV_NAMES};
pub use load::{cortex_home, default_toml_path};
pub use runtime::{RuntimeConfig, RuntimeProfile};

/// Read the adapter emergency stop before loading potentially unavailable TOML.
/// This narrow bootstrap path performs no filesystem access; normal settings
/// still resolve through [`Config::load`].
pub fn adapter_disabled_by_environment() -> bool {
    matches!(
        std::env::var("CORTEX_ADAPTER_DISABLE").as_deref(),
        Ok("1" | "true")
    )
}

pub use sub::{
    AccessControlConfig, AdapterConfig, AnalyzerConfig, AutoMemoryConfig, CanaryConfig,
    ClassifierConfig, ClaudeArchiveConfig, ConsolidatorConfig, CrossProjectConfig, DashboardConfig,
    DoctorConfig, EmbedderConfig, IngestionConfig, McpConfig, MeiliConfig, NexusConfig,
    PreThinkingConfig, RerankerConfig, RetentionConfig, RulebookConfig, TemporalConfig,
    VerifyConfig,
};
