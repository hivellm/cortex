//! Offline installation from one executable and its embedded, versioned plugin assets.
use crate::{files, LocalError, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PLUGIN_FILES: usize = 4096;

macro_rules! asset {
    ($client:literal, $path:literal) => {
        (
            concat!($client, "/", $path),
            include_bytes!(concat!(
                "../../../packages/cortex-",
                $client,
                "-plugin/",
                $path
            ))
            .as_slice(),
        )
    };
}

fn assets() -> Vec<(&'static str, &'static [u8])> {
    vec![
        (".claude-plugin/marketplace.json", br#"{"name":"hivellm-cortex-local","owner":{"name":"HiveLLM"},"plugins":[{"name":"cortex","source":"./claude","version":"0.4.0"}]}"#),
        (".agents/plugins/marketplace.json", br#"{"name":"hivellm-cortex-local","interface":{"displayName":"HiveLLM Cortex Local"},"plugins":[{"name":"cortex","source":{"source":"local","path":"./codex"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}]}"#),
        asset!("codex", ".codex-plugin/plugin.json"),
        asset!("codex", "mcp.json"),
        asset!("codex", "hooks/hooks.json"),
        asset!("codex", "scripts/cortex-hook-launcher"),
        asset!("codex", "scripts/cortex-hook-launcher.cmd"),
        asset!("codex", "skills/cortex-memory/SKILL.md"),
        asset!("codex", "README.md"),
        asset!("codex", ".gitattributes"),
        asset!("claude", ".claude-plugin/plugin.json"),
        asset!("claude", ".mcp.json"),
        asset!("claude", "hooks/hooks.json"),
        asset!("claude", "hooks/cortex-session-start.sh"),
        asset!("claude", "hooks/cortex-user-prompt.sh"),
        asset!("claude", "hooks/cortex-post-tool.sh"),
        asset!("claude", "hooks/cortex-stop.sh"),
        asset!("claude", "hooks/cortex-subagent-stop.sh"),
        asset!("claude", "skills/cortex-memory/SKILL.md"),
        asset!("claude", "skills/cortex-context/SKILL.md"),
        asset!("claude", "skills/cortex-laws/SKILL.md"),
        asset!("claude", "skills/cortex-audit/SKILL.md"),
        asset!("claude", "commands/cortex-audit.md"),
        asset!("claude", "commands/cortex-decisions.md"),
        asset!("claude", "commands/cortex-laws.md"),
        asset!("claude", "commands/cortex-pre-thinking.md"),
        asset!("claude", "commands/cortex-query.md"),
        asset!("claude", "commands/cortex-status.md"),
        asset!("claude", "agents/cortex-context-curator.md"),
        asset!("claude", "agents/cortex-historian.md"),
        asset!("claude", "agents/cortex-lawkeeper.md"),
        asset!("claude", "README.md"),
        asset!("claude", ".gitattributes"),
    ]
}

fn rendered_assets(home: &Path, binary: &Path) -> Result<Vec<(&'static str, Vec<u8>)>> {
    let home = home
        .to_str()
        .ok_or_else(|| invalid("installation home must be valid UTF-8 for client manifests"))?;
    let binary = binary.to_str().ok_or_else(|| {
        invalid("installed executable path must be valid UTF-8 for client manifests")
    })?;
    assets()
        .into_iter()
        .map(|(path, bytes)| {
            if matches!(path, "claude/.mcp.json" | "codex/mcp.json") {
                let mut manifest: serde_json::Value = serde_json::from_slice(bytes)?;
                let server = manifest
                    .pointer_mut("/mcpServers/cortex")
                    .and_then(serde_json::Value::as_object_mut)
                    .ok_or_else(|| invalid("embedded Cortex MCP server template is invalid"))?;
                server.insert("command".into(), serde_json::Value::String(binary.into()));
                server.insert("env".into(), serde_json::json!({"CORTEX_HOME": home}));
                let mut rendered = serde_json::to_vec_pretty(&manifest)?;
                rendered.push(b'\n');
                Ok((path, rendered))
            } else {
                Ok((path, bytes.to_vec()))
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub changed: bool,
    pub binary: PathBuf,
    pub binary_sha256: String,
    pub plugin_root: PathBuf,
    pub claude_plugin: PathBuf,
    pub codex_plugin: PathBuf,
    pub files: Vec<InstalledFile>,
    pub backups: Vec<PathBuf>,
    pub instructions: Vec<String>,
}

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(message.into())
}

fn regular_file(path: &Path) -> Result<()> {
    files::ensure_no_symlinks(path)?;
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(invalid("installer expects a regular file"));
    }
    Ok(())
}

fn hash_file(path: &Path, limit: u64) -> Result<String> {
    regular_file(path)?;
    let mut file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(invalid("installation file exceeds size limit"));
    }
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > limit {
            return Err(invalid("installation file grew beyond size limit"));
        }
        hasher.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn directory(path: &Path) -> Result<()> {
    files::ensure_no_symlinks(path)?;
    fs::create_dir_all(path)?;
    files::ensure_no_symlinks(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(invalid("installer expects a directory"));
    }
    files::sync_directory(path)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        files::sync_directory(parent)?;
    }
    Ok(())
}

fn inventory(root: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((path, depth)) = pending.pop() {
        files::ensure_no_symlinks(&path)?;
        if depth > 16 {
            return Err(invalid("installed plugin tree exceeds depth limit"));
        }
        let metadata = fs::symlink_metadata(&path)?;
        seen += 1;
        if seen > MAX_PLUGIN_FILES {
            return Err(invalid("installed plugin tree exceeds entry limit"));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                if seen + pending.len() >= MAX_PLUGIN_FILES {
                    return Err(invalid("installed plugin tree exceeds entry limit"));
                }
                pending.push((entry?.path(), depth + 1));
            }
        } else if metadata.is_file() {
            entries.insert(
                path.strip_prefix(root)
                    .map_err(|_| invalid("invalid plugin path"))?
                    .to_path_buf(),
                hash_file(&path, 8 * 1024 * 1024)?,
            );
        } else {
            return Err(invalid("installed plugin tree contains a special file"));
        }
    }
    Ok(entries)
}

fn executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn has_execute_permissions(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o111 == 0o111)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(true)
    }
}

