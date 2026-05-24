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
//!    [`ConfigBuilder::with_overrides`]).
//! 2. Environment variable — every field carries its legacy
//!    `CORTEX_*` name as a serde alias so existing operator
//!    scripts keep working byte-for-byte.
//! 3. `cortex.toml` at the workspace root or any path passed
//!    explicitly to [`Config::load_from`].
//! 4. Built-in `Default` (every field documents its default).
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
mod sub;

pub use audit::{audit, EnvVarUsage};
pub use config::{Config, ConfigError, SCHEMA_VERSION};
pub use env_map::{env_name_for, KNOWN_ENV_NAMES};
pub use load::default_toml_path;
pub use sub::{
    DashboardConfig, EmbedderConfig, IngestionConfig, MeiliConfig, NexusConfig, PreThinkingConfig,
    RetentionConfig,
};
