//! `cortex-laws` — Laws DSL v1.
//!
//! Loads `.yml` law files from a directory and evaluates tool actions against
//! them. Each law declares a trigger (tool / action / args pattern), a verdict
//! (allow / deny / warn), and the severity of a violation.
//!
//! # Example
//!
//! ```ignore
//! let registry = LawRegistry::load("laws/")?;
//! let ctx = EvalContext { tool: "Bash", action: "run", args: "git commit --no-verify", ..Default::default() };
//! let verdicts = registry.evaluate(&ctx);
//! for v in verdicts { if v.verdict == RuleVerdict::Deny { /* block */ } }
//! ```

use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LawError {
    #[error("I/O error reading laws dir: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error in {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("duplicate law id '{id}' in {path} (first seen in {first_path})")]
    DuplicateId {
        id: String,
        path: String,
        first_path: String,
    },
    #[error("invalid args_match regex '{pattern}' in law '{id}': {source}")]
    InvalidRegex {
        pattern: String,
        id: String,
        #[source]
        source: regex::Error,
    },
}

// ── Schema ────────────────────────────────────────────────────────────────────

/// Severity level for a law violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// Conditions under which the law is triggered. All specified fields must
/// match (AND logic). Unset fields are wildcards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Match the tool name (e.g. `"Bash"`, `"Edit"`). Case-sensitive.
    pub tool: Option<String>,
    /// Match an action qualifier within the tool (e.g. `"git_push"`).
    pub action: Option<String>,
    /// Regex matched against the full args string.
    pub args_match: Option<String>,
}

impl Trigger {
    fn is_empty(&self) -> bool {
        self.tool.is_none() && self.action.is_none() && self.args_match.is_none()
    }
}

/// What the law recommends when triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleVerdict {
    Allow,
    Deny,
    Warn,
}

/// The rule section of a law — verdict + optional guard condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Outcome when the trigger fires.
    pub verdict: RuleVerdict,
    /// Human-readable condition description (informational; not evaluated).
    pub when: Option<String>,
}

/// A single law loaded from a `.yml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Law {
    pub id: String,
    pub title: Option<String>,
    pub severity: Severity,
    pub trigger: Trigger,
    pub rule: Rule,
    pub rationale: String,
}

// ── Evaluation ────────────────────────────────────────────────────────────────

/// Input context provided to [`LawRegistry::evaluate`].
#[derive(Debug, Clone, Default)]
pub struct EvalContext<'a> {
    /// Tool name (e.g. `"Bash"`, `"Edit"`, `"Write"`).
    pub tool: &'a str,
    /// Optional action qualifier (e.g. `"git_commit"`, `"file_edit"`).
    pub action: &'a str,
    /// Full args string to match against `trigger.args_match`.
    pub args: &'a str,
}

/// One verdict emitted when a law's trigger fires.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub law_id: String,
    pub title: Option<String>,
    pub severity: Severity,
    pub verdict: RuleVerdict,
    pub rationale: String,
    pub when: Option<String>,
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Compiled law — trigger fields pre-compiled for fast evaluation.
struct CompiledLaw {
    law: Law,
    args_re: Option<Regex>,
}

impl CompiledLaw {
    fn matches(&self, ctx: &EvalContext) -> bool {
        let t = &self.law.trigger;
        if t.is_empty() {
            return true;
        }
        if let Some(tool) = &t.tool {
            if !ctx.tool.eq_ignore_ascii_case(tool) {
                return false;
            }
        }
        if let Some(action) = &t.action {
            if !ctx.action.eq_ignore_ascii_case(action) {
                return false;
            }
        }
        if let Some(re) = &self.args_re {
            if !re.is_match(ctx.args) {
                return false;
            }
        }
        true
    }
}

/// Holds the loaded and compiled set of laws; thread-safe via `Arc`.
pub struct LawRegistry {
    laws: Vec<CompiledLaw>,
}

impl LawRegistry {
    /// Load every `.yml` file under `dir` recursively.
    ///
    /// Returns [`LawError::DuplicateId`] if any two files share the same `id`.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, LawError> {
        let dir = dir.as_ref();
        let mut laws: Vec<CompiledLaw> = Vec::new();
        let mut seen: HashMap<String, String> = HashMap::new();

        if !dir.exists() {
            return Ok(Self { laws });
        }

