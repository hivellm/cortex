//! ADR-016 load path — CLI > env > `cortex.toml` > default.
//!
//! Each layer produces a `serde_json::Value` (object). Higher
//! layers merge-on-top of lower ones via [`merge_json`] (deep
//! object merge; scalars / arrays at higher precedence wins).
//! The final merged JSON is `serde_json::from_value::<Config>`
//! deserialised so every field's own `#[serde(default)]` fires
//! for absent paths.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{Config, ConfigError};
use crate::env_map::KNOWN_ENV_NAMES;

impl Config {
    /// Resolve the runtime configuration from the standard
    /// sources in priority order:
    ///
    /// 1. `cortex.toml` at `<cwd>/cortex.toml` (skipped silently
    ///    if absent).
    /// 2. Every `CORTEX_*` env var listed in
    ///    [`crate::env_map::KNOWN_ENV_NAMES`] (overrides toml).
    /// 3. Built-in [`Config::default`] for any field still
    ///    unset.
    ///
    /// CLI overrides are NOT applied here — binaries that thread
    /// flags through call [`Config::load_with_cli`] instead.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&default_toml_path(), |name| std::env::var(name).ok())
    }

    /// Like [`Self::load`] but reads the TOML from `toml_path`
    /// and resolves env values via `env_lookup` (production
    /// binds it to `std::env::var(name).ok()`; tests bind it to
    /// a `HashMap` snapshot for hermetic round-trip testing).
    pub fn load_from<F>(toml_path: &Path, env_lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        // Layer 1: defaults serialised as a JSON object.
        let mut merged: Value = serde_json::to_value(Config::default())?;

        // Layer 2: cortex.toml on top (when the file exists).
        if toml_path.exists() {
            let raw = std::fs::read_to_string(toml_path)?;
            let toml_value: toml::Value = toml::from_str(&raw)?;
            let toml_as_json: Value = serde_json::to_value(toml_value)?;
            merge_json(&mut merged, toml_as_json);
        }

        // Layer 3: env overlay on top of toml.
        let env_overlay = env_overlay(&env_lookup)?;
        if !env_overlay.is_null() {
            merge_json(&mut merged, env_overlay);
        }

        let cfg: Config = serde_json::from_value(merged)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Resolve the config and apply the binary's CLI overrides
    /// last. `overrides` is a JSON object whose shape mirrors
    /// [`Config`] — binaries build it from `clap` flags and
    /// pass it here. Absent keys leave the resolved value
    /// untouched.
    pub fn load_with_cli<F>(
        toml_path: &Path,
        env_lookup: F,
        overrides: Value,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut merged: Value = serde_json::to_value(Config::default())?;
        if toml_path.exists() {
            let raw = std::fs::read_to_string(toml_path)?;
            let toml_value: toml::Value = toml::from_str(&raw)?;
            merge_json(&mut merged, serde_json::to_value(toml_value)?);
        }
        let env_overlay = env_overlay(&env_lookup)?;
        if !env_overlay.is_null() {
            merge_json(&mut merged, env_overlay);
        }
        if !overrides.is_null() {
            merge_json(&mut merged, overrides);
        }
        let cfg: Config = serde_json::from_value(merged)?;
        cfg.validate()?;
        Ok(cfg)
    }
}

/// Production helper: resolve `cortex.toml` relative to the
/// caller's `$CARGO_MANIFEST_DIR` (binaries) or fall back to the
/// current working directory.
pub fn default_toml_path() -> PathBuf {
    if let Ok(p) = std::env::var("CORTEX_CONFIG_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("cortex.toml")
}

/// Walk [`KNOWN_ENV_NAMES`] and build a `serde_json::Value` tree
/// that places each set env var's value at its JSON pointer.
/// Values that fail to parse as a number / bool stay as
/// strings; serde's deserialiser picks the right type from the
/// target field via its `#[serde(default)]` chain.
pub(crate) fn env_overlay<F>(env_lookup: &F) -> Result<Value, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let mut overlay = Value::Object(Default::default());
    let mut any = false;
    for (env_name, pointer) in KNOWN_ENV_NAMES {
        let Some(raw) = env_lookup(env_name) else {
            continue;
        };
        if raw.is_empty() {
            // Empty string treated as "unset" — operators
            // routinely `unset` by setting blank in compose
            // overrides; matching that shape keeps the env layer
            // surprise-free.
            continue;
        }
        any = true;
        let json_value = coerce_to_json_scalar(&raw);
        set_by_pointer(&mut overlay, pointer, json_value);
    }
    Ok(if any { overlay } else { Value::Null })
}

