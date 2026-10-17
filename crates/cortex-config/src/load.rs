//! Runtime load path — CLI > env > selected TOML > defaults.
//!
//! Each layer produces a `serde_json::Value` (object). Higher
//! layers merge-on-top of lower ones via [`merge_json`] (deep
//! object merge; scalars / arrays at higher precedence wins).
//! The final merged JSON is `serde_json::from_value::<Config>`
//! deserialised so every field's own `#[serde(default)]` fires
//! for absent paths.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{Config, ConfigError};
use crate::env_map::KNOWN_ENV_NAMES;

impl Config {
    /// Resolve runtime configuration from environment variables over one
    /// selected TOML file over built-in defaults.
    ///
    /// [`default_toml_path`] selects `CORTEX_CONFIG_FILE`, otherwise an
    /// existing user config (`CORTEX_HOME/config.toml` or
    /// `~/.cortex/config.toml`), otherwise `./cortex.toml`. Files are selected,
    /// never merged together: absent fields use defaults, not repo values.
    /// An explicitly selected missing file also uses defaults plus env;
    /// malformed or unreadable selected files return an error.
    ///
    /// CLI overrides are NOT applied here — binaries that thread
    /// flags through call [`Config::load_with_cli`] instead.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&default_toml_path(), |name| std::env::var(name).ok())
    }

    /// Read only `toml_path`, overlay `env_lookup`, then validate.
    /// No user/repo discovery or `CORTEX_CONFIG_FILE` lookup occurs here.
    /// A missing file uses defaults plus env; other read and parse errors
    /// propagate. Tests can inject a `HashMap` snapshot for hermetic loading.
    pub fn load_from<F>(toml_path: &Path, env_lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::load_with_cli(toml_path, env_lookup, Value::Null)
    }

    /// Resolve the config and apply the binary's CLI overrides
    /// last. `overrides` is a JSON object whose shape mirrors
    /// [`Config`] — binaries build it from `clap` flags and
    /// pass it here. Absent keys leave the resolved value
    /// untouched. Like [`Self::load_from`], this reads only `toml_path`;
    /// callers wanting discovery can pass [`default_toml_path`].
    pub fn load_with_cli<F>(
        toml_path: &Path,
        env_lookup: F,
        overrides: Value,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut merged: Value = serde_json::to_value(Config::default())?;
        match std::fs::read_to_string(toml_path) {
            Ok(raw) => {
                let toml_value: toml::Value = toml::from_str(&raw)?;
                merge_json(&mut merged, serde_json::to_value(toml_value)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
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

/// Select one runtime TOML file: nonempty `CORTEX_CONFIG_FILE`, otherwise
/// an existing [`cortex_home`]`/config.toml`, otherwise `./cortex.toml`.
/// Explicit paths win even when missing; a missing user config falls back
/// to the repo file. Probe errors retain the user path so loading surfaces
/// the error. Selected files never merge with lower-priority files.
#[must_use]
pub fn default_toml_path() -> PathBuf {
    default_toml_path_with(&|name| std::env::var_os(name), |path| {
        path.try_exists().unwrap_or(true)
    })
}

/// Resolve the user runtime directory without creating it.
/// Nonempty `CORTEX_HOME` wins; otherwise use `~/.cortex`. On Windows,
/// `USERPROFILE` is preferred over `HOME`; other platforms use `HOME`.
/// Empty values are unset. Returns `None` if no home is available.
/// Paths supplied by environment variables are used verbatim, including
/// relative overrides; normal absolute home paths work independently of cwd.
#[must_use]
pub fn cortex_home() -> Option<PathBuf> {
    cortex_home_with(&|name| std::env::var_os(name), cfg!(windows))
}

fn cortex_home_with<F>(env_lookup: &F, windows: bool) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<OsString>,
{
    let nonempty = |name| env_lookup(name).filter(|value| !value.is_empty());
    nonempty("CORTEX_HOME").map(PathBuf::from).or_else(|| {
        let home = if windows {
            nonempty("USERPROFILE").or_else(|| nonempty("HOME"))
        } else {
            nonempty("HOME")
        };
        home.map(|home| PathBuf::from(home).join(".cortex"))
    })
}

fn default_toml_path_with<F, E>(env_lookup: &F, exists: E) -> PathBuf
where
    F: Fn(&str) -> Option<OsString>,
    E: Fn(&Path) -> bool,
{
    if let Some(path) = env_lookup("CORTEX_CONFIG_FILE").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = cortex_home_with(env_lookup, cfg!(windows))
        .map(|home| home.join("config.toml"))
        .filter(|path| exists(path))
    {
        return path;
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

    fn path_env(values: &[(&str, OsString)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = values
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn write_config(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    // Resolve relative paths against an injected cwd, never changing process env/cwd.
    fn discovered_config(
        cwd: &Path,
        env: impl Fn(&str) -> Option<OsString>,
        overrides: Value,
    ) -> Result<Config, ConfigError> {
        let selected =
            default_toml_path_with(&env, |path| cwd.join(path).try_exists().unwrap_or(true));
        Config::load_with_cli(
            &cwd.join(selected),
            |name| env(name).and_then(|value| value.into_string().ok()),
            overrides,
        )
    }

    #[test]
    fn discovery_precedence_selects_one_file_then_env_then_cli() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let home = dir.path().join("user");
        let explicit = dir.path().join("explicit.toml");
        write_config(
            &repo.join("cortex.toml"),
            "[retention]\nbatch_size = 10\n[meili]\nmeili_api_key = 'repo-secret'\n",
        );
        write_config(&home.join("config.toml"), "[retention]\nbatch_size = 20\n");
        write_config(&explicit, "[retention]\nbatch_size = 30\n");

        for (include_explicit, include_env, include_cli, expected) in [
            (false, false, false, 20),
            (true, false, false, 30),
            (true, true, false, 40),
            (true, true, true, 50),
        ] {
            let mut values = vec![("CORTEX_HOME", home.as_os_str().to_owned())];
            if include_explicit {
                values.push(("CORTEX_CONFIG_FILE", explicit.as_os_str().to_owned()));
            }
            if include_env {
                values.push(("CORTEX_RETENTION_BATCH_SIZE", "40".into()));
            }
            let overrides = if include_cli {
                serde_json::json!({"retention": {"batch_size": 50}})
            } else {
                Value::Null
            };
            let cfg = discovered_config(&repo, path_env(&values), overrides).unwrap();
            assert_eq!(cfg.retention.batch_size, expected);
            assert_eq!(
                cfg.meili.meili_api_key,
                Config::default().meili.meili_api_key
            );
        }
    }

    #[test]
    fn user_config_is_independent_of_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("user");
        write_config(&home.join("config.toml"), "[retention]\nbatch_size = 25\n");
        for name in ["repo-a", "repo-b"] {
            let repo = dir.path().join(name);
            write_config(&repo.join("cortex.toml"), "invalid = [");
            let cfg = discovered_config(
                &repo,
                path_env(&[("CORTEX_HOME", home.as_os_str().to_owned())]),
                Value::Null,
            )
            .unwrap();
            assert_eq!(cfg.retention.batch_size, 25);
        }
    }

    #[test]
    fn missing_user_config_falls_back_to_repo_then_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let home = dir.path().join("missing-user");
        write_config(&repo.join("cortex.toml"), "[retention]\nbatch_size = 15\n");
        let values = [("CORTEX_HOME", home.as_os_str().to_owned())];
        assert_eq!(
            discovered_config(&repo, path_env(&values), Value::Null)
                .unwrap()
                .retention
                .batch_size,
            15
        );
        let mut expected = Config::default();
        expected.ingestion.home = Some(home.to_string_lossy().into_owned());
        assert_eq!(
            discovered_config(&dir.path().join("no-repo"), path_env(&values), Value::Null).unwrap(),
            expected
        );
    }

    #[test]
    fn missing_explicit_file_does_not_fall_through_to_other_files() {
        let dir = tempfile::tempdir().unwrap();
        write_config(&dir.path().join("cortex.toml"), "invalid = [");
        write_config(&dir.path().join("user/config.toml"), "invalid = [");
        let cfg = discovered_config(
            dir.path(),
            path_env(&[
                ("CORTEX_HOME", dir.path().join("user").into_os_string()),
                ("CORTEX_CONFIG_FILE", "missing-explicit.toml".into()),
                ("CORTEX_RETENTION_BATCH_SIZE", "77".into()),
            ]),
            Value::Null,
        )
        .unwrap();
        assert_eq!(cfg.retention.batch_size, 77);
    }

    #[test]
    fn selected_user_parse_and_read_errors_do_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("user");
        let config = home.join("config.toml");
        write_config(
            &dir.path().join("cortex.toml"),
            "[retention]\nbatch_size = 15\n",
        );
        write_config(&config, "invalid = [");
        let values = [("CORTEX_HOME", home.as_os_str().to_owned())];
        assert!(matches!(
            discovered_config(dir.path(), path_env(&values), Value::Null),
            Err(ConfigError::Toml(_))
        ));
        // Invalid UTF-8 produces a portable read error without changing ACLs.
        std::fs::write(&config, [0xff]).unwrap();
        assert!(matches!(
            discovered_config(dir.path(), path_env(&values), Value::Null),
            Err(ConfigError::Io(_))
        ));
    }

    #[test]
    fn selected_directory_returns_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("user");
        std::fs::create_dir_all(home.join("config.toml")).unwrap();
        assert!(matches!(
            discovered_config(
                dir.path(),
                path_env(&[("CORTEX_HOME", home.into_os_string())]),
                Value::Null
            ),
            Err(ConfigError::Io(_))
        ));
    }

    #[test]
    fn explicit_loaders_ignore_discovery_environment() {
        let dir = tempfile::tempdir().unwrap();
        let supplied = dir.path().join("supplied.toml");
        write_config(&supplied, "[retention]\nbatch_size = 13\n");
        let values = [
            ("CORTEX_HOME", dir.path().join("user").into_os_string()),
            (
                "CORTEX_CONFIG_FILE",
                dir.path().join("bad.toml").into_os_string(),
            ),
        ];
        write_config(&dir.path().join("bad.toml"), "invalid = [");
        write_config(&dir.path().join("user/config.toml"), "invalid = [");
        let env = path_env(&values);
        let lookup = |name: &str| env(name).and_then(|value| value.into_string().ok());
        assert_eq!(
            Config::load_from(&supplied, lookup)
                .unwrap()
                .retention
                .batch_size,
            13
        );
        assert_eq!(
            Config::load_with_cli(&supplied, lookup, Value::Null)
                .unwrap()
                .retention
                .batch_size,
            13
        );
    }

    #[test]
    fn cortex_home_respects_platform_precedence_and_empty_values() {
        let values = [
            ("CORTEX_HOME", "override".into()),
            ("USERPROFILE", "profile".into()),
            ("HOME", "home".into()),
        ];
        for windows in [true, false] {
            assert_eq!(
                cortex_home_with(&path_env(&values), windows),
                Some(PathBuf::from("override"))
            );
        }
        let values = [
            ("CORTEX_HOME", "".into()),
            ("USERPROFILE", "profile".into()),
            ("HOME", "home".into()),
        ];
        assert_eq!(
            cortex_home_with(&path_env(&values), true),
            Some(PathBuf::from("profile/.cortex"))
        );
        assert_eq!(
            cortex_home_with(&path_env(&values), false),
            Some(PathBuf::from("home/.cortex"))
        );
        assert_eq!(
            cortex_home_with(
                &path_env(&[("USERPROFILE", "".into()), ("HOME", "home".into())]),
                true
            ),
            Some(PathBuf::from("home/.cortex"))
        );
        for windows in [true, false] {
            assert_eq!(cortex_home_with(&path_env(&[]), windows), None);
            assert_eq!(
                cortex_home_with(&path_env(&[("HOME", "".into())]), windows),
                None
            );
        }
    }

    #[test]
    fn empty_discovery_values_are_unset_and_home_config_is_selected() {
        let env = path_env(&[
            ("CORTEX_CONFIG_FILE", "".into()),
            ("CORTEX_HOME", "".into()),
            ("USERPROFILE", "profile".into()),
            ("HOME", "home".into()),
        ]);
        let expected = if cfg!(windows) { "profile" } else { "home" };
        assert_eq!(
            default_toml_path_with(&env, |_| true),
            PathBuf::from(expected).join(".cortex/config.toml")
        );
        assert_eq!(
            default_toml_path_with(&env, |_| false),
            PathBuf::from("cortex.toml")
        );
        assert_eq!(
            default_toml_path_with(&path_env(&[]), |_| panic!("no home to probe")),
            PathBuf::from("cortex.toml")
        );
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
