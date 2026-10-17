//! Phase35 §1.4 — `cortex-ops install-binaries` and `cortex-ops doctor-install`.
//!
//! The installer copies the daily runtime binaries from a release build into
//! a stable user directory (default `<Cortex home>/bin`), verifying SHA-256
//! after each copy and recording an `install-manifest.json`. A binary that is
//! running cannot be overwritten on Windows, so an existing one is renamed
//! aside first (renaming a running executable is allowed) and restored if the
//! copy fails. Dry-run by default; `--apply` writes.
//!
//! The doctor resolves each binary through `PATH` and reports one that is
//! missing, shadowed by another directory earlier in `PATH`, or whose bytes
//! differ from the manifest (stale) or from an optional build directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cortex_adapter_claude_code::legacy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const BINARIES: [&str; 4] = [
    "cortex-api",
    "cortex-adapter-claude",
    "cortex-hook",
    "cortex-mcp-server",
];
const MANIFEST: &str = "install-manifest.json";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Manifest {
    version: u32,
    source: PathBuf,
    binaries: BTreeMap<String, String>,
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

pub(super) fn default_bin_dir() -> Option<PathBuf> {
    cortex_config::cortex_home().map(|home| home.join("bin"))
}

pub(super) fn install_binaries(apply: bool, from: PathBuf, to: Option<PathBuf>) -> ExitCode {
    let Some(to) = to.or_else(default_bin_dir) else {
        eprintln!("install-binaries: no Cortex home; set CORTEX_HOME or pass --to");
        return ExitCode::FAILURE;
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    match install(&from, &to, apply, &stamp) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            if !apply {
                println!("dry run: nothing written; re-run with --apply to install");
            } else if !dir_on_path(&to, &path_dirs()) {
                println!(
                    "note: {} is not on PATH; add it so clients can launch the binaries by name",
                    to.display()
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("install-binaries: {error}");
            ExitCode::FAILURE
        }
    }
}

fn install(from: &Path, to: &Path, apply: bool, stamp: &str) -> Result<Vec<String>, String> {
    let mut hashes = BTreeMap::new();
    for name in BINARIES {
        let source = from.join(exe(name));
        let hash = sha256_file(&source).map_err(|e| format!("{}: {e}", source.display()))?;
        hashes.insert(name.to_string(), hash);
    }
    let mut lines = Vec::new();
    if apply {
        std::fs::create_dir_all(to).map_err(|e| format!("create {}: {e}", to.display()))?;
        lines.extend(remove_stale_asides(to));
    }
    for name in BINARIES {
        let source = from.join(exe(name));
        let target = to.join(exe(name));
        let hash = &hashes[name];
        if sha256_file(&target).ok().as_ref() == Some(hash) {
            lines.push(format!("unchanged: {}", target.display()));
            continue;
        }
        if !apply {
            let verb = if target.exists() { "replace" } else { "create" };
            lines.push(format!("{verb}: {} (sha256 {hash})", target.display()));
            continue;
        }
        replace_binary(&source, &target, hash, stamp)?;
        lines.push(format!("installed: {} (sha256 {hash})", target.display()));
    }
    if apply {
        let manifest = Manifest {
            version: 1,
            source: from.to_path_buf(),
            binaries: hashes,
        };
        let body = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        std::fs::write(to.join(MANIFEST), body).map_err(|e| format!("write manifest: {e}"))?;
    }
    Ok(lines)
}

/// Copy `source` over `target`, moving a possibly running `target` aside
/// first and putting it back when the copy cannot be verified.
fn replace_binary(source: &Path, target: &Path, hash: &str, stamp: &str) -> Result<(), String> {
    let staged = sibling(target, ".new");
    let _ = std::fs::remove_file(&staged);
    std::fs::copy(source, &staged).map_err(|e| format!("copy to {}: {e}", staged.display()))?;
    if sha256_file(&staged).ok().as_deref() != Some(hash) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("{} failed SHA-256 verification", staged.display()));
    }
    let aside = sibling(target, &format!(".old-{stamp}"));
    let moved_aside = target.exists();
    if moved_aside {
        std::fs::rename(target, &aside).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("move {} aside: {e}", target.display())
        })?;
    }
    if let Err(error) = std::fs::rename(&staged, target) {
        if moved_aside {
            let _ = std::fs::rename(&aside, target);
        }
        let _ = std::fs::remove_file(&staged);
        return Err(format!("place {}: {error}", target.display()));
    }
    Ok(())
}

