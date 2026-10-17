//! Phase35 §1.10 — detect Cortex hooks still registered directly in Claude
//! Code's user settings, so the plugin does not deliver an event twice and
//! doctor can report a double install.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Plugin identifier Claude Code records in `enabledPlugins`.
pub const PLUGIN_ID: &str = "cortex@hivellm-cortex";

/// Claude Code user settings file: `$CLAUDE_CONFIG_DIR/settings.json`, else
/// `<home>/.claude/settings.json`.
#[must_use]
pub fn settings_path(claude_config_dir: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    claude_config_dir
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".claude")))
        .map(|dir| dir.join("settings.json"))
}

/// Events whose settings hooks invoke Cortex directly (the native
/// `cortex-hook` binary or a legacy `cortex-*` shim) rather than through a
/// plugin root.
#[must_use]
pub fn legacy_events(settings: &Value) -> BTreeSet<String> {
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return BTreeSet::new();
    };
    hooks
        .iter()
        .filter(|(_, groups)| {
            groups
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(Value::as_array))
                .flatten()
                .filter_map(|hook| hook.get("command").and_then(Value::as_str))
                .any(is_legacy_cortex_command)
        })
        .map(|(event, _)| event.clone())
        .collect()
}

fn is_legacy_cortex_command(command: &str) -> bool {
    !command.contains("CLAUDE_PLUGIN_ROOT")
        && (command.contains("cortex-hook") || command.contains("hooks/cortex-"))
}

/// [`legacy_events`] of the settings file at `path`; a missing or unreadable
/// file has none.
#[must_use]
pub fn read_legacy_events(path: &Path) -> BTreeSet<String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|settings| legacy_events(&settings))
        .unwrap_or_default()
}

/// Whether the settings enable the Cortex plugin.
#[must_use]
pub fn plugin_enabled(settings: &Value) -> bool {
    settings
        .get("enabledPlugins")
        .and_then(|plugins| plugins.get(PLUGIN_ID))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_native_and_shim_commands_but_not_plugin_hooks() {
        let settings = json!({"hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": "\"E:/x/bin/cortex-hook.exe\" SessionStart --pipe \"//./pipe/cortex-adapter-claude\""}]}],
            "Stop": [{"matcher": "*", "hooks": [{"command": "bash ~/.claude/hooks/cortex-stop.sh"}]}],
            "PreToolUse": [{"hooks": [{"command": "sh \"${CLAUDE_PLUGIN_ROOT}/scripts/cortex-hook-launcher\" PreToolUse"}]}],
            "PostToolUse": [{"hooks": [{"command": "other-tool post"}]}],
            "Notification": "malformed"
        }});
        assert_eq!(
            legacy_events(&settings).into_iter().collect::<Vec<_>>(),
            ["SessionStart", "Stop"]
        );
        assert!(legacy_events(&json!({})).is_empty());
    }

    #[test]
    fn unreadable_or_invalid_settings_have_no_legacy_events() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_legacy_events(&dir.path().join("absent.json")).is_empty());
        let invalid = dir.path().join("settings.json");
        std::fs::write(&invalid, "{ not json").unwrap();
        assert!(read_legacy_events(&invalid).is_empty());
    }

    #[test]
    fn settings_path_prefers_claude_config_dir() {
        assert_eq!(
            settings_path(Some(Path::new("/cfg")), Some(Path::new("/home"))),
            Some(PathBuf::from("/cfg/settings.json"))
        );
        assert_eq!(
            settings_path(Some(Path::new("")), Some(Path::new("/home"))),
            Some(PathBuf::from("/home/.claude/settings.json"))
        );
        assert_eq!(settings_path(None, None), None);
    }

    #[test]
    fn plugin_enabled_reads_enabled_plugins() {
        assert!(plugin_enabled(
            &json!({"enabledPlugins": {PLUGIN_ID: true}})
        ));
        assert!(!plugin_enabled(
            &json!({"enabledPlugins": {PLUGIN_ID: false}})
        ));
        assert!(!plugin_enabled(&json!({})));
    }
}
