//! Daily API credential self-discovery (phase35 §1.2).
//!
//! Client binaries (MCP server, adapter) authenticate to the daily API with
//! a bearer token. When neither the selected TOML nor `CORTEX_API_TOKEN`
//! supplies one, [`Config::load_client`] reads it from
//! `<Cortex home>/api-token`, so no client configuration has to carry it.
//! The file holds only the token; surrounding whitespace is ignored. Errors
//! name the path and a reason, never the file contents.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError};
use crate::load::cortex_home;

/// File name of the API token inside the Cortex home directory.
pub const API_TOKEN_FILE: &str = "api-token";

const MAX_TOKEN_BYTES: u64 = 4096;

/// Path of the user API token file, `<Cortex home>/api-token`, without
/// touching the filesystem. `None` when no home directory is available.
#[must_use]
pub fn api_token_path() -> Option<PathBuf> {
    cortex_home().map(|home| home.join(API_TOKEN_FILE))
}

impl Config {
    /// [`Config::load`] for API clients: when the resolved configuration has
    /// no nonempty `dashboard.api_token`, read it from [`api_token_path`].
    /// A missing or empty file leaves the token unset (anonymous transport).
    /// An unreadable, oversized or (on Unix) group/other-accessible file is
    /// an error, so a broken credential is reported instead of silently
    /// degrading to unauthenticated requests.
    pub fn load_client() -> Result<Self, ConfigError> {
        let mut config = Self::load()?;
        config.apply_api_token_file(api_token_path().as_deref())?;
        Ok(config)
    }

    /// Fill `dashboard.api_token` from `path` when it is unset or empty.
    /// Configured tokens (TOML or `CORTEX_API_TOKEN`) always win.
    pub fn apply_api_token_file(&mut self, path: Option<&Path>) -> Result<(), ConfigError> {
        if self
            .dashboard
            .api_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
        {
            return Ok(());
        }
        if let Some(path) = path {
            self.dashboard.api_token = read_api_token(path)?;
        }
        Ok(())
    }
}

