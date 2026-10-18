//! Small standalone configuration surface; repository identity is never guessed.
use crate::{files, LocalError, Result};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub fn home(explicit: Option<&Path>) -> Result<PathBuf> {
    let path = if let Some(path) = explicit {
        if path.as_os_str().is_empty() {
            return Err(LocalError::Invalid("Cortex home cannot be empty".into()));
        }
        path.to_path_buf()
    } else if let Some(path) = std::env::var_os("CORTEX_HOME").filter(|value| !value.is_empty()) {
        PathBuf::from(path)
    } else {
        let user = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| LocalError::Invalid("set CORTEX_HOME or pass --home".into()))?;
        PathBuf::from(user).join(".cortex")
    };
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}

pub fn resolve_repository(cwd: &Path, explicit: Option<&str>) -> Result<String> {
    if let Some(repository) = explicit {
        files::validate_scope(repository)?;
        return Ok(repository.to_owned());
    }
    let directory = cwd.canonicalize()?;
    if !directory.is_dir() {
        return Err(LocalError::Invalid(
            "repository cwd must be a directory".into(),
        ));
    }
    for parent in directory.ancestors() {
        let path = parent.join("cortex.toml");
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if parent.join(".git").exists() {
                    break;
                }
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if !file.metadata()?.is_file() {
            return Err(LocalError::Invalid(
                "cortex.toml must be a regular file".into(),
            ));
        }
        let mut text = String::new();
        file.take(65_537).read_to_string(&mut text)?;
        if text.len() > 65_536 {
            return Err(LocalError::Invalid("cortex.toml exceeds 64 KiB".into()));
        }
        let config: toml::Value = toml::from_str(&text)
            .map_err(|_| LocalError::Invalid("cannot parse repository cortex.toml".into()))?;
        let repository = config
            .get("cortex")
            .and_then(|value| value.get("id"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| LocalError::Invalid("cortex.toml requires [cortex] id".into()))?;
        files::validate_scope(repository)?;
        return Ok(repository.to_owned());
    }
    Err(LocalError::Invalid(
        "repository unknown: pass --repository or configure [cortex] id in cortex.toml".into(),
    ))
}

/// Explicit classification alone reads this credential; capture and retrieval do not.
#[must_use]
pub fn typesafe_key() -> Option<String> {
    std::env::var("TYPESAFE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
}

pub fn read_json(path: &Path, maximum: usize) -> Result<serde_json::Value> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)?;
    } else {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(LocalError::Invalid(
                "JSON input must be a regular file".into(),
            ));
        }
        file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    }
    if bytes.len() > maximum {
        return Err(LocalError::Invalid("JSON input exceeds size limit".into()));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
