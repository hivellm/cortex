//! Per-repo `cortex.toml` schema + global defaults.
//!
//! Mirrors `docs/specs/09-bootstrap-cli.md` §Per-repo configuration. The
//! parser is serde-driven so adding a new field is a one-line struct
//! change. Defaults match spec 09 §"If `cortex.toml` is missing":
//! junk excludes, symbol-level code chunking, all commits, no PR
//! enrichment, no extra redaction patterns.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Top-level repo configuration parsed from `cortex.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CortexToml {
    /// Wrapper for the `[cortex]` table. Optional so a missing or
    /// minimal `cortex.toml` still parses.
    #[serde(default)]
    pub cortex: CortexSection,
}

/// Body of the `[cortex]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CortexSection {
    /// Override for the repo `id`. Defaults to the repo directory name.
    #[serde(default)]
    pub id: Option<String>,
    /// `[cortex.exclude]` block.
    #[serde(default)]
    pub exclude: ExcludeConfig,
    /// `[cortex.chunking]` block.
    #[serde(default)]
    pub chunking: ChunkingConfig,
    /// `[cortex.redaction]` block.
    #[serde(default)]
    pub redaction: RedactionConfig,
    /// `[cortex.git]` block.
    #[serde(default)]
    pub git: GitConfig,
    /// `[cortex.decisions]` block.
    #[serde(default)]
    pub decisions: PromoteConfig,
    /// `[cortex.laws]` block.
    #[serde(default)]
    pub laws: PromoteConfig,
    /// `[cortex.memories]` block.
    #[serde(default)]
    pub memories: MemoriesConfig,
}

/// `[cortex.exclude]` — paths and extensions the walker drops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludeConfig {
    /// Glob-style path prefixes to drop (e.g. `target/`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// File extensions to drop (without leading dot).
    #[serde(default)]
    pub extensions: Vec<String>,
}

impl Default for ExcludeConfig {
    fn default() -> Self {
        // Spec 09 §"If cortex.toml is missing": exclude common junk.
        Self {
            paths: vec![
                "target/".into(),
                "node_modules/".into(),
                "dist/".into(),
                "tmp/".into(),
                ".git/".into(),
            ],
            extensions: vec![
                "lock".into(),
                "log".into(),
                "pack".into(),
                "png".into(),
                "jpg".into(),
                "jpeg".into(),
                "gif".into(),
                "pdf".into(),
                "zip".into(),
                "gz".into(),
                "tar".into(),
                "exe".into(),
                "dll".into(),
                "so".into(),
                "dylib".into(),
            ],
        }
    }
}

/// `[cortex.chunking]` — code/doc strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// `symbol` | `window` | `auto`.
    #[serde(default = "default_code_strategy")]
    pub code_strategy: String,
    /// `section` | `window`.
    #[serde(default = "default_doc_strategy")]
    pub doc_strategy: String,
}

fn default_code_strategy() -> String {
    "auto".into()
}

fn default_doc_strategy() -> String {
    "section".into()
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            code_strategy: default_code_strategy(),
            doc_strategy: default_doc_strategy(),
        }
    }
}

/// `[cortex.redaction]` — per-repo extra patterns merged into the
/// global redactor catalog for the duration of this repo's walk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedactionConfig {
    /// Extra pattern entries.
    #[serde(default)]
    pub extra_patterns: Vec<ExtraPattern>,
}

/// One redaction pattern carried in `[cortex.redaction.extra_patterns]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraPattern {
    /// Pattern name (used as the redaction token tag).
    pub name: String,
    /// Regex source.
    pub regex: String,
}

/// `[cortex.git]` — git walker options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Whether to walk commits at all.
    #[serde(default = "default_true")]
    pub include_commits: bool,
    /// Whether to enrich with PR bodies via `gh api`.
    #[serde(default)]
    pub include_prs: bool,
    /// Earliest commit to include (`YYYY-MM-DD` or git ref). `None`
    /// walks all history.
    #[serde(default)]
    pub since: Option<String>,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            include_commits: true,
            include_prs: false,
            since: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Shared shape for `[cortex.decisions]` and `[cortex.laws]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromoteConfig {
    /// Glob patterns for files that should produce `*.imported` events.
    #[serde(default)]
    pub promote_patterns: Vec<String>,
}

/// `[cortex.memories]` — files imported as `memory.imported` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoriesConfig {
    /// Glob patterns / paths of memory files.
    #[serde(default)]
    pub import_files: Vec<String>,
}

/// Failure modes raised while loading a `cortex.toml`.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Filesystem read failed.
    #[error("config read: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse failed.
    #[error("config parse: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Load `cortex.toml` from `path`, falling back to defaults when the
/// file is missing.
pub fn load_or_default(path: &Path) -> Result<CortexToml, ConfigError> {
    if !path.exists() {
        return Ok(CortexToml::default());
    }
    let body = fs::read_to_string(path)?;
    Ok(toml::from_str(&body)?)
}

/// Convenience: load `<repo_root>/cortex.toml` or fall back.
pub fn load_for_repo(repo_root: &Path) -> Result<CortexToml, ConfigError> {
    load_or_default(&repo_root.join("cortex.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = load_for_repo(tmp.path()).expect("default load");
        assert!(!cfg.cortex.exclude.paths.is_empty());
        assert!(cfg.cortex.git.include_commits);
        assert!(!cfg.cortex.git.include_prs);
        assert_eq!(cfg.cortex.chunking.code_strategy, "auto");
    }

    #[test]
    fn parses_full_example_from_spec_09() {
        let body = r#"
[cortex]
id = "Vectorizer"

[cortex.exclude]
paths = ["target/", "node_modules/"]
extensions = ["lock", "log"]

[cortex.chunking]
code_strategy = "symbol"
doc_strategy = "section"

[cortex.redaction]
extra_patterns = [
  { name = "internal_token", regex = "HIVE_TOKEN_[A-Z0-9]{24}" }
]

[cortex.git]
include_commits = true
include_prs = true
since = "2019-01-01"

[cortex.decisions]
promote_patterns = ["docs/decisions/*.md", "ADR-*.md"]

[cortex.laws]
promote_patterns = ["rulebook/laws/*.yaml"]

[cortex.memories]
import_files = ["CLAUDE.md", "AGENTS.md"]
"#;
        let cfg: CortexToml = toml::from_str(body).expect("parse spec example");
        assert_eq!(cfg.cortex.id.as_deref(), Some("Vectorizer"));
        assert_eq!(cfg.cortex.exclude.paths.len(), 2);
        assert_eq!(cfg.cortex.chunking.code_strategy, "symbol");
        assert_eq!(cfg.cortex.redaction.extra_patterns.len(), 1);
        assert!(cfg.cortex.git.include_prs);
        assert_eq!(cfg.cortex.git.since.as_deref(), Some("2019-01-01"));
        assert_eq!(cfg.cortex.decisions.promote_patterns.len(), 2);
        assert_eq!(cfg.cortex.memories.import_files.len(), 2);
    }
}
