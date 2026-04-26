//! Install / uninstall framework — patches `~/.claude/settings.json`
//! and ships the hook shim scripts. Spec 10 §Install / uninstall.
//!
//! The settings patch is **idempotent** by design: it scans for an
//! existing `cortex` block by name and replaces it byte-identically
//! when re-running. Uninstall removes only the cortex-owned hook
//! entries, preserving any non-cortex hooks the user has wired up.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

/// One hook shim file we ship.
#[derive(Debug, Clone, Copy)]
pub struct HookShim {
    /// Hook discriminator name as Claude Code labels it.
    pub hook_name: &'static str,
    /// Bash shim file name (Linux/macOS).
    pub sh_filename: &'static str,
    /// PowerShell shim file name (Windows).
    pub ps1_filename: &'static str,
    /// Bash shim source baked at build time.
    pub sh_source: &'static str,
    /// PowerShell shim source baked at build time.
    pub ps1_source: &'static str,
}

/// Every hook the adapter wires up.
pub const HOOK_SHIMS: &[HookShim] = &[
    HookShim {
        hook_name: "SessionStart",
        sh_filename: "cortex-session-start.sh",
        ps1_filename: "cortex-session-start.ps1",
        sh_source: include_str!("../hooks/cortex-session-start.sh"),
        ps1_source: include_str!("../hooks/cortex-session-start.ps1"),
    },
    HookShim {
        hook_name: "UserPromptSubmit",
        sh_filename: "cortex-user-prompt.sh",
        ps1_filename: "cortex-user-prompt.ps1",
        sh_source: include_str!("../hooks/cortex-user-prompt.sh"),
        ps1_source: include_str!("../hooks/cortex-user-prompt.ps1"),
    },
    HookShim {
        hook_name: "PreToolUse",
        sh_filename: "cortex-pre-tool.sh",
        ps1_filename: "cortex-pre-tool.ps1",
        sh_source: include_str!("../hooks/cortex-pre-tool.sh"),
        ps1_source: include_str!("../hooks/cortex-pre-tool.ps1"),
    },
    HookShim {
        hook_name: "PostToolUse",
        sh_filename: "cortex-post-tool.sh",
        ps1_filename: "cortex-post-tool.ps1",
        sh_source: include_str!("../hooks/cortex-post-tool.sh"),
        ps1_source: include_str!("../hooks/cortex-post-tool.ps1"),
    },
    HookShim {
        hook_name: "Stop",
        sh_filename: "cortex-stop.sh",
        ps1_filename: "cortex-stop.ps1",
        sh_source: include_str!("../hooks/cortex-stop.sh"),
        ps1_source: include_str!("../hooks/cortex-stop.ps1"),
    },
    HookShim {
        hook_name: "SubagentStop",
        sh_filename: "cortex-subagent-stop.sh",
        ps1_filename: "cortex-subagent-stop.ps1",
        sh_source: include_str!("../hooks/cortex-subagent-stop.sh"),
        ps1_source: include_str!("../hooks/cortex-subagent-stop.ps1"),
    },
    HookShim {
        hook_name: "Notification",
        sh_filename: "cortex-notification.sh",
        ps1_filename: "cortex-notification.ps1",
        sh_source: include_str!("../hooks/cortex-notification.sh"),
        ps1_source: include_str!("../hooks/cortex-notification.ps1"),
    },
];

/// Failure modes raised by the install framework.
#[derive(Debug, Error)]
pub enum InstallError {
    /// Filesystem failure.
    #[error("install io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialise / parse failure.
    #[error("install json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Layout describing where the install / uninstall actions read and
/// write files. Tests pass an in-tempdir layout; the binary uses
/// [`Layout::from_home`].
#[derive(Debug, Clone)]
pub struct Layout {
    /// `~/.claude/` directory.
    pub claude_dir: PathBuf,
    /// `~/.claude/settings.json` path.
    pub settings_path: PathBuf,
    /// `~/.claude/hooks/` directory.
    pub hooks_dir: PathBuf,
}

impl Layout {
    /// Build the standard layout under `home`.
    pub fn from_home(home: &Path) -> Self {
        let claude_dir = home.join(".claude");
        Self {
            settings_path: claude_dir.join("settings.json"),
            hooks_dir: claude_dir.join("hooks"),
            claude_dir,
        }
    }
}

/// Resolved paths and report a single install run produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallReport {
    /// Hooks installed in this run.
    pub hooks_written: Vec<String>,
    /// Whether the settings.json patch produced a modified file.
    pub settings_modified: bool,
}

