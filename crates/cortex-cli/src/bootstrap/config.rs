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
    /// `[cortex.analyses]` block — promote audit / deep-analysis
    /// reports (typically under `docs/analysis/**/*.md`) to a
    /// dedicated `analysis.imported` event kind so they fan out
    /// to the analyses index/collection/sub-graph instead of the
    /// generic docs bucket.
    #[serde(default)]
    pub analyses: PromoteConfig,
    /// `[cortex.memories]` block.
    #[serde(default)]
    pub memories: MemoriesConfig,
    /// `[cortex.classification]` block — path→level+compartments rules.
    #[serde(default)]
    pub classification: ClassificationConfig,
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
    /// File size ceiling in bytes. Files above this are dropped with
    /// `reason: "oversize"` instead of being shipped to Synap. The
    /// default 8 MB sits below Synap's 10 MB request-body limit with
    /// headroom for envelope wrapping and JSON expansion (binary
    /// chars escape to multi-byte sequences). Repos with vendored
    /// blobs (Tml/docs/docs.json hit 12 MB on 2026-04-27) override
    /// to a smaller value.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
}

/// Default file size ceiling for the bootstrap walker — see the
/// `[cortex.exclude].max_file_bytes` field. `pub const` so callers
/// (and the [`super::walker::MAX_FILE_BYTES`] re-export) can reach
/// it from `const` contexts.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Serde wrapper for [`DEFAULT_MAX_FILE_BYTES`] — `serde(default)`
/// only accepts a function path, so this trampoline lets the default
/// flow through.
pub fn default_max_file_bytes() -> u64 {
    DEFAULT_MAX_FILE_BYTES
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
            max_file_bytes: default_max_file_bytes(),
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
    /// Phase11k §3.2 — optional regex string applied to the first
    /// token of `## ` headings inside Law-classified files. When set
    /// AND a heading's first token matches, the bootstrap walker
    /// emits one `law.imported` envelope per match (with `law_id` =
    /// the matched token) instead of a single envelope for the whole
    /// file. Unset = single-law-per-file behaviour preserved.
    #[serde(default)]
    pub extract_pattern: Option<String>,
}

/// `[cortex.memories]` — files imported as `memory.imported` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoriesConfig {
    /// Glob patterns / paths of memory files.
    #[serde(default)]
    pub import_files: Vec<String>,
}

/// `[cortex.classification]` — glob-to-level+compartments path rules.
///
/// Rules are matched in declaration order; first match wins. An absent
/// `[cortex.classification]` section means "no path-based rules" — the
/// stamper falls back to the global default level (phase21 §3.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassificationConfig {
    /// Path-rule entries (`pattern`, `level`, `compartments`).
    #[serde(default)]
    pub rules: Vec<ClassificationRule>,
}

/// One entry in `[cortex.classification].rules`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationRule {
    /// Glob pattern matched against the repo-relative path (e.g.
    /// `"docs/financial/**"`).
    pub pattern: String,
    /// Sensitivity level label (`public` | `internal` | `confidential` |
    /// `restricted`). Validated at parse time — unknown names are an error.
    pub level: String,
    /// Optional compartments this rule asserts (e.g. `["financial"]`).
    #[serde(default)]
    pub compartments: Vec<String>,
}

/// Valid sensitivity level labels (corresponding to ordinals 0–3).
pub const VALID_LEVEL_LABELS: &[&str] = &["public", "internal", "confidential", "restricted"];

/// Return the ordinal value (0–3) for a level label, or `None` if unknown.
pub fn level_label_to_ordinal(label: &str) -> Option<u8> {
    match label {
        "public" => Some(0),
        "internal" => Some(1),
        "confidential" => Some(2),
        "restricted" => Some(3),
        _ => None,
    }
}

/// Error returned when a `ClassificationRule` carries an unknown level.
#[derive(Debug, Clone)]
pub struct UnknownLevelError {
    /// The unrecognised level string.
    pub label: String,
    /// Zero-based index of the offending rule in the `rules` list.
    pub rule_index: usize,
}

impl std::fmt::Display for UnknownLevelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "classification rule[{}]: unknown level {:?} (valid: {})",
            self.rule_index,
            self.label,
            VALID_LEVEL_LABELS.join(", ")
        )
    }
}

impl std::error::Error for UnknownLevelError {}

impl ClassificationConfig {
    /// Validate all rules in this config, returning the first unknown-level
    /// error found, or `Ok(())` when all levels are valid.
    pub fn validate(&self) -> Result<(), UnknownLevelError> {
        for (i, rule) in self.rules.iter().enumerate() {
            if level_label_to_ordinal(&rule.level).is_none() {
                return Err(UnknownLevelError {
                    label: rule.level.clone(),
                    rule_index: i,
                });
            }
        }
        Ok(())
    }

    /// Apply path rules to `rel_path`, returning the `(level_ordinal,
    /// compartments)` floor stamped by matching rules.
    ///
    /// Matching is performed with `globset`. When multiple rules match,
    /// the rule with the **longest pattern** string wins (a proxy for
    /// most-specific match). On exact-length ties the **most-restrictive**
    /// (highest ordinal) rule wins. Returns `None` when no rule matches.
    pub fn apply_to_path(&self, rel_path: &str) -> Option<(u8, Vec<String>)> {
        use globset::Glob;

        let mut best: Option<(usize, u8, Vec<String>)> = None; // (pattern_len, level, compartments)

        for rule in &self.rules {
            let Some(ordinal) = level_label_to_ordinal(&rule.level) else {
                continue;
            };
            let matcher = match Glob::new(&rule.pattern).map(|g| g.compile_matcher()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if matcher.is_match(rel_path) {
                let pat_len = rule.pattern.len();
                let better = match &best {
                    None => true,
                    Some((best_len, best_ord, _)) => {
                        pat_len > *best_len
                            || (pat_len == *best_len && ordinal > *best_ord)
                    }
                };
                if better {
                    best = Some((pat_len, ordinal, rule.compartments.clone()));
                }
            }
        }

        best.map(|(_, level, compartments)| (level, compartments))
    }
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

[cortex.analyses]
promote_patterns = ["docs/analysis/**/*.md"]

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
        assert_eq!(cfg.cortex.analyses.promote_patterns.len(), 1);
        assert_eq!(
            cfg.cortex.analyses.promote_patterns[0],
            "docs/analysis/**/*.md"
        );
        assert_eq!(cfg.cortex.memories.import_files.len(), 2);
    }

