use cortex_local::{install::install, LocalError};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn source(directory: &Path, bytes: &[u8]) -> std::path::PathBuf {
    let path = directory.join("standalone-release");
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn complete_offline_bundle_is_verified_and_reinstallation_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"synthetic standalone artifact v1");
    let home = temp.path().join("home");
    let first = install(&home, &binary).unwrap();
    assert!(first.changed);
    assert!(first.backups.is_empty());
    assert_eq!(fs::read(&first.binary).unwrap(), fs::read(&binary).unwrap());
    assert_eq!(
        first.binary_sha256,
        format!("{:x}", Sha256::digest(fs::read(&binary).unwrap()))
    );
    assert!(first.files.len() >= 30);
    for asset in &first.files {
        assert_eq!(
            asset.sha256,
            format!("{:x}", Sha256::digest(fs::read(&asset.path).unwrap()))
        );
        assert!(asset.path.starts_with(&first.plugin_root));
    }
    for relative in [
        ".claude-plugin/marketplace.json",
        ".agents/plugins/marketplace.json",
        "codex/.codex-plugin/plugin.json",
        "claude/.claude-plugin/plugin.json",
    ] {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(first.plugin_root.join(relative)).unwrap()).unwrap();
        assert!(value.is_object());
    }
    assert!(!first.claude_plugin.join("scripts/sync-cache.sh").exists());
    assert!(!first.claude_plugin.join("scripts/install-mcp.js").exists());
    let second = install(&home, &first.binary).unwrap();
    assert!(!second.changed);
    assert!(second.backups.is_empty());
    assert_eq!(second.binary_sha256, first.binary_sha256);
    assert!(second
        .instructions
        .iter()
        .any(|line| line.contains("CORTEX_HOME")
            && line.contains(&fs::canonicalize(&home).unwrap().display().to_string())));
    assert_eq!(fs::read_dir(home.join("backups")).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(&first.binary).unwrap().permissions().mode() & 0o111,
            0
        );
        assert_ne!(
            fs::metadata(first.codex_plugin.join("scripts/cortex-hook-launcher"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn same_bytes_restore_missing_execute_permissions_and_accept_ancestor_alias() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release");
    let real_parent = temp.path().join("real-parent");
    fs::create_dir(&real_parent).unwrap();
    let alias = temp.path().join("alias");
    symlink(&real_parent, &alias).unwrap();
    let home = alias.join("home");
    let installed = install(&home, &binary).unwrap();
    assert_eq!(
        installed.binary.parent().unwrap().parent().unwrap(),
        fs::canonicalize(real_parent.join("home")).unwrap()
    );
    let launcher = installed.codex_plugin.join("scripts/cortex-hook-launcher");
    fs::set_permissions(&installed.binary, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o644)).unwrap();
    let repaired = install(&home, &binary).unwrap();
    assert!(repaired.changed);
    assert_eq!(repaired.backups.len(), 2);
    assert_eq!(repaired.binary_sha256, installed.binary_sha256);
    assert_eq!(
        fs::metadata(&repaired.binary).unwrap().permissions().mode() & 0o111,
        0o111
    );
    assert_eq!(
        fs::metadata(&launcher).unwrap().permissions().mode() & 0o111,
        0o111
    );
    assert!(!install(&home, &binary).unwrap().changed);
}

#[cfg(windows)]
#[test]
fn windows_ancestor_junction_resolves_to_real_installation_root() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release");
    let real_parent = temp.path().join("real-parent");
    fs::create_dir(&real_parent).unwrap();
    let alias = temp.path().join("alias");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&alias)
        .arg(&real_parent)
        .output()
        .unwrap();
    assert!(status.status.success());
    let report = install(&alias.join("home"), &binary).unwrap();
    let expected = fs::canonicalize(real_parent.join("home")).unwrap();
    assert!(report.binary.starts_with(expected));
    assert!(!install(&alias.join("home"), &binary).unwrap().changed);
}

