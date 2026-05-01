//! Loader for the versioned settings.v1.json file.
//!
//! The settings file lives at `crates/cortex-fulltext/settings/`. The
//! crate ships its v1 contents inline via `include_str!` so the
//! binary doesn't need disk access to bootstrap; `load_settings_v1`
//! provides an alternate path-based loader for tests and for the
//! `cortex-fulltext-worker` binary when an operator wants to override
//! the in-tree version.

use std::fs;
use std::path::Path;

use serde_json::Value;
use thiserror::Error;

/// Source for the v1 settings, baked into the binary at build time.
pub const SETTINGS_V1: &str = include_str!("../../settings/settings.v1.json");

/// Failure modes while loading settings from disk.
#[derive(Debug, Error)]
pub enum SettingsLoadError {
    /// Filesystem access failed.
    #[error("settings file unreadable: {0}")]
    Io(#[from] std::io::Error),
    /// JSON parse failed.
    #[error("settings json parse failed: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Parse the baked-in v1 settings.
pub fn settings_v1_json() -> Result<Value, SettingsLoadError> {
    Ok(serde_json::from_str(SETTINGS_V1)?)
}

/// Load settings from a file on disk (operator-supplied override).
pub fn load_settings_v1(path: &Path) -> Result<Value, SettingsLoadError> {
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_in_settings_parse() {
        let v = settings_v1_json().expect("v1 settings parse");
        // Phase11i §3.3 — version bumped to v3 so the
        // settings-version watcher triggers a re-index pass that
        // adds `model` / `tool` / `session_id` / `outcome` to the
        // filterable set. Phase11b's v2 introduced `path_prefixes`.
        assert_eq!(v["version"], "v3");
        assert!(v["searchableAttributes"].as_array().unwrap().len() >= 4);
        assert!(v["sortableAttributes"].as_array().unwrap().contains(
            &Value::String("ts".to_string())
        ));
    }

    #[test]
    fn relevance_axis_fields_are_filterable() {
        // Phase11i §3.3-§3.5 — every axis-driving field must
        // appear in filterableAttributes so the keyword lane can
        // filter on it. The relevance-tuning IT depends on this
        // contract; pin it explicitly so a future settings rev
        // cannot silently drop one.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        for required in ["model", "tool", "session_id", "outcome"] {
            assert!(
                filterable.contains(&Value::String(required.to_string())),
                "filterableAttributes must include `{required}` for phase11i §3.3-§3.5",
            );
        }
    }

    #[test]
    fn path_prefixes_is_filterable() {
        // Phase11b §2.2 — `meili_lane::build_filter` emits
        // `path_prefixes IN [...]`; Meilisearch rejects filters on
        // attributes that aren't declared filterable, so this is the
        // load-bearing assertion.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        assert!(
            filterable.contains(&Value::String("path_prefixes".to_string())),
            "filterableAttributes must include `path_prefixes`",
        );
    }
}
