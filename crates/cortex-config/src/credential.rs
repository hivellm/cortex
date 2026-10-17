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

    #[test]
    fn debug_output_redacts_token() {
        let mut config = Config::default();
        config.dashboard.api_token = Some(SECRET.into());
        let rendered = format!("{config:?} {:?}", config.dashboard);
        assert!(!rendered.contains(SECRET));
        assert!(rendered.contains("api_token: Some(\"<redacted>\")"));
    }
}