#[test]
fn update_retains_binary_plugin_customizations_and_unrelated_memory_settings() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release one");
    let home = temp.path().join("home");
    let initial = install(&home, &binary).unwrap();
    let untouched = [
        "local/records/evidence.md",
        "config.toml",
        "native-memory/MEMORY.md",
        "plugins/unrelated/config.json",
    ];
    for path in untouched {
        let path = home.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"user-owned data").unwrap();
    }
    fs::write(
        initial.codex_plugin.join("README.md"),
        b"customized old plugin",
    )
    .unwrap();
    fs::write(
        initial.plugin_root.join("personal.txt"),
        b"retain extra files",
    )
    .unwrap();
    fs::write(&binary, b"release two").unwrap();
    let update = install(&home, &binary).unwrap();
    assert!(update.changed);
    assert_eq!(update.backups.len(), 2);
    let binary_backup = update.backups.iter().find(|path| path.is_file()).unwrap();
    let plugin_backup = update.backups.iter().find(|path| path.is_dir()).unwrap();
    assert_eq!(fs::read(binary_backup).unwrap(), b"release one");
    assert_eq!(
        fs::read(plugin_backup.join("codex/README.md")).unwrap(),
        b"customized old plugin"
    );
    assert_eq!(
        fs::read(plugin_backup.join("personal.txt")).unwrap(),
        b"retain extra files"
    );
    assert_eq!(fs::read(&update.binary).unwrap(), b"release two");
    for path in untouched {
        assert_eq!(fs::read(home.join(path)).unwrap(), b"user-owned data");
    }
}

#[test]
fn installed_mcp_manifests_use_absolute_executable_and_custom_home_with_spaces() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"standalone artifact");
    let home = temp.path().join("custom home with spaces");
    let installed = install(&home, &binary).unwrap();
    let canonical_home = fs::canonicalize(&home).unwrap();
    for manifest in [
        installed.claude_plugin.join(".mcp.json"),
        installed.codex_plugin.join("mcp.json"),
    ] {
        let bytes = fs::read(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let server = &value["mcpServers"]["cortex"];
        assert_eq!(server["command"], installed.binary.to_str().unwrap());
        assert!(Path::new(server["command"].as_str().unwrap()).is_absolute());
        assert_eq!(
            server["env"]["CORTEX_HOME"],
            canonical_home.to_str().unwrap()
        );
        assert_eq!(server["args"], serde_json::json!(["mcp"]));
        let recorded = installed
            .files
            .iter()
            .find(|entry| entry.path == manifest)
            .unwrap();
        assert_eq!(recorded.sha256, format!("{:x}", Sha256::digest(bytes)));
    }
    assert!(!install(&home, &binary).unwrap().changed);
    assert!(installed
        .instructions
        .iter()
        .any(|line| line.contains("do not require PATH changes")));
}

#[test]
fn invalid_source_and_blocked_destination_preserve_existing_installation() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release one");
    let home = temp.path().join("home");
    let installed = install(&home, &binary).unwrap();
    let before = fs::read(&installed.binary).unwrap();
    let manifest = fs::read(installed.codex_plugin.join(".codex-plugin/plugin.json")).unwrap();
    fs::write(&binary, b"").unwrap();
    assert!(matches!(
        install(&home, &binary),
        Err(LocalError::Invalid(_))
    ));
    assert_eq!(fs::read(&installed.binary).unwrap(), before);
    assert_eq!(
        fs::read(installed.codex_plugin.join(".codex-plugin/plugin.json")).unwrap(),
        manifest
    );
    let blocked_home = temp.path().join("blocked");
    fs::create_dir_all(&blocked_home).unwrap();
    fs::write(blocked_home.join("bin"), b"user file").unwrap();
    fs::write(&binary, b"release two").unwrap();
    assert!(install(&blocked_home, &binary).is_err());
    assert_eq!(fs::read(blocked_home.join("bin")).unwrap(), b"user file");
    assert!(!blocked_home.join("plugins").exists());
}

#[test]
fn active_install_lock_prevents_second_publication() {
    use fs2::FileExt;
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release one");
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(home.join(".install.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert!(matches!(
        install(&home, &binary),
        Err(LocalError::Conflict(_))
    ));
    assert!(!home.join("bin").exists());
}

#[cfg(unix)]
#[test]
fn symlinked_home_binary_and_nested_plugin_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release");
    let real_home = temp.path().join("real");
    fs::create_dir(&real_home).unwrap();
    let link_home = temp.path().join("link");
    symlink(&real_home, &link_home).unwrap();
    assert!(install(&link_home, &binary).is_err());
    let installed = install(&real_home, &binary).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, installed.codex_plugin.join("redirect")).unwrap();
    fs::write(&binary, b"release two").unwrap();
    assert!(install(&real_home, &binary).is_err());
    assert_eq!(fs::read(installed.binary).unwrap(), b"release");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_junction_destinations_are_rejected_without_touching_target() {
    let temp = tempfile::tempdir().unwrap();
    let binary = source(temp.path(), b"release");
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let home = temp.path().join("home");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&home)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(install(&home, &binary).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let safe_home = temp.path().join("safe");
    let installed = install(&safe_home, &binary).unwrap();
    let redirect = installed.codex_plugin.join("redirect");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&redirect)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(status.status.success());
    fs::write(&binary, b"release two").unwrap();
    assert!(install(&safe_home, &binary).is_err());
    assert_eq!(fs::read(installed.binary).unwrap(), b"release");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