        for entry in walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "yml"))
        {
            let path = entry.path();
            let path_str = path.display().to_string();
            let content = std::fs::read_to_string(path)?;
            let law: Law = serde_yaml::from_str(&content).map_err(|e| LawError::Yaml {
                path: path_str.clone(),
                source: e,
            })?;

            if let Some(first) = seen.get(&law.id) {
                return Err(LawError::DuplicateId {
                    id: law.id,
                    path: path_str,
                    first_path: first.clone(),
                });
            }
            seen.insert(law.id.clone(), path_str.clone());

            let args_re = if let Some(pattern) = &law.trigger.args_match {
                Some(Regex::new(pattern).map_err(|e| LawError::InvalidRegex {
                    pattern: pattern.clone(),
                    id: law.id.clone(),
                    source: e,
                })?)
            } else {
                None
            };

            laws.push(CompiledLaw { law, args_re });
        }

        Ok(Self { laws })
    }

    /// Create an empty registry (useful for testing / disabled mode).
    pub fn empty() -> Self {
        Self { laws: Vec::new() }
    }

    /// Evaluate `ctx` against all loaded laws.
    ///
    /// Returns one [`Verdict`] per matching law. Order follows load order
    /// (walkdir lexicographic). Laws with `trigger` fully unset always fire.
    pub fn evaluate(&self, ctx: &EvalContext) -> Vec<Verdict> {
        self.laws
            .iter()
            .filter(|cl| cl.matches(ctx))
            .map(|cl| Verdict {
                law_id: cl.law.id.clone(),
                title: cl.law.title.clone(),
                severity: cl.law.severity.clone(),
                verdict: cl.law.rule.verdict.clone(),
                rationale: cl.law.rationale.clone(),
                when: cl.law.rule.when.clone(),
            })
            .collect()
    }

    /// Number of laws in the registry.
    pub fn len(&self) -> usize {
        self.laws.len()
    }

    pub fn is_empty(&self) -> bool {
        self.laws.is_empty()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::TempDir;

    fn law_file(dir: &TempDir, name: &str, content: &str) {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    const LAW_007_YAML: &str = r#"
id: LAW-007
title: Never bypass pre-commit hooks
severity: critical
trigger:
  tool: Bash
  args_match: "--no-verify"
rule:
  verdict: deny
  when: "when args contain --no-verify"
rationale: "Pre-commit hooks enforce quality gates. Bypassing them masks errors."
"#;

    const LAW_009_WARN_YAML: &str = r#"
id: LAW-009
severity: high
trigger:
  tool: Edit
rule:
  verdict: warn
  when: "when editing files"
rationale: "Edit each file sequentially."
"#;

    const LAW_ALLOW_YAML: &str = r#"
id: LAW-010
severity: info
trigger:
  tool: Read
rule:
  verdict: allow
rationale: "Reads are always safe."
"#;

    const LAW_WILDCARD_YAML: &str = r#"
id: LAW-011
severity: info
trigger: {}
rule:
  verdict: warn
rationale: "Fires on every action."
"#;

    /// Empty laws dir loads cleanly with 0 laws.
    #[test]
    fn empty_dir_loads_zero_laws() {
        let tmp = TempDir::new().unwrap();
        let reg = LawRegistry::load(tmp.path()).unwrap();
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }

    /// Non-existent dir returns empty registry (not an error).
    #[test]
    fn nonexistent_dir_returns_empty() {
        let reg = LawRegistry::load("/tmp/does_not_exist_cortex_laws_test_xyz").unwrap();
        assert!(reg.is_empty());
    }

    /// A well-formed law file loads and evaluates correctly.
    #[test]
    fn law_007_denies_no_verify() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "law-007.yml", LAW_007_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();
        assert_eq!(reg.len(), 1);

        let ctx = EvalContext {
            tool: "Bash",
            action: "",
            args: "git commit --no-verify",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, RuleVerdict::Deny);
        assert_eq!(v[0].severity, Severity::Critical);
        assert_eq!(v[0].law_id, "LAW-007");
    }

    /// Trigger doesn't fire when args don't match the regex.
    #[test]
    fn law_007_allows_normal_commit() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "law-007.yml", LAW_007_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();

        let ctx = EvalContext {
            tool: "Bash",
            action: "",
            args: "git commit -m 'fix: bug'",
        };
        let v = reg.evaluate(&ctx);
        assert!(v.is_empty(), "normal commit must not trigger LAW-007");
    }

    /// Trigger is skipped when the tool doesn't match.
    #[test]
    fn tool_mismatch_does_not_trigger() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "law-007.yml", LAW_007_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();

        let ctx = EvalContext {
            tool: "Edit",
            action: "",
            args: "--no-verify",
        };
        let v = reg.evaluate(&ctx);
        assert!(v.is_empty(), "Edit tool must not trigger Bash-scoped law");
    }

    /// Duplicate id across two files returns an error.
    #[test]
    fn duplicate_id_returns_error() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "a.yml", LAW_007_YAML);
        law_file(&tmp, "b.yml", LAW_007_YAML);
        let result = LawRegistry::load(tmp.path());
        assert!(
            matches!(result, Err(LawError::DuplicateId { .. })),
            "duplicate id must error"
        );
    }

    /// Multiple laws loaded; each fires independently.
    #[test]
    fn multiple_laws_fire_independently() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "law-007.yml", LAW_007_YAML);
        law_file(&tmp, "law-009.yml", LAW_009_WARN_YAML);
        law_file(&tmp, "law-010.yml", LAW_ALLOW_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();
        assert_eq!(reg.len(), 3);

        // Edit tool triggers LAW-009 (warn) only.
        let ctx = EvalContext {
            tool: "Edit",
            action: "",
            args: "src/lib.rs",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].law_id, "LAW-009");
        assert_eq!(v[0].verdict, RuleVerdict::Warn);

        // Read tool triggers LAW-010 (allow) only.
        let ctx = EvalContext {
            tool: "Read",
            action: "",
            args: "src/lib.rs",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, RuleVerdict::Allow);
    }

    /// Wildcard trigger (empty trigger block) fires on every action.
    #[test]
    fn wildcard_trigger_fires_on_all() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "wildcard.yml", LAW_WILDCARD_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();

        for tool in &["Bash", "Edit", "Write", "Read", "Grep"] {
            let ctx = EvalContext {
                tool,
                action: "",
                args: "",
            };
            let v = reg.evaluate(&ctx);
            assert_eq!(v.len(), 1, "{tool} must trigger wildcard law");
        }
    }

    /// Invalid regex in args_match returns a parse error.
    #[test]
    fn invalid_args_match_regex_returns_error() {
        let tmp = TempDir::new().unwrap();
        law_file(
            &tmp,
            "bad.yml",
            r#"
id: LAW-BAD
severity: info
trigger:
  args_match: "["
rule:
  verdict: deny
rationale: "Bad regex."
"#,
        );
        let result = LawRegistry::load(tmp.path());
        assert!(
            matches!(result, Err(LawError::InvalidRegex { .. })),
            "invalid regex must error"
        );
    }

    /// `LawRegistry::empty()` creates a zero-law registry.
    #[test]
    fn empty_registry_evaluates_to_nothing() {
        let reg = LawRegistry::empty();
        let ctx = EvalContext {
            tool: "Bash",
            action: "",
            args: "--no-verify",
        };
        assert!(reg.evaluate(&ctx).is_empty());
    }

    // ── Starter-law fixture tests — one per verdict (§5.2) ──────────────────

    /// Deny verdict fires when the trigger matches.
    #[test]
    fn deny_verdict_returned_when_trigger_matches() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "deny.yml", LAW_007_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();
        let ctx = EvalContext {
            tool: "Bash",
            action: "",
            args: "git push --no-verify",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, RuleVerdict::Deny);
        assert_eq!(v[0].severity, Severity::Critical);
    }

    /// Warn verdict fires when the trigger matches.
    #[test]
    fn warn_verdict_returned_when_trigger_matches() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "warn.yml", LAW_009_WARN_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();
        let ctx = EvalContext {
            tool: "Edit",
            action: "",
            args: "src/main.rs",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, RuleVerdict::Warn);
        assert_eq!(v[0].severity, Severity::High);
    }

    /// Allow verdict fires when the trigger matches.
    #[test]
    fn allow_verdict_returned_when_trigger_matches() {
        let tmp = TempDir::new().unwrap();
        law_file(&tmp, "allow.yml", LAW_ALLOW_YAML);
        let reg = LawRegistry::load(tmp.path()).unwrap();
        let ctx = EvalContext {
            tool: "Read",
            action: "",
            args: "src/main.rs",
        };
        let v = reg.evaluate(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, RuleVerdict::Allow);
        assert_eq!(v[0].severity, Severity::Info);
    }
}