/// Install hook shims + patch settings.json for the current platform.
pub fn install(layout: &Layout) -> Result<InstallReport, InstallError> {
    fs::create_dir_all(&layout.hooks_dir)?;
    let mut hooks_written: Vec<String> = Vec::new();
    for shim in HOOK_SHIMS {
        let sh_path = layout.hooks_dir.join(shim.sh_filename);
        fs::write(&sh_path, shim.sh_source)?;
        let ps1_path = layout.hooks_dir.join(shim.ps1_filename);
        fs::write(&ps1_path, shim.ps1_source)?;
        hooks_written.push(shim.hook_name.to_string());
        // chmod +x on Unix so Claude Code can execute the shim.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&sh_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&sh_path, perms);
            }
        }
    }
    let settings_modified = patch_settings(&layout.settings_path)?;
    Ok(InstallReport {
        hooks_written,
        settings_modified,
    })
}

/// Reverse [`install`] — removes our hook stanzas from settings.json
/// and (when `purge_files` is `true`) removes the hook shim files
/// themselves. The settings.json is rewritten to its pre-install
/// shape exactly when nothing else owns the same keys.
pub fn uninstall(
    layout: &Layout,
    purge_files: bool,
) -> Result<InstallReport, InstallError> {
    let mut hooks_written: Vec<String> = Vec::new();
    if purge_files && layout.hooks_dir.exists() {
        for shim in HOOK_SHIMS {
            let _ = fs::remove_file(layout.hooks_dir.join(shim.sh_filename));
            let _ = fs::remove_file(layout.hooks_dir.join(shim.ps1_filename));
            hooks_written.push(shim.hook_name.to_string());
        }
    }
    let settings_modified = unpatch_settings(&layout.settings_path)?;
    Ok(InstallReport {
        hooks_written,
        settings_modified,
    })
}