    #[test]
    fn analyses_block_defaults_to_empty() {
        let cfg: CortexToml = toml::from_str("").expect("parse empty");
        assert!(cfg.cortex.analyses.promote_patterns.is_empty());
    }

    #[test]
    fn classification_block_defaults_to_empty_rules() {
        let cfg: CortexToml = toml::from_str("").expect("parse empty");
        assert!(cfg.cortex.classification.rules.is_empty());
        assert!(cfg.cortex.classification.validate().is_ok());
    }

    #[test]
    fn classification_rules_parse_correctly() {
        let body = r#"
[cortex.classification]
rules = [
  { pattern = "docs/financial/**", level = "confidential", compartments = ["financial"] },
  { pattern = "hr/**", level = "restricted", compartments = ["hr", "customer_pii"] },
  { pattern = "public/**", level = "public" },
]
"#;
        let cfg: CortexToml = toml::from_str(body).expect("parse classification");
        let rules = &cfg.cortex.classification.rules;
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].pattern, "docs/financial/**");
        assert_eq!(rules[0].level, "confidential");
        assert_eq!(rules[0].compartments, vec!["financial"]);
        assert_eq!(rules[1].compartments, vec!["hr", "customer_pii"]);
        assert_eq!(rules[2].compartments.len(), 0, "no compartments defaults to empty vec");
        assert!(cfg.cortex.classification.validate().is_ok());
    }

    #[test]
    fn classification_validate_rejects_unknown_level() {
        let body = r#"
[cortex.classification]
rules = [
  { pattern = "docs/**", level = "top-secret" },
]
"#;
        let cfg: CortexToml = toml::from_str(body).expect("parse");
        let err = cfg.cortex.classification.validate().expect_err("should fail");
        assert_eq!(err.rule_index, 0);
        assert_eq!(err.label, "top-secret");
        assert!(err.to_string().contains("top-secret"));
        assert!(err.to_string().contains("public"));
    }

    #[test]
    fn level_label_to_ordinal_covers_all_valid_labels() {
        assert_eq!(level_label_to_ordinal("public"), Some(0));
        assert_eq!(level_label_to_ordinal("internal"), Some(1));
        assert_eq!(level_label_to_ordinal("confidential"), Some(2));
        assert_eq!(level_label_to_ordinal("restricted"), Some(3));
        assert_eq!(level_label_to_ordinal("secret"), None);
        assert_eq!(level_label_to_ordinal(""), None);
    }

    #[test]
    fn apply_to_path_returns_none_when_no_rules() {
        let cfg = ClassificationConfig::default();
        assert!(cfg.apply_to_path("docs/financial/report.md").is_none());
    }

    #[test]
    fn apply_to_path_matches_glob_pattern() {
        let cfg: ClassificationConfig = toml::from_str(
            r#"rules = [
              { pattern = "docs/financial/**", level = "confidential", compartments = ["financial"] },
            ]"#,
        )
        .expect("parse");
        let hit = cfg
            .apply_to_path("docs/financial/q1-2026.md")
            .expect("should match");
        assert_eq!(hit.0, 2, "confidential = 2");
        assert_eq!(hit.1, vec!["financial"]);
        assert!(cfg.apply_to_path("docs/legal/note.md").is_none());
    }

    #[test]
    fn apply_to_path_longest_pattern_wins_over_shorter() {
        // docs/financial/restricted/** is more specific than docs/**
        let cfg: ClassificationConfig = toml::from_str(
            r#"rules = [
              { pattern = "docs/**", level = "internal", compartments = [] },
              { pattern = "docs/financial/**", level = "confidential", compartments = ["financial"] },
            ]"#,
        )
        .expect("parse");
        let (level, comps) = cfg
            .apply_to_path("docs/financial/report.md")
            .expect("match");
        assert_eq!(level, 2, "longer pattern wins → confidential=2");
        assert_eq!(comps, vec!["financial"]);
    }

    #[test]
    fn apply_to_path_most_restrictive_wins_on_pattern_length_tie() {
        // Two rules with same-length pattern but different levels.
        let cfg: ClassificationConfig = toml::from_str(
            r#"rules = [
              { pattern = "a/**", level = "public" },
              { pattern = "a/**", level = "restricted" },
            ]"#,
        )
        .expect("parse");
        let (level, _) = cfg.apply_to_path("a/b/c.rs").expect("match");
        assert_eq!(level, 3, "most restrictive (restricted=3) wins on tie");
    }

    #[test]
    fn validate_reports_first_bad_rule_index() {
        let body = r#"
[cortex.classification]
rules = [
  { pattern = "a/**", level = "public" },
  { pattern = "b/**", level = "internal" },
  { pattern = "c/**", level = "UNKNOWN" },
  { pattern = "d/**", level = "public" },
]
"#;
        let cfg: CortexToml = toml::from_str(body).expect("parse");
        let err = cfg.cortex.classification.validate().expect_err("should fail");
        assert_eq!(err.rule_index, 2);
    }
}
