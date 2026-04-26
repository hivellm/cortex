//! Lints the `cortex-plugin/` directory against the Claude Code
//! plugin reference. Spec 18 §CI validates the asset tree.
//!
//! Runs as `cortex-mcp-server validate <plugin-dir>`. Exits non-zero
//! when a required file is missing or malformed; the goal is for CI
//! to refuse to ship a plugin with corrupted assets.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

/// Outcome of running [`validate_plugin`]. Empty `errors` ⇒ pass.
#[derive(Debug, Default)]
pub struct ValidationReport {
    /// Hard errors that fail the validation.
    pub errors: Vec<String>,
    /// Soft warnings that print but don't fail CI.
    pub warnings: Vec<String>,
}

impl ValidationReport {
    /// `true` when no hard errors were recorded.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    name: String,
    version: String,
    description: Option<String>,
    #[serde(default)]
    author: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct McpConfig {
    #[serde(rename = "mcpServers")]
    mcp_servers: serde_json::Map<String, Value>,
}

/// Run the lint against `plugin_dir` and return the accumulated report.
pub fn validate_plugin(plugin_dir: &Path) -> ValidationReport {
    let mut report = ValidationReport::default();

    if !plugin_dir.is_dir() {
        report
            .errors
            .push(format!("not a directory: {}", plugin_dir.display()));
        return report;
    }

    validate_manifest(plugin_dir, &mut report);
    validate_mcp_config(plugin_dir, &mut report);
    validate_marketplace(plugin_dir, &mut report);
    validate_skills(plugin_dir, &mut report);
    validate_agents(plugin_dir, &mut report);
    validate_commands(plugin_dir, &mut report);
    validate_readme(plugin_dir, &mut report);

    report
}

fn validate_manifest(plugin_dir: &Path, report: &mut ValidationReport) {
    let path = plugin_dir.join(".claude-plugin").join("plugin.json");
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            report
                .errors
                .push(format!("missing {}: {e}", path.display()));
            return;
        }
    };
    let parsed: PluginManifest = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("malformed {}: {e}", path.display()));
            return;
        }
    };
    if parsed.name.trim().is_empty() {
        report
            .errors
            .push(format!("{}: name must not be empty", path.display()));
    }
    if parsed.version.trim().is_empty() {
        report
            .errors
            .push(format!("{}: version must not be empty", path.display()));
    }
    if parsed
        .description
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        report
            .warnings
            .push(format!("{}: description is empty", path.display()));
    }
    if parsed.author.is_none() {
        report
            .warnings
            .push(format!("{}: author block missing", path.display()));
    }
}

fn validate_mcp_config(plugin_dir: &Path, report: &mut ValidationReport) {
    let path = plugin_dir.join(".mcp.json");
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            report
                .errors
                .push(format!("missing {}: {e}", path.display()));
            return;
        }
    };
    let parsed: McpConfig = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("malformed {}: {e}", path.display()));
            return;
        }
    };
    if !parsed.mcp_servers.contains_key("cortex") {
        report.errors.push(format!(
            "{}: mcpServers must define an entry named \"cortex\"",
            path.display()
        ));
        return;
    }
    let cortex = &parsed.mcp_servers["cortex"];
    if cortex.get("command").and_then(Value::as_str).is_none() {
        report.errors.push(format!(
            "{}: mcpServers.cortex.command must be a string",
            path.display()
        ));
    }
}

fn validate_marketplace(plugin_dir: &Path, report: &mut ValidationReport) {
    let path = plugin_dir.join(".claude-plugin").join("marketplace.json");
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            report.warnings.push(format!(
                "{} missing — marketplace install disabled",
                path.display()
            ));
            return;
        }
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("malformed {}: {e}", path.display()));
            return;
        }
    };
    if parsed.get("plugins").and_then(Value::as_array).is_none() {
        report
            .errors
            .push(format!("{}: plugins[] missing", path.display()));
    }
}

fn validate_skills(plugin_dir: &Path, report: &mut ValidationReport) {
    let dir = plugin_dir.join("skills");
    let mut found = false;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                report.errors.push(format!(
                    "{}: SKILL.md missing inside skill directory",
                    path.display()
                ));
                continue;
            }
            check_yaml_frontmatter(&skill_md, &["name", "description"], report);
            found = true;
        }
    }
    if !found {
        report
            .warnings
            .push(format!("{}: no skill directories found", dir.display()));
    }
}

