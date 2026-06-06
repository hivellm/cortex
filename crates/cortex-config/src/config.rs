//! ADR-016 top-level `Config` struct + error type.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sub::{
    AdapterConfig, AnalyzerConfig, AutoMemoryConfig, CanaryConfig, ClassifierConfig,
    ClaudeArchiveConfig, ConsolidatorConfig, CrossProjectConfig, DashboardConfig, DoctorConfig,
    EmbedderConfig, IngestionConfig, McpConfig, MeiliConfig, NexusConfig, PreThinkingConfig,
    RetentionConfig, RulebookConfig, TemporalConfig,
};

/// Current schema version. Bump when a sub-struct gains an
/// incompatible field rename or a backwards-breaking default
/// flip; serde tag-based discrimination then dispatches a
/// migration on load.
pub const SCHEMA_VERSION: &str = "v1";

/// ADR-016 — top-level configuration root. Carries every domain
/// sub-struct + the schema discriminator so future versions
/// migrate cleanly via serde tag dispatch (`#[serde(tag =
/// "schema_version")]` — landed as an enum once `v2` exists; for
/// now the field is a plain `String` defaulted to
/// [`SCHEMA_VERSION`] so the on-disk shape is ready for that
/// flip without touching every consumer).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Schema discriminator. Defaults to [`SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Retention sweep knobs.
    #[serde(default)]
    pub retention: RetentionConfig,
    /// Embedder worker knobs (Vectorizer URL, batch size, …).
    #[serde(default)]
    pub embedder: EmbedderConfig,
    /// Meilisearch full-text indexer knobs.
    #[serde(default)]
    pub meili: MeiliConfig,
    /// Nexus graph store knobs.
    #[serde(default)]
    pub nexus: NexusConfig,
    /// Ingestion router knobs (bind addr, archive root, zstd).
    #[serde(default)]
    pub ingestion: IngestionConfig,
    /// Dashboard / `/v1/*` API surface knobs.
    #[serde(default)]
    pub dashboard: DashboardConfig,
    /// Pre-thinking bundle knobs.
    #[serde(default)]
    pub pre_thinking: PreThinkingConfig,
    /// Rulebook task-loader knobs (single vs. multi-project roots).
    #[serde(default)]
    pub rulebook: RulebookConfig,
    /// Opt-in synthetic canary runner knobs.
    #[serde(default)]
    pub canary: CanaryConfig,
    /// Doctor subcommand knobs (bench gate).
    #[serde(default)]
    pub doctor: DoctorConfig,
    /// Classifier worker health-probe knobs.
    #[serde(default)]
    pub classifier: ClassifierConfig,
    /// Consolidator JSONL fallback knobs.
    #[serde(default)]
    pub consolidator: ConsolidatorConfig,
    /// Auto-memory integration knobs.
    #[serde(default)]
    pub auto_memory: AutoMemoryConfig,
    /// Claude Code CLI analyzer knobs.
    #[serde(default)]
    pub analyzer: AnalyzerConfig,
    /// `cortex-claude-archive` bin knobs.
    #[serde(default)]
    pub claude_archive: ClaudeArchiveConfig,
    /// Claude Code adapter knobs (hook shim + admin port).
    #[serde(default)]
    pub adapter: AdapterConfig,
    /// Phase14i §2.3 — `cortex-mcp-server` per-tool timeout knobs.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Phase18 §3.4 — temporal classifier config (timeline /
    /// branching retrieval).
    #[serde(default)]
    pub temporal: TemporalConfig,
    /// Phase18 §5.3 — cross-project retrieval axis config. Default
    /// OFF per ADR-020.
    #[serde(default)]
    pub cross_project: CrossProjectConfig,
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            retention: RetentionConfig::default(),
            embedder: EmbedderConfig::default(),
            meili: MeiliConfig::default(),
            nexus: NexusConfig::default(),
            ingestion: IngestionConfig::default(),
            dashboard: DashboardConfig::default(),
            pre_thinking: PreThinkingConfig::default(),
            rulebook: RulebookConfig::default(),
            canary: CanaryConfig::default(),
            doctor: DoctorConfig::default(),
            classifier: ClassifierConfig::default(),
            consolidator: ConsolidatorConfig::default(),
            auto_memory: AutoMemoryConfig::default(),
            analyzer: AnalyzerConfig::default(),
            claude_archive: ClaudeArchiveConfig::default(),
            adapter: AdapterConfig::default(),
            mcp: McpConfig::default(),
            temporal: TemporalConfig::default(),
            cross_project: CrossProjectConfig::default(),
        }
    }
}

/// Errors raised during [`Config::load`] / [`Config::load_from`].
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Underlying I/O failure (file read, missing path).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// TOML deserialisation failure.
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    /// JSON deserialisation failure (env-overlay path uses JSON
    /// as the intermediate representation).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Caller supplied a non-v1 `schema_version` and no migration
    /// is registered for it. The error names the value seen so
    /// the operator can pin / downgrade.
    #[error("unsupported schema_version: {0:?} (this build only handles {SCHEMA_VERSION:?})")]
    UnsupportedSchemaVersion(String),
}

impl Config {
    /// Sanity-check the resolved config. Currently only validates
    /// the schema version; future fields (e.g. mutually exclusive
    /// auth combos) hang here.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        Ok(())
    }
}