fn remove_stale_asides(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = BINARIES
            .iter()
            .any(|bin| name.starts_with(&format!("{}.old-", exe(bin))));
        if !ours {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => lines.push(format!("removed previous: {name}")),
            Err(_) => lines.push(format!("kept in use: {name}")),
        }
    }
    lines
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn dir_on_path(dir: &Path, path: &[PathBuf]) -> bool {
    path.iter().any(|entry| same_dir(entry, dir))
}

fn resolve_on_path(name: &str, path: &[PathBuf]) -> Option<PathBuf> {
    path.iter()
        .map(|dir| dir.join(exe(name)))
        .find(|candidate| candidate.is_file())
}

#[derive(Debug, Serialize, PartialEq)]
struct Finding {
    binary: &'static str,
    status: &'static str,
    resolved: Option<PathBuf>,
    detail: String,
}

fn diagnose(bin_dir: &Path, path: &[PathBuf], build: Option<&Path>) -> Vec<Finding> {
    let manifest: Option<Manifest> = std::fs::read(bin_dir.join(MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    BINARIES
        .iter()
        .map(|&binary| {
            let resolved = resolve_on_path(binary, path);
            let installed = bin_dir.join(exe(binary));
            let (status, detail) = match &resolved {
                None => ("missing", "not found on PATH".to_string()),
                Some(found) if !same_dir(found, &installed) => (
                    "shadowed",
                    format!("PATH resolves before {}", installed.display()),
                ),
                Some(found) => {
                    let actual = sha256_file(found).ok();
                    let recorded = manifest.as_ref().and_then(|m| m.binaries.get(binary));
                    let built = build.and_then(|dir| sha256_file(&dir.join(exe(binary))).ok());
                    if recorded.is_none() {
                        ("stale", "no install manifest entry".to_string())
                    } else if actual.as_ref() != recorded {
                        ("stale", "differs from install manifest".to_string())
                    } else if built.is_some() && built != actual {
                        ("stale", "differs from the build directory".to_string())
                    } else {
                        ("ok", "matches install manifest".to_string())
                    }
                }
            };
            Finding {
                binary,
                status,
                resolved,
                detail,
            }
        })
        .collect()
}

pub(super) fn doctor_install(
    bin_dir: Option<PathBuf>,
    build: Option<PathBuf>,
    json: bool,
) -> ExitCode {
    let Some(bin_dir) = bin_dir.or_else(default_bin_dir) else {
        eprintln!("doctor-install: no Cortex home; set CORTEX_HOME or pass --bin-dir");
        return ExitCode::FAILURE;
    };
    let mut findings = diagnose(&bin_dir, &path_dirs(), build.as_deref());
    let home = std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let claude_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    if let Some(settings) = legacy::settings_path(claude_dir.as_deref(), home.as_deref()) {
        findings.extend(double_install(&settings));
    }
    if json {
        match serde_json::to_string_pretty(&findings) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("doctor-install: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        for finding in &findings {
            println!(
                "{:<22} {:<9} {}",
                finding.binary, finding.status, finding.detail
            );
        }
    }
    if findings.iter().all(|f| f.status == "ok") {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

fn double_install(settings_path: &Path) -> Option<Finding> {
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(settings_path).ok()?).ok()?;
    let events = legacy::legacy_events(&settings);
    (legacy::plugin_enabled(&settings) && !events.is_empty()).then(|| Finding {
        binary: "claude-settings",
        status: "double_install",
        resolved: Some(settings_path.to_path_buf()),
        detail: format!(
            "plugin {} is enabled and settings hooks still deliver: {}",
            legacy::PLUGIN_ID,
            events.into_iter().collect::<Vec<_>>().join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_dir(root: &Path, marker: &str) -> PathBuf {
        let dir = root.join(format!("build-{marker}"));
        std::fs::create_dir_all(&dir).unwrap();
        for name in BINARIES {
            std::fs::write(dir.join(exe(name)), format!("{name}-{marker}")).unwrap();
        }
        dir
    }

    #[test]
    fn dry_run_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let from = build_dir(root.path(), "a");
        let to = root.path().join("bin");
        let lines = install(&from, &to, false, "s").unwrap();
        assert!(!to.exists());
        assert!(lines.iter().all(|l| l.starts_with("create: ")), "{lines:?}");
    }

    #[test]
    fn install_is_idempotent_and_upgrades_with_aside_copy() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("bin");
        let first = build_dir(root.path(), "a");
        install(&first, &to, true, "s1").unwrap();
        let again = install(&first, &to, true, "s2").unwrap();
        assert!(
            again.iter().all(|l| l.starts_with("unchanged: ")),
            "{again:?}"
        );

        // Hold the old binary open like a running process would.
        let held = std::fs::File::open(to.join(exe("cortex-hook"))).unwrap();
        let second = build_dir(root.path(), "b");
        install(&second, &to, true, "s3").unwrap();
        drop(held);
        for name in BINARIES {
            assert_eq!(
                std::fs::read_to_string(to.join(exe(name))).unwrap(),
                format!("{name}-b")
            );
        }
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(to.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest.source, second);
        assert!(to.join(format!("{}.old-s3", exe("cortex-hook"))).exists());

        let lines = install(&second, &to, true, "s4").unwrap();
        assert!(lines.iter().any(|l| l.starts_with("removed previous: ")));
        assert!(!to.join(format!("{}.old-s3", exe("cortex-hook"))).exists());
    }

    #[test]
    fn missing_source_binary_fails_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let from = build_dir(root.path(), "a");
        std::fs::remove_file(from.join(exe("cortex-api"))).unwrap();
        let to = root.path().join("bin");
        assert!(install(&from, &to, true, "s").is_err());
        assert!(!to.exists());
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn should_keep_existing_binary_when_staged_copy_fails_sha256_verification() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        std::fs::write(&source, "new-bytes").unwrap();
        let to = root.path().join("bin");
        std::fs::create_dir_all(&to).unwrap();
        let target = to.join(exe("cortex-hook"));
        std::fs::write(&target, "old-bytes").unwrap();

        let error = replace_binary(&source, &target, "0".repeat(64).as_str(), "s").unwrap_err();
        assert!(error.contains("failed SHA-256 verification"), "{error}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old-bytes");
        assert_eq!(names(&to), vec![exe("cortex-hook")]);
    }

    #[test]
    fn should_not_create_binary_when_sha256_verification_fails_on_fresh_install() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        std::fs::write(&source, "new-bytes").unwrap();
        let to = root.path().join("bin");
        std::fs::create_dir_all(&to).unwrap();
        let target = to.join(exe("cortex-hook"));

        assert!(replace_binary(&source, &target, "deadbeef", "s").is_err());
        assert!(names(&to).is_empty());
    }

    #[test]
    fn should_leave_target_intact_when_rename_aside_fails() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        std::fs::write(&source, "new-bytes").unwrap();
        let hash = sha256_file(&source).unwrap();
        let to = root.path().join("bin");
        std::fs::create_dir_all(&to).unwrap();
        let target = to.join(exe("cortex-hook"));
        std::fs::write(&target, "old-bytes").unwrap();
        // A non-empty directory at the aside path makes the rename fail on
        // every platform.
        let blocker = sibling(&target, ".old-s");
        std::fs::create_dir_all(blocker.join("occupied")).unwrap();

        let error = replace_binary(&source, &target, &hash, "s").unwrap_err();
        assert!(error.contains("aside"), "{error}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old-bytes");
        assert!(!sibling(&target, ".new").exists());

        // The next attempt with a free aside path succeeds.
        replace_binary(&source, &target, &hash, "s2").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new-bytes");
        assert!(!sibling(&target, ".new").exists());
        assert_eq!(
            std::fs::read_to_string(sibling(&target, ".old-s2")).unwrap(),
            "old-bytes"
        );
    }

    #[test]
    fn should_remove_only_own_stale_asides_and_report_undeletable_ones() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        let hook_aside = format!("{}.old-s1", exe("cortex-hook"));
        let api_aside = format!("{}.old-s2", exe("cortex-api"));
        let stuck = format!("{}.old-s3", exe("cortex-mcp-server"));
        for name in [&hook_aside, &api_aside] {
            std::fs::write(dir.join(name), "old").unwrap();
        }
        std::fs::write(dir.join("other-tool.exe.old-s1"), "foreign").unwrap();
        std::fs::write(dir.join(exe("cortex-hook")), "current").unwrap();
        std::fs::write(dir.join(format!("{}.bak", exe("cortex-hook"))), "bak").unwrap();
        // A directory cannot be removed with remove_file, standing in for an
        // aside that is still locked by a running process.
        std::fs::create_dir_all(dir.join(&stuck)).unwrap();

        let mut lines = remove_stale_asides(dir);
        lines.sort();
        assert_eq!(
            lines,
            vec![
                format!("kept in use: {stuck}"),
                format!("removed previous: {api_aside}"),
                format!("removed previous: {hook_aside}"),
            ]
        );
        let left = names(dir);
        assert!(!left.contains(&hook_aside) && !left.contains(&api_aside));
        assert!(left.contains(&stuck));
        assert!(left.contains(&"other-tool.exe.old-s1".to_string()));
        assert!(left.contains(&exe("cortex-hook")));
    }

    #[test]
    fn should_return_no_lines_when_aside_dir_is_missing() {
        let root = tempfile::tempdir().unwrap();
        assert!(remove_stale_asides(&root.path().join("absent")).is_empty());
    }

    #[test]
    fn should_not_remove_stale_asides_during_dry_run() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("bin");
        let first = build_dir(root.path(), "a");
        install(&first, &to, true, "s1").unwrap();
        let aside = to.join(format!("{}.old-s0", exe("cortex-hook")));
        std::fs::write(&aside, "old").unwrap();
        let second = build_dir(root.path(), "b");
        let lines = install(&second, &to, false, "s2").unwrap();
        assert!(aside.exists());
        assert!(
            lines.iter().all(|l| l.starts_with("replace: ")),
            "{lines:?}"
        );
        assert_eq!(
            std::fs::read_to_string(to.join(exe("cortex-hook"))).unwrap(),
            "cortex-hook-a"
        );
    }

    #[test]
    fn doctor_reports_ok_missing_shadowed_and_stale() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("bin");
        let build = build_dir(root.path(), "a");
        install(&build, &to, true, "s").unwrap();

        let ok = diagnose(&to, std::slice::from_ref(&to), Some(&build));
        assert!(ok.iter().all(|f| f.status == "ok"), "{ok:?}");
        assert!(diagnose(&to, &[], None)
            .iter()
            .all(|f| f.status == "missing"));

        let shadow = build_dir(root.path(), "shadow");
        let shadowed = diagnose(&to, &[shadow, to.clone()], None);
        assert!(shadowed.iter().all(|f| f.status == "shadowed"));

        let newer = build_dir(root.path(), "b");
        let stale_build = diagnose(&to, std::slice::from_ref(&to), Some(&newer));
        assert!(stale_build.iter().all(|f| f.status == "stale"));

        std::fs::write(to.join(exe("cortex-hook")), "tampered").unwrap();
        let tampered = diagnose(&to, std::slice::from_ref(&to), None);
        let hook = tampered.iter().find(|f| f.binary == "cortex-hook").unwrap();
        assert_eq!(hook.status, "stale");
        assert_eq!(hook.detail, "differs from install manifest");
    }

    #[test]
    fn double_install_is_reported_only_with_plugin_and_legacy_hooks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let hook = serde_json::json!({"command": "cortex-hook Stop"});
        let write = |plugin: bool, hooks: bool| {
            let mut settings = serde_json::json!({"enabledPlugins": {legacy::PLUGIN_ID: plugin}});
            if hooks {
                settings["hooks"] = serde_json::json!({"Stop": [{"hooks": [hook.clone()]}]});
            }
            std::fs::write(&path, settings.to_string()).unwrap();
        };
        write(true, true);
        let finding = double_install(&path).unwrap();
        assert_eq!(finding.status, "double_install");
        assert!(finding.detail.ends_with("Stop"));
        write(false, true);
        assert!(double_install(&path).is_none());
        write(true, false);
        assert!(double_install(&path).is_none());
        assert!(double_install(&root.path().join("absent.json")).is_none());
    }
}