/// Apply the cortex hook entries to `settings.json`. Returns `true`
/// when the file changed on disk.
fn patch_settings(path: &Path) -> Result<bool, InstallError> {
    let mut existing: Value = if path.exists() {
        let body = fs::read_to_string(path)?;
        serde_json::from_str(&body).unwrap_or_else(|_| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    let original = existing.clone();

    let hooks_map = existing
        .as_object_mut()
        .expect("settings.json root must be an object")
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks_obj = hooks_map.as_object_mut().expect("hooks must be an object");

    for shim in HOOK_SHIMS {
        let entry = build_hook_entry(shim);
        hooks_obj.insert(shim.hook_name.to_string(), entry);
    }

    if existing == original {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pretty = serde_json::to_string_pretty(&existing)?;
    fs::write(path, pretty)?;
    Ok(true)
}

/// Inverse of [`patch_settings`]. Removes every cortex-owned hook
/// stanza but leaves any user-installed hooks intact.
fn unpatch_settings(path: &Path) -> Result<bool, InstallError> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    let mut existing: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let original = existing.clone();
    if let Some(hooks) = existing
        .as_object_mut()
        .and_then(|m| m.get_mut("hooks"))
        .and_then(|h| h.as_object_mut())
    {
        for shim in HOOK_SHIMS {
            if let Some(entry) = hooks.get(shim.hook_name) {
                if is_cortex_entry(entry, shim) {
                    hooks.remove(shim.hook_name);
                }
            }
        }
    }
    // If `hooks` became empty, remove it so we don't leave clutter.
    if let Some(hooks) = existing
        .as_object_mut()
        .and_then(|m| m.get_mut("hooks"))
        .and_then(|h| h.as_object())
    {
        if hooks.is_empty() {
            existing
                .as_object_mut()
                .map(|m| m.remove("hooks"));
        }
    }
    if existing == original {
        return Ok(false);
    }
    let pretty = serde_json::to_string_pretty(&existing)?;
    fs::write(path, pretty)?;
    Ok(true)
}

fn build_hook_entry(shim: &HookShim) -> Value {
    json!({
        "type": "command",
        "command": format!("cortex-{}", shim.hook_name.to_ascii_lowercase()),
        "owner": "cortex"
    })
}

fn is_cortex_entry(entry: &Value, _shim: &HookShim) -> bool {
    entry
        .get("owner")
        .and_then(|v| v.as_str())
        .map(|s| s == "cortex")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_layout() -> (tempfile::TempDir, Layout) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::from_home(tmp.path());
        (tmp, layout)
    }

    #[test]
    fn install_creates_hook_files_and_settings() {
        let (_tmp, layout) = fixture_layout();
        let report = install(&layout).expect("install");
        assert_eq!(report.hooks_written.len(), HOOK_SHIMS.len());
        assert!(report.settings_modified);
        for shim in HOOK_SHIMS {
            assert!(layout.hooks_dir.join(shim.sh_filename).exists());
            assert!(layout.hooks_dir.join(shim.ps1_filename).exists());
        }
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), HOOK_SHIMS.len());
        for shim in HOOK_SHIMS {
            assert!(hooks.contains_key(shim.hook_name));
        }
    }

    #[test]
    fn install_is_idempotent_byte_identical_after_two_runs() {
        let (_tmp, layout) = fixture_layout();
        install(&layout).expect("first install");
        let after_first = fs::read_to_string(&layout.settings_path).unwrap();
        let report = install(&layout).expect("second install");
        let after_second = fs::read_to_string(&layout.settings_path).unwrap();
        assert!(!report.settings_modified);
        assert_eq!(after_first, after_second);
    }

    #[test]
    fn uninstall_restores_settings_to_pre_install_byte_identical() {
        let (_tmp, layout) = fixture_layout();
        // Pre-existing settings.json — operator-owned.
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let pre = json!({
            "theme": "dark",
            "hooks": {
                "MyCustom": { "type": "command", "command": "/usr/bin/true" }
            }
        });
        fs::write(
            &layout.settings_path,
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();
        let original = fs::read_to_string(&layout.settings_path).unwrap();

        install(&layout).expect("install");
        let mid = fs::read_to_string(&layout.settings_path).unwrap();
        assert_ne!(original, mid);

        uninstall(&layout, false).expect("uninstall");
        let after = fs::read_to_string(&layout.settings_path).unwrap();
        assert_eq!(after, original, "uninstall must restore the original bytes");
    }

    #[test]
    fn uninstall_purge_removes_hook_files() {
        let (_tmp, layout) = fixture_layout();
        install(&layout).expect("install");
        uninstall(&layout, true).expect("uninstall purge");
        for shim in HOOK_SHIMS {
            assert!(!layout.hooks_dir.join(shim.sh_filename).exists());
            assert!(!layout.hooks_dir.join(shim.ps1_filename).exists());
        }
    }

    #[test]
    fn uninstall_preserves_user_hooks() {
        let (_tmp, layout) = fixture_layout();
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let pre = json!({
            "hooks": {
                "MyCustom": { "type": "command", "command": "/usr/bin/true" }
            }
        });
        fs::write(&layout.settings_path, serde_json::to_string_pretty(&pre).unwrap()).unwrap();
        install(&layout).unwrap();
        uninstall(&layout, false).unwrap();
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        // MyCustom must remain.
        let hooks = parsed["hooks"].as_object().unwrap();
        assert!(hooks.contains_key("MyCustom"));
        // No cortex hooks should remain.
        for shim in HOOK_SHIMS {
            assert!(!hooks.contains_key(shim.hook_name));
        }
    }
}