fn validate_agents(plugin_dir: &Path, report: &mut ValidationReport) {
    let dir = plugin_dir.join("agents");
    let mut found = false;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            check_yaml_frontmatter(&path, &["name", "description"], report);
            found = true;
        }
    }
    if !found {
        report
            .warnings
            .push(format!("{}: no agents found", dir.display()));
    }
}

fn validate_commands(plugin_dir: &Path, report: &mut ValidationReport) {
    let dir = plugin_dir.join("commands");
    let mut found = false;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            check_yaml_frontmatter(&path, &["description"], report);
            found = true;
        }
    }
    if !found {
        report
            .warnings
            .push(format!("{}: no commands found", dir.display()));
    }
}

fn validate_readme(plugin_dir: &Path, report: &mut ValidationReport) {
    let path = plugin_dir.join("README.md");
    if !path.exists() {
        report.warnings.push(format!("{} missing", path.display()));
    }
}

fn check_yaml_frontmatter(path: &Path, required: &[&str], report: &mut ValidationReport) {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            report
                .errors
                .push(format!("cannot read {}: {e}", path.display()));
            return;
        }
    };
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        report.errors.push(format!(
            "{}: missing YAML frontmatter (file must start with `---`)",
            path.display()
        ));
        return;
    }
    let after = &trimmed[3..];
    let close = match after.find("\n---") {
        Some(i) => i,
        None => {
            report.errors.push(format!(
                "{}: YAML frontmatter is unterminated",
                path.display()
            ));
            return;
        }
    };
    let header = &after[..close];
    for key in required {
        // naive but tolerant: look for `key:` at column 0 of any line.
        let needle = format!("{key}:");
        let has_key = header.lines().any(|l| l.trim_start().starts_with(&needle));
        if !has_key {
            report.errors.push(format!(
                "{}: frontmatter missing required key `{key}`",
                path.display()
            ));
        }
    }
}

/// Helper used by the binary: print the report and return an exit
/// code (0 when `is_ok`, 1 otherwise).
pub fn print_report(report: &ValidationReport, plugin_dir: &Path) -> i32 {
    if report.errors.is_empty() && report.warnings.is_empty() {
        eprintln!("✓ {} validates clean", plugin_dir.display());
        return 0;
    }
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    for e in &report.errors {
        eprintln!("error: {e}");
    }
    if report.errors.is_empty() {
        eprintln!("✓ {} validates with warnings", plugin_dir.display());
        0
    } else {
        eprintln!(
            "✗ {} failed validation: {} error(s)",
            plugin_dir.display(),
            report.errors.len()
        );
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write(path: PathBuf, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn build_clean_plugin(root: &Path) {
        write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"cortex","description":"x","version":"0.1.0","author":{"name":"HiveLLM"}}"#,
        );
        write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{"name":"hivellm-cortex","plugins":[{"name":"cortex","version":"0.1.0"}]}"#,
        );
        write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"cortex":{"command":"cortex-mcp-server","args":["serve"]}}}"#,
        );
        write(root.join("README.md"), "# cortex plugin");
        write(
            root.join("skills/cortex-context/SKILL.md"),
            "---\nname: cortex-context\ndescription: ctx\n---\nbody",
        );
        write(
            root.join("agents/cortex-historian.md"),
            "---\nname: cortex-historian\ndescription: hist\n---\nbody",
        );
        write(
            root.join("commands/cortex-status.md"),
            "---\ndescription: show status\n---\nbody",
        );
    }

    #[test]
    fn clean_tree_validates() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        let report = validate_plugin(dir.path());
        assert!(report.is_ok(), "errors: {:?}", report.errors);
    }

    #[test]
    fn missing_manifest_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        fs::remove_file(dir.path().join(".claude-plugin/plugin.json")).unwrap();
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.contains("plugin.json")));
    }

    #[test]
    fn missing_mcp_section_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        write(dir.path().join(".mcp.json"), r#"{"mcpServers":{}}"#);
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("must define an entry named \"cortex\"")));
    }

    #[test]
    fn agent_without_frontmatter_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        fs::write(
            dir.path().join("agents/cortex-historian.md"),
            "no frontmatter here",
        )
        .unwrap();
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("missing YAML frontmatter")));
    }

    #[test]
    fn skill_directory_without_skill_md_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        fs::remove_file(dir.path().join("skills/cortex-context/SKILL.md")).unwrap();
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.contains("SKILL.md missing")));
    }

    #[test]
    fn nonexistent_root_fails() {
        let report = validate_plugin(Path::new("/this/path/does/not/exist"));
        assert!(!report.is_ok());
    }
}