fn executable_asset(relative: &str) -> bool {
    relative.ends_with(".sh") || relative.ends_with("/cortex-hook-launcher")
}

struct Publication {
    staged: PathBuf,
    destination: PathBuf,
    backup: PathBuf,
    saved: bool,
    published: bool,
}

// Each rename is atomic on the same volume. The old tree is retained before
// publication; any in-process failure restores every completed rename in reverse.
fn publish(
    publications: &mut [Publication],
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<Vec<PathBuf>> {
    let result = (|| -> Result<()> {
        for publication in publications.iter_mut() {
            files::ensure_no_symlinks(&publication.destination)?;
            if publication.destination.try_exists()? {
                rename(&publication.destination, &publication.backup)?;
                publication.saved = true;
                files::sync_directory(
                    publication
                        .backup
                        .parent()
                        .ok_or_else(|| invalid("missing backup parent"))?,
                )?;
            }
            rename(&publication.staged, &publication.destination)?;
            publication.published = true;
            files::sync_directory(
                publication
                    .destination
                    .parent()
                    .ok_or_else(|| invalid("missing destination parent"))?,
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for publication in publications.iter_mut().rev() {
            if publication.published {
                if rename(&publication.destination, &publication.staged).is_err() {
                    return Err(invalid("installation failed and rollback could not move the new artifact; prior artifacts remain in backups"));
                }
                publication.published = false;
            }
            if publication.saved {
                if rename(&publication.backup, &publication.destination).is_err() {
                    return Err(invalid("installation failed and rollback could not restore an artifact; prior artifacts remain in backups"));
                }
                publication.saved = false;
                files::sync_directory(
                    publication
                        .destination
                        .parent()
                        .ok_or_else(|| invalid("missing destination parent"))?,
                )?;
            }
        }
        return Err(error);
    }
    Ok(publications
        .iter()
        .filter(|item| item.saved)
        .map(|item| item.backup.clone())
        .collect())
}

/// Install locally without fetching, editing client settings, or touching stored memory.
/// The caller validates that `source_binary` is a trusted standalone Cortex release.
pub fn install(home: &Path, source_binary: &Path) -> Result<InstallReport> {
    regular_file(source_binary)?;
    if fs::metadata(source_binary)?.len() == 0 {
        return Err(invalid("source executable is empty"));
    }
    let binary_sha256 = hash_file(source_binary, MAX_BINARY_BYTES)?;
    let home = files::prepare_root(home)?;
    directory(&home)?;
    let lock_path = home.join(".install.lock");
    files::ensure_no_symlinks(&lock_path)?;
    if lock_path.try_exists()? {
        regular_file(&lock_path)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.try_lock_exclusive().map_err(|_| {
        LocalError::Conflict("another installation holds the local install lock".into())
    })?;
    let bin_dir = home.join("bin");
    let plugins = home.join("plugins");
    directory(&bin_dir)?;
    directory(&plugins)?;
    let binary = bin_dir.join(if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    });
    let plugin_root = plugins.join("cortex-standalone");
    files::ensure_no_symlinks(&binary)?;
    files::ensure_no_symlinks(&plugin_root)?;
    let binary_changed = !binary.try_exists()?
        || hash_file(&binary, MAX_BINARY_BYTES)? != binary_sha256
        || !has_execute_permissions(&binary)?;
    let assets = rendered_assets(&home, &binary)?;
    let expected: BTreeMap<PathBuf, String> = assets
        .iter()
        .map(|(path, bytes)| (PathBuf::from(path), files::hash(bytes)))
        .collect();
    let mut plugins_changed = !plugin_root.try_exists()? || inventory(&plugin_root)? != expected;
    if !plugins_changed {
        for (relative, _) in &assets {
            if executable_asset(relative) && !has_execute_permissions(&plugin_root.join(relative))?
            {
                plugins_changed = true;
                break;
            }
        }
    }
    let mut report = InstallReport {
        changed: binary_changed || plugins_changed,
        binary: binary.clone(), binary_sha256: binary_sha256.clone(),
        claude_plugin: plugin_root.join("claude"), codex_plugin: plugin_root.join("codex"),
        plugin_root: plugin_root.clone(),
        files: expected.iter().map(|(path, hash)| InstalledFile { path: plugin_root.join(path), sha256: hash.clone() }).collect(),
        backups: Vec::new(),
        instructions: vec![
            format!("Optionally add {} to PATH for CLI use, or use the executable by its full path. Installed MCP manifests use the absolute executable path and do not require PATH changes.", bin_dir.display()),
            format!("Installed MCP manifests set CORTEX_HOME to {}. For lifecycle hooks with a custom home, set the same CORTEX_HOME in the environment used to launch Claude Code or Codex.", home.display()),
            format!("Register the local marketplace at {} in Claude Code or Codex; select the Cortex plugin.", plugin_root.display()),
            "Install or refresh client caches with `claude plugin install cortex@hivellm-cortex-local --scope user` and `codex plugin add cortex@hivellm-cortex-local`, for the clients you use. Restart those clients and grant trust to changed Cortex hooks.".into(),
            "No client settings or native agent memory were rewritten. Classification remains explicitly opt-in.".into(),
        ],
    };
    if !report.changed {
        return Ok(report);
    }
    let stage = tempfile::Builder::new()
        .prefix(".install-stage-")
        .tempdir_in(&home)?;
    let staged_binary = stage.path().join("cortex-new");
    if binary_changed {
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged_binary)?;
        let copied = std::io::copy(
            &mut File::open(source_binary)?.take(MAX_BINARY_BYTES + 1),
            &mut destination,
        )?;
        if copied > MAX_BINARY_BYTES {
            return Err(invalid("source executable grew beyond size limit"));
        }
        executable(&staged_binary)?;
        destination.sync_all()?;
        if hash_file(&staged_binary, MAX_BINARY_BYTES)? != binary_sha256 {
            return Err(invalid("source executable changed during staged copy"));
        }
    }
    let staged_plugins = stage.path().join("plugins-new");
    if plugins_changed {
        directory(&staged_plugins)?;
        for (relative, bytes) in assets {
            let path = staged_plugins.join(relative);
            directory(
                path.parent()
                    .ok_or_else(|| invalid("invalid embedded plugin path"))?,
            )?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            file.write_all(&bytes)?;
            if executable_asset(relative) {
                executable(&path)?;
            }
            file.sync_all()?;
            files::sync_directory(
                path.parent()
                    .ok_or_else(|| invalid("missing plugin parent"))?,
            )?;
        }
        if inventory(&staged_plugins)? != expected {
            return Err(invalid("staged plugin verification failed"));
        }
    }
    let backups = home.join("backups");
    directory(&backups)?;
    let backup_root = backups.join(format!("install-{}", ulid::Ulid::new()));
    directory(&backup_root)?;
    let mut publications = Vec::new();
    if plugins_changed {
        publications.push(Publication {
            staged: staged_plugins,
            destination: plugin_root,
            backup: backup_root.join("plugins"),
            saved: false,
            published: false,
        });
    }
    if binary_changed {
        publications.push(Publication {
            staged: staged_binary,
            destination: binary,
            backup: backup_root.join(if cfg!(windows) {
                "cortex.exe"
            } else {
                "cortex"
            }),
            saved: false,
            published: false,
        });
    }
    report.backups = publish(&mut publications, |from, to| fs::rename(from, to))?;
    files::sync_directory(&home)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_publication_failure_restores_every_prior_artifact() {
        let root = tempfile::tempdir().unwrap();
        let mut publications = Vec::new();
        for name in ["plugin", "binary"] {
            let staged = root.path().join(format!("{name}-new"));
            let destination = root.path().join(name);
            fs::write(&staged, format!("new {name}")).unwrap();
            fs::write(&destination, format!("old {name}")).unwrap();
            publications.push(Publication {
                staged,
                destination,
                backup: root.path().join(format!("{name}-backup")),
                saved: false,
                published: false,
            });
        }
        let mut calls = 0;
        let error = publish(&mut publications, |from, to| {
            calls += 1;
            if calls == 4 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected second publish failure",
                ));
            }
            fs::rename(from, to)
        })
        .unwrap_err();
        assert!(matches!(error, LocalError::Io(_)));
        for name in ["plugin", "binary"] {
            assert_eq!(
                fs::read_to_string(root.path().join(name)).unwrap(),
                format!("old {name}")
            );
            assert_eq!(
                fs::read_to_string(root.path().join(format!("{name}-new"))).unwrap(),
                format!("new {name}")
            );
        }
    }
}