/// Coerce a raw env-var string to its tightest JSON scalar so
/// numeric / boolean fields don't have to opt into custom
/// deserialisers. Falls through to `Value::String` when no
/// tighter type fits.
fn coerce_to_json_scalar(raw: &str) -> Value {
    if let Ok(b) = raw.parse::<bool>() {
        return Value::Bool(b);
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(u) = raw.parse::<u64>() {
        return Value::from(u);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Value::from(f);
    }
    Value::String(raw.to_string())
}

/// Deep merge `other` on top of `base` in-place. Objects merge
/// key-wise; non-object values at the same path are overwritten
/// by `other` (higher precedence wins). Mirrors the standard
/// "deep merge" idiom used in `figment` / `config-rs` without
/// the dep weight.
fn merge_json(base: &mut Value, other: Value) {
    match (base, other) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge_json(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, v) => {
            *slot = v;
        }
    }
}

/// Insert `value` into `tree` at the JSON pointer `pointer`,
/// creating intermediate `Object` nodes as needed. The pointer
/// MUST start with `/` per RFC 6901.
fn set_by_pointer(tree: &mut Value, pointer: &str, value: Value) {
    let segments: Vec<&str> = pointer
        .strip_prefix('/')
        .unwrap_or(pointer)
        .split('/')
        .collect();
    let mut cursor = tree;
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i + 1 == segments.len();
        if !cursor.is_object() {
            *cursor = Value::Object(Default::default());
        }
        let obj = cursor.as_object_mut().expect("just-made object");
        if is_last {
            obj.insert((*seg).to_string(), value);
            return;
        }
        let next = obj
            .entry((*seg).to_string())
            .or_insert_with(|| Value::Object(Default::default()));
        cursor = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    fn env_from(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |name: &str| map.get(name).map(|v| v.to_string())
    }

    #[test]
    fn loads_full_defaults_when_no_toml_no_env() {
        let cfg = Config::load_from(Path::new("__nonexistent.toml"), |_| None).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn env_overrides_default_for_url_field() {
        let env = env_from(HashMap::from([(
            "CORTEX_EMBEDDER_VECTORIZER_URL",
            "http://test:9999",
        )]));
        let cfg = Config::load_from(Path::new("__nonexistent.toml"), env).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url.as_deref(),
            Some("http://test:9999")
        );
        // Other fields keep their default values.
        assert_eq!(cfg.meili.meili_url, MeiliConfigDefault::meili_url());
    }

    #[test]
    fn env_overrides_numeric_field_via_coercion() {
        let env = env_from(HashMap::from([("CORTEX_RETENTION_BATCH_SIZE", "1024")]));
        let cfg = Config::load_from(Path::new("__nonexistent.toml"), env).unwrap();
        assert_eq!(cfg.retention.batch_size, 1024);
    }

    #[test]
    fn toml_overrides_default_and_env_overrides_toml() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("cortex.toml");
        std::fs::write(
            &toml_path,
            r#"
                [embedder]
                vectorizer_url = "http://from-toml:1"
                [meili]
                meili_url = "http://meili-from-toml:1"
            "#,
        )
        .unwrap();

        // No env → toml wins over default.
        let cfg = Config::load_from(&toml_path, |_| None).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url.as_deref(),
            Some("http://from-toml:1")
        );
        assert_eq!(
            cfg.meili.meili_url.as_deref(),
            Some("http://meili-from-toml:1")
        );

        // Env on top → env wins over toml for the field it sets.
        let env = env_from(HashMap::from([(
            "CORTEX_EMBEDDER_VECTORIZER_URL",
            "http://from-env:2",
        )]));
        let cfg = Config::load_from(&toml_path, env).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url.as_deref(),
            Some("http://from-env:2")
        );
        // Meili kept the toml value because env did not override it.
        assert_eq!(
            cfg.meili.meili_url.as_deref(),
            Some("http://meili-from-toml:1")
        );
    }

    #[test]
    fn cli_overrides_win_over_env_and_toml() {
        let env = env_from(HashMap::from([(
            "CORTEX_EMBEDDER_VECTORIZER_URL",
            "http://from-env:1",
        )]));
        let cli = serde_json::json!({
            "embedder": { "vectorizer_url": "http://from-cli:42" }
        });
        let cfg = Config::load_with_cli(Path::new("__nonexistent.toml"), env, cli).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url.as_deref(),
            Some("http://from-cli:42")
        );
    }

    #[test]
    fn empty_env_value_is_treated_as_unset() {
        let env = env_from(HashMap::from([("CORTEX_EMBEDDER_VECTORIZER_URL", "")]));
        let cfg = Config::load_from(Path::new("__nonexistent.toml"), env).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url,
            EmbedderConfigDefault::vectorizer_url(),
            "empty env value MUST fall through to the default, not clobber it"
        );
    }

    #[test]
    fn load_with_cli_reads_toml_when_file_present() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("cortex.toml");
        std::fs::write(
            &toml_path,
            r#"
                [embedder]
                vectorizer_url = "http://from-toml-cli:1"
            "#,
        )
        .unwrap();
        let cli = serde_json::json!({});
        let cfg = Config::load_with_cli(&toml_path, |_| None, cli).unwrap();
        assert_eq!(
            cfg.embedder.vectorizer_url.as_deref(),
            Some("http://from-toml-cli:1")
        );
    }

    #[test]
    fn load_with_cli_layers_env_then_cli_on_top_of_toml() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("cortex.toml");
        std::fs::write(
            &toml_path,
            r#"
                [embedder]
                vectorizer_url = "http://toml:1"
            "#,
        )
        .unwrap();
        let env = env_from(HashMap::from([(
            "CORTEX_EMBEDDER_VECTORIZER_URL",
            "http://env:2",
        )]));
        // Null CLI overlay leaves env winner intact.
        let cfg = Config::load_with_cli(&toml_path, env, Value::Null).unwrap();
        assert_eq!(cfg.embedder.vectorizer_url.as_deref(), Some("http://env:2"));
    }

    #[test]
    fn coerce_to_json_scalar_picks_tightest_type() {
        assert_eq!(coerce_to_json_scalar("true"), Value::Bool(true));
        assert_eq!(coerce_to_json_scalar("false"), Value::Bool(false));
        assert_eq!(coerce_to_json_scalar("42"), Value::from(42_i64));
        assert_eq!(coerce_to_json_scalar("-7"), Value::from(-7_i64));
        assert_eq!(
            coerce_to_json_scalar("9223372036854775808"),
            Value::from(9_223_372_036_854_775_808_u64)
        );
        assert_eq!(coerce_to_json_scalar("1.5"), Value::from(1.5_f64));
        assert_eq!(
            coerce_to_json_scalar("http://x:1"),
            Value::String("http://x:1".to_string())
        );
    }

    #[test]
    fn set_by_pointer_overwrites_intermediate_scalar() {
        // Previous layer dropped a scalar at /a; deeper pointer
        // /a/b/c must replace it with an object tree.
        let mut tree = serde_json::json!({ "a": "scalar-value" });
        set_by_pointer(&mut tree, "/a/b/c", Value::from(7_i64));
        assert_eq!(tree, serde_json::json!({ "a": { "b": { "c": 7 } } }));
    }

    #[test]
    fn merge_json_object_into_scalar_replaces() {
        let mut base = Value::String("scalar".into());
        merge_json(&mut base, serde_json::json!({ "k": 1 }));
        assert_eq!(base, serde_json::json!({ "k": 1 }));
    }

    #[test]
    fn deserialize_from_empty_json_fires_default_schema_version() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.schema_version, crate::config::SCHEMA_VERSION);
    }

    #[test]
    fn unsupported_schema_version_fails_validation() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("cortex.toml");
        std::fs::write(&toml_path, "schema_version = \"v999\"\n").unwrap();
        let err = Config::load_from(&toml_path, |_| None).unwrap_err();
        match err {
            ConfigError::UnsupportedSchemaVersion(v) => assert_eq!(v, "v999"),
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    // -----------------------------------------------------------
    // Helpers: re-expose the default fn values without making
    // them public from sub.rs.
    // -----------------------------------------------------------

    struct MeiliConfigDefault;
    impl MeiliConfigDefault {
        fn meili_url() -> Option<String> {
            crate::MeiliConfig::default().meili_url
        }
    }

    struct EmbedderConfigDefault;
    impl EmbedderConfigDefault {
        fn vectorizer_url() -> Option<String> {
            crate::EmbedderConfig::default().vectorizer_url
        }
    }
}
