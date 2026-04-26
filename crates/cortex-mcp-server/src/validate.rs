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
    validate_hooks(plugin_dir, &mut report);
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

fn validate_hooks(plugin_dir: &Path, report: &mut ValidationReport) {
    let hooks_dir = plugin_dir.join("hooks");
    if !hooks_dir.is_dir() {
        report.warnings.push(format!(
            "{} missing — capture is opt-in",
            hooks_dir.display()
        ));
        return;
    }
    // Claude Code's plugin loader expects `hooks.json` at the plugin
    // ROOT, not under `hooks/`. The shim scripts live under `hooks/`
    // but the descriptor itself is colocated with `plugin.json`.
    let descriptor_path = plugin_dir.join("hooks.json");
    let raw = match fs::read_to_string(&descriptor_path) {
        Ok(s) => s,
        Err(e) => {
            report.errors.push(format!(
                "missing {}: {e} — hooks/ directory present but no descriptor at plugin root",
                descriptor_path.display()
            ));
            return;
        }
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(format!("malformed {}: {e}", descriptor_path.display()));
            return;
        }
    };

    let hooks_obj = match parsed.get("hooks").and_then(Value::as_object) {
        Some(o) => o,
        None => {
            report.errors.push(format!(
                "{}: top-level `hooks` object missing",
                descriptor_path.display()
            ));
            return;
        }
    };

    let mut referenced: std::collections::BTreeSet<String> = Default::default();
    for (event, group) in hooks_obj {
        let arr = match group.as_array() {
            Some(a) => a,
            None => {
                report.errors.push(format!(
                    "{}: hooks.{event} must be an array",
                    descriptor_path.display()
                ));
                continue;
            }
        };
        for entry in arr {
            let inner = entry.get("hooks").and_then(Value::as_array);
            let Some(inner) = inner else {
                report.errors.push(format!(
                    "{}: hooks.{event}[].hooks[] missing",
                    descriptor_path.display()
                ));
                continue;
            };
            for h in inner {
                let cmd = h.get("command").and_then(Value::as_str).unwrap_or("");
                if cmd.is_empty() {
                    report.errors.push(format!(
                        "{}: hooks.{event}[].hooks[].command must be a non-empty string",
                        descriptor_path.display()
                    ));
                    continue;
                }
                if let Some(script) = extract_plugin_script(cmd) {
                    let candidate = plugin_dir.join(&script);
                    if !candidate.exists() {
                        report.errors.push(format!(
                            "{}: hooks.{event} references missing script `{script}`",
                            descriptor_path.display()
                        ));
                    } else {
                        referenced.insert(script);
                    }
                }
            }
        }
    }

    // Orphan-script check: every cortex-*.sh in hooks/ must be referenced.
    if let Ok(entries) = fs::read_dir(&hooks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sh") {
                continue;
            }
            let rel = match path.strip_prefix(plugin_dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if !referenced.contains(&rel) {
                report.errors.push(format!(
                    "{}: orphan shim `{rel}` not referenced from hooks.json",
                    descriptor_path.display()
                ));
            }
        }
    }
}

/// Extract the `${CLAUDE_PLUGIN_ROOT}/<rest>` path from a command
/// string. Returns the path **relative to the plugin root**, with
/// forward slashes, ready to join against `plugin_dir`.
fn extract_plugin_script(cmd: &str) -> Option<String> {
    let needle = "${CLAUDE_PLUGIN_ROOT}";
    let start = cmd.find(needle)? + needle.len();
    let tail = &cmd[start..];
    // Skip a leading separator.
    let tail = tail.trim_start_matches(['/', '\\']);
    // Stop at quote / whitespace.
    let end = tail
        .find(|c: char| c == '"' || c.is_whitespace())
        .unwrap_or(tail.len());
    let path = tail[..end].trim().replace('\\', "/");
    if path.is_empty() {
        None
    } else {
        Some(path)
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
        write(
            root.join("hooks/cortex-user-prompt.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        );
        write(
            root.join("hooks.json"),
            r#"{"hooks":{"UserPromptSubmit":[{"matcher":"*","hooks":[{"type":"command","command":"bash \"${CLAUDE_PLUGIN_ROOT}/hooks/cortex-user-prompt.sh\"","timeout":5}]}]}}"#,
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

    #[test]
    fn hooks_descriptor_referencing_missing_script_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        write(
            dir.path().join("hooks.json"),
            r#"{"hooks":{"PostToolUse":[{"matcher":"*","hooks":[{"type":"command","command":"bash \"${CLAUDE_PLUGIN_ROOT}/hooks/cortex-does-not-exist.sh\""}]}]}}"#,
        );
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("missing script `hooks/cortex-does-not-exist.sh`")));
    }

    #[test]
    fn orphan_hook_script_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        write(
            dir.path().join("hooks/cortex-orphan.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        );
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("orphan shim `hooks/cortex-orphan.sh`")));
    }

    #[test]
    fn malformed_hooks_json_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        write(dir.path().join("hooks.json"), "{ not json");
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("malformed") && e.contains("hooks.json")));
    }

    #[test]
    fn hooks_dir_without_descriptor_fails() {
        let dir = tempfile::tempdir().unwrap();
        build_clean_plugin(dir.path());
        fs::remove_file(dir.path().join("hooks.json")).unwrap();
        let report = validate_plugin(dir.path());
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("hooks/ directory present but no descriptor at plugin root")
                || e.contains("missing")));
    }

    #[test]
    fn extract_plugin_script_handles_quoted_path() {
        let cmd = "bash \"${CLAUDE_PLUGIN_ROOT}/hooks/cortex-user-prompt.sh\"";
        assert_eq!(
            extract_plugin_script(cmd).as_deref(),
            Some("hooks/cortex-user-prompt.sh")
        );
    }

    #[test]
    fn extract_plugin_script_handles_unquoted_path() {
        let cmd = "node ${CLAUDE_PLUGIN_ROOT}/dist/hook.js arg1";
        assert_eq!(extract_plugin_script(cmd).as_deref(), Some("dist/hook.js"));
    }

    #[test]
    fn extract_plugin_script_returns_none_when_marker_absent() {
        assert!(extract_plugin_script("bash some-other-script.sh").is_none());
    }
}