fn read_api_token(path: &Path) -> Result<Option<String>, ConfigError> {
    let credential_error = |reason: &'static str| ConfigError::Credential {
        path: path.to_path_buf(),
        reason,
    };
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err(credential_error("not a regular file")),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(credential_error("unreadable")),
    }
    let file = std::fs::File::open(path).map_err(|_| credential_error("unreadable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| credential_error("unreadable"))?;
    if !metadata.is_file() {
        return Err(credential_error("not a regular file"));
    }
    if metadata.len() > MAX_TOKEN_BYTES {
        return Err(credential_error("larger than 4096 bytes"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(credential_error(
                "accessible to group or others; restrict it to mode 0600",
            ));
        }
    }
    let mut raw = String::new();
    file.take(MAX_TOKEN_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|_| credential_error("unreadable or not UTF-8"))?;
    let token = raw.trim();
    if token.chars().any(char::is_control) {
        return Err(credential_error("contains control characters"));
    }
    Ok((!token.is_empty()).then(|| token.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "synthetic-credential-0f3a";

    fn private_file(dir: &Path, body: &[u8]) -> PathBuf {
        let path = dir.join(API_TOKEN_FILE);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    #[test]
    fn unset_token_is_read_and_trimmed_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = private_file(dir.path(), format!("  {SECRET}\r\n").as_bytes());
        for initial in [None, Some(String::new())] {
            let mut config = Config::default();
            config.dashboard.api_token = initial;
            config.apply_api_token_file(Some(&path)).unwrap();
            assert_eq!(config.dashboard.api_token.as_deref(), Some(SECRET));
        }
    }

    #[test]
    fn configured_token_wins_and_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        // A directory would be an error if it were consulted.
        let mut config = Config::default();
        config.dashboard.api_token = Some("configured".into());
        config.apply_api_token_file(Some(dir.path())).unwrap();
        assert_eq!(config.dashboard.api_token.as_deref(), Some("configured"));
    }

    #[test]
    fn missing_or_empty_file_and_no_home_keep_anonymous_transport() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config
            .apply_api_token_file(Some(&dir.path().join("absent")))
            .unwrap();
        config.apply_api_token_file(None).unwrap();
        let empty = private_file(dir.path(), b" \n");
        config.apply_api_token_file(Some(&empty)).unwrap();
        assert_eq!(config.dashboard.api_token, None);
    }

    #[test]
    fn broken_credentials_error_without_echoing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let cases: Vec<(PathBuf, &str)> = vec![
            (dir.path().to_path_buf(), "not a regular file"),
            (
                private_file(dir.path(), &vec![b'a'; 5000]),
                "larger than 4096 bytes",
            ),
        ];
        for (path, reason) in cases {
            let error = Config::default()
                .apply_api_token_file(Some(&path))
                .unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
        let control = private_file(dir.path(), format!("{SECRET}\u{1b}x").as_bytes());
        let error = Config::default()
            .apply_api_token_file(Some(&control))
            .unwrap_err();
        assert!(!error.to_string().contains(SECRET));
        assert!(!format!("{error:?}").contains(SECRET));
        let invalid = private_file(dir.path(), &[0xff, 0xfe]);
        assert!(matches!(
            Config::default().apply_api_token_file(Some(&invalid)),
            Err(ConfigError::Credential { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_file_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = private_file(dir.path(), SECRET.as_bytes());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let error = Config::default()
            .apply_api_token_file(Some(&path))
            .unwrap_err();
        assert!(error.to_string().contains("0600"));
    }

    const PROBE_ENV: &str = "CREDENTIAL_LOAD_CLIENT_PROBE";
    const PROBE_TEST: &str = "credential::tests::load_client_probe_child";

    /// Child half of the `load_client_*` tests: a no-op in a normal run.
    /// When re-executed with [`PROBE_ENV`], it runs the real
    /// [`Config::load_client`] against the child's process environment and
    /// reports the outcome on `PROBE_*` lines for the parent to assert.
    #[test]
    fn load_client_probe_child() {
        if std::env::var_os(PROBE_ENV).is_none() {
            return;
        }
        match Config::load_client() {
            Ok(config) => {
                println!(
                    "PROBE_TOKEN {}",
                    config.dashboard.api_token.as_deref().unwrap_or("<none>")
                );
                println!("PROBE_DEBUG {config:?}");
            }
            Err(error) => {
                println!("PROBE_ERROR {error}");
                println!("PROBE_ERROR_DEBUG {error:?}");
            }
        }
    }

    /// Re-execute this test binary so `load_client` reads a hermetic
    /// environment: every inherited `CORTEX_*` variable is removed, the
    /// Cortex home and config file point into `home`, and the parent's own
    /// environment is never mutated.
    fn load_client_in_child(home: &Path, env: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", PROBE_TEST, "--nocapture", "--test-threads=1"]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("CORTEX_") {
                command.env_remove(&key);
            }
        }
        command
            .env(PROBE_ENV, "1")
            .env("CORTEX_HOME", home)
            .env("CORTEX_CONFIG_FILE", home.join("config.toml"))
            .env("HOME", home)
            .env("USERPROFILE", home)
            .current_dir(home);
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "child failed: {stdout}");
        assert!(stdout.contains("1 passed"), "probe did not run: {stdout}");
        stdout
            .lines()
            // libtest may print `test <name> ... ` before the first line.
            .filter_map(|line| line.split_once("PROBE_").map(|(_, rest)| rest))
            .filter_map(|line| line.split_once(' '))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn probe_value<'a>(lines: &'a [(String, String)], key: &str) -> &'a str {
        lines
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_else(|| panic!("missing PROBE_{key} in {lines:?}"))
    }

    #[test]
    fn load_client_reads_home_token_file_when_env_token_is_unset() {
        let home = tempfile::tempdir().unwrap();
        let lines = load_client_in_child(home.path(), &[]);
        assert_eq!(probe_value(&lines, "TOKEN"), "<none>");

        private_file(home.path(), format!("  {SECRET}\n").as_bytes());
        let lines = load_client_in_child(home.path(), &[]);
        assert_eq!(probe_value(&lines, "TOKEN"), SECRET);
        let debug = probe_value(&lines, "DEBUG");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains(SECRET), "token leaked into Debug: {debug}");
    }

    #[test]
    fn load_client_prefers_configured_token_over_home_token_file() {
        let home = tempfile::tempdir().unwrap();
        private_file(home.path(), SECRET.as_bytes());
        let lines = load_client_in_child(home.path(), &[("CORTEX_API_TOKEN", "from-env")]);
        assert_eq!(probe_value(&lines, "TOKEN"), "from-env");

        std::fs::write(
            home.path().join("config.toml"),
            "[dashboard]\napi_token = \"from-toml\"\n",
        )
        .unwrap();
        let lines = load_client_in_child(home.path(), &[]);
        assert_eq!(probe_value(&lines, "TOKEN"), "from-toml");
    }

    #[test]
    fn load_client_error_names_path_but_never_token_when_file_is_broken() {
        for body in [
            SECRET.repeat(4096 / SECRET.len() + 1),
            format!("{SECRET}\u{7}"),
        ] {
            let home = tempfile::tempdir().unwrap();
            private_file(home.path(), body.as_bytes());
            let lines = load_client_in_child(home.path(), &[]);
            let display = probe_value(&lines, "ERROR");
            let debug = probe_value(&lines, "ERROR_DEBUG");
            assert!(display.contains(API_TOKEN_FILE), "{display}");
            assert!(!display.contains(SECRET), "token leaked: {display}");
            assert!(!debug.contains(SECRET), "token leaked: {debug}");
        }
    }

    #[test]
    fn debug_output_redacts_token() {
        let mut config = Config::default();
        config.dashboard.api_token = Some(SECRET.into());
        let rendered = format!("{config:?} {:?}", config.dashboard);
        assert!(!rendered.contains(SECRET));
        assert!(rendered.contains("api_token: Some(\"<redacted>\")"));
    }
}
