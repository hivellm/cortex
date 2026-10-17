//! Runtime selection changes component wiring, never stored data or credentials.

use serde::{Deserialize, Serialize};

/// Process runtime policy. Existing installations retain full-stack behavior.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    /// Existing retrieval lanes and background jobs remain eligible.
    #[default]
    Full,
    /// Local API, durable memory and client transport without bulk dependencies.
    Daily,
}

/// Typed runtime knobs, resolved with normal TOML/env/CLI precedence.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// Profile controlling optional process components. Env: `CORTEX_RUNTIME_PROFILE`.
    #[serde(default)]
    pub profile: RuntimeProfile,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use std::path::Path;

    #[test]
    fn runtime_profile_defaults_to_compatible_full() {
        let cfg = Config::load_from(Path::new("__absent_runtime_fixture.toml"), |_| None).unwrap();
        assert_eq!(cfg.runtime.profile, RuntimeProfile::Full);
        assert!(!cfg.is_daily());
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn runtime_profile_resolves_toml_then_env_then_cli() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cortex.toml");
        std::fs::write(&path, "[runtime]\nprofile = 'daily'\n").unwrap();
        assert!(Config::load_from(&path, |_| None).unwrap().is_daily());
        let env = |key: &str| (key == "CORTEX_RUNTIME_PROFILE").then(|| "full".to_string());
        assert!(!Config::load_from(&path, env).unwrap().is_daily());
        assert!(Config::load_with_cli(
            &path,
            env,
            serde_json::json!({"runtime":{"profile":"daily"}})
        )
        .unwrap()
        .is_daily());
    }

    #[test]
    fn runtime_profile_rejects_unknown_values_instead_of_enabling_full() {
        let result = Config::load_from(Path::new("__absent_runtime_fixture.toml"), |key| {
            (key == "CORTEX_RUNTIME_PROFILE").then(|| "daliy".to_string())
        });
        assert!(result.is_err());
    }

    #[test]
    fn runtime_daily_preserves_backend_auth_storage_and_foreign_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cortex.toml");
        std::fs::write(
            &path,
            r#"
            [embedder]
            vectorizer_url = "http://127.0.0.1:19001"
            vectorizer_api_key = "fixture-vector-token"
            [meili]
            meili_url = "http://127.0.0.1:19002"
            meili_api_key = "fixture-meili-token"
            [nexus]
            nexus_url = "http://127.0.0.1:19003"
            [ingestion]
            metadata_db = "fixture/metadata.sqlite"
            archive_root = "fixture/archive"
            archive_incremental_enabled = true
            [dashboard]
            api_token = "fixture-api-token"
            watch = true
            [reranker]
            enabled = true
            endpoint = "http://127.0.0.1:19004"
            [access_control]
            enabled = true
            [laws]
            dir = "fixture/laws"
        "#,
        )
        .unwrap();
        let full = Config::load_from(&path, |_| None).unwrap();
        let mut daily = Config::load_from(&path, |key| {
            (key == "CORTEX_RUNTIME_PROFILE").then(|| "daily".to_string())
        })
        .unwrap();
        assert!(daily.is_daily());
        assert!(daily.ingestion.archive_incremental_enabled);
        daily.runtime.profile = RuntimeProfile::Full;
        assert_eq!(
            daily, full,
            "profile selection must not rewrite unrelated configuration"
        );
    }
}
