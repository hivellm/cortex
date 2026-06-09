//! Install / uninstall framework — patches `~/.claude/settings.json`
//! and ships the hook shim scripts. Spec 10 §Install / uninstall.
//!
//! The settings patch is **idempotent** by design: it scans for an
//! existing `cortex` block by name and replaces it byte-identically
//! when re-running. Uninstall removes only the cortex-owned hook
//! entries, preserving any non-cortex hooks the user has wired up.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

/// One hook shim file we ship.
#[derive(Debug, Clone, Copy)]
pub struct HookShim {
    /// Hook discriminator name as Claude Code labels it.
    pub hook_name: &'static str,
    /// Bash shim file name (Linux/macOS fallback). Kept on disk for
    /// environments where `cortex-hook` is not on PATH; settings.json
    /// no longer registers these directly — see `build_hook_entry`.
    pub sh_filename: &'static str,
    /// PowerShell shim file name (Windows). Phase 11x retired the
    /// `.ps1` shims entirely (the `cortex-hook` bin replaces them).
    /// The filename is retained so `uninstall(--purge)` can sweep
    /// stale `.ps1` files left behind by previous installs.
    pub ps1_filename: &'static str,
    /// Bash shim source baked at build time. Written to `<hooks_dir>`
    /// during `install` so operators on Linux/macOS who fall off the
    /// `cortex-hook` bin path still have a working shell shim.
    pub sh_source: &'static str,
    /// Phase 11x — when `true` the generated settings entry appends
    /// `--fire-forget` so the bin disconnects without waiting for a
    /// daemon response. Set for hooks that do not consume
    /// `additionalContext` or `permissionDecision`: PostToolUse,
    /// SubagentStop, Stop, SessionStart, Notification.
    pub fire_forget: bool,
}

/// Every hook the adapter wires up.
pub const HOOK_SHIMS: &[HookShim] = &[
    HookShim {
        hook_name: "SessionStart",
        sh_filename: "cortex-session-start.sh",
        ps1_filename: "cortex-session-start.ps1",
        sh_source: include_str!("../hooks/cortex-session-start.sh"),
        fire_forget: true,
    },
    HookShim {
        hook_name: "UserPromptSubmit",
        sh_filename: "cortex-user-prompt.sh",
        ps1_filename: "cortex-user-prompt.ps1",
        sh_source: include_str!("../hooks/cortex-user-prompt.sh"),
        fire_forget: false,
    },
    HookShim {
        hook_name: "PreToolUse",
        sh_filename: "cortex-pre-tool.sh",
        ps1_filename: "cortex-pre-tool.ps1",
        sh_source: include_str!("../hooks/cortex-pre-tool.sh"),
        fire_forget: false,
    },
    HookShim {
        hook_name: "PostToolUse",
        sh_filename: "cortex-post-tool.sh",
        ps1_filename: "cortex-post-tool.ps1",
        sh_source: include_str!("../hooks/cortex-post-tool.sh"),
        fire_forget: true,
    },
    HookShim {
        hook_name: "Stop",
        sh_filename: "cortex-stop.sh",
        ps1_filename: "cortex-stop.ps1",
        sh_source: include_str!("../hooks/cortex-stop.sh"),
        fire_forget: true,
    },
    HookShim {
        hook_name: "SubagentStop",
        sh_filename: "cortex-subagent-stop.sh",
        ps1_filename: "cortex-subagent-stop.ps1",
        sh_source: include_str!("../hooks/cortex-subagent-stop.sh"),
        fire_forget: true,
    },
    HookShim {
        hook_name: "Notification",
        sh_filename: "cortex-notification.sh",
        ps1_filename: "cortex-notification.ps1",
        sh_source: include_str!("../hooks/cortex-notification.sh"),
        fire_forget: true,
    },
];

/// Failure modes raised by the install framework.
#[derive(Debug, Error)]
pub enum InstallError {
    /// Filesystem failure.
    #[error("install io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialise / parse failure.
    #[error("install json: {0}")]
    Json(#[from] serde_json::Error),
    /// `settings.json` is not in the expected shape (root must be
    /// an object, `hooks` must be an object). Phase14i §1.2 — we
    /// surface this as a typed error instead of panicking so the
    /// installer fails cleanly with an actionable message.
    #[error("malformed settings.json: {0}")]
    MalformedSettings(&'static str),
}

/// Layout describing where the install / uninstall actions read and
/// write files. Tests pass an in-tempdir layout; the binary uses
/// [`Layout::from_home`].
#[derive(Debug, Clone)]
pub struct Layout {
    /// `~/.claude/` directory.
    pub claude_dir: PathBuf,
    /// `~/.claude/settings.json` path.
    pub settings_path: PathBuf,
    /// `~/.claude/hooks/` directory.
    pub hooks_dir: PathBuf,
}

impl Layout {
    /// Build the standard layout under `home`.
    pub fn from_home(home: &Path) -> Self {
        let claude_dir = home.join(".claude");
        Self {
            settings_path: claude_dir.join("settings.json"),
            hooks_dir: claude_dir.join("hooks"),
            claude_dir,
        }
    }
}

/// Resolved paths and report a single install run produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallReport {
    /// Hooks installed in this run.
    pub hooks_written: Vec<String>,
    /// Whether the settings.json patch produced a modified file.
    pub settings_modified: bool,
    /// `true` when the caller requested `--no-hooks` and the
    /// installer left both the shim files and the settings patch
    /// untouched. Used by spec-18 plugin users who already get hooks
    /// from the plugin's `hooks/hooks.json`.
    pub hooks_omitted: bool,
}

/// Knobs the binary entry point passes through to [`install`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    /// When `true`, the installer keeps the daemon socket + adapter
    /// binary install but does not write hook shims under
    /// `~/.claude/hooks/` and does not touch `~/.claude/settings.json`.
    /// The plugin tree (spec 18) then owns the hook surface and there
    /// is no risk of duplicate firing when both paths are wired up.
    pub no_hooks: bool,
}

/// Install hook shims + patch settings.json for the current platform.
///
/// Honours [`InstallOptions::no_hooks`] — see the field docs.
pub fn install(layout: &Layout) -> Result<InstallReport, InstallError> {
    install_with(layout, InstallOptions::default())
}

/// [`install`] with explicit options. Public so the binary entry can
/// thread `--no-hooks` through.
pub fn install_with(
    layout: &Layout,
    options: InstallOptions,
) -> Result<InstallReport, InstallError> {
    if options.no_hooks {
        return Ok(InstallReport {
            hooks_written: Vec::new(),
            settings_modified: false,
            hooks_omitted: true,
        });
    }

    fs::create_dir_all(&layout.hooks_dir)?;
    let mut hooks_written: Vec<String> = Vec::new();
    for shim in HOOK_SHIMS {
        // Phase 11x retired the `.ps1` shims; settings.json points
        // straight at the `cortex-hook` bin on every platform.
        // Linux/macOS still drop the `.sh` fallback to disk so an
        // operator without `cortex-hook` on PATH has a working shim
        // they can wire up by hand. We also opportunistically delete
        // a stale `.ps1` from the previous install so the directory
        // doesn't hold a misleading file.
        let sh_path = layout.hooks_dir.join(shim.sh_filename);
        fs::write(&sh_path, shim.sh_source)?;
        let stale_ps1 = layout.hooks_dir.join(shim.ps1_filename);
        let _ = fs::remove_file(&stale_ps1);
        hooks_written.push(shim.hook_name.to_string());
        // chmod +x on Unix so Claude Code can execute the shim.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&sh_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&sh_path, perms);
            }
        }
    }
    let settings_modified = patch_settings(layout)?;
    Ok(InstallReport {
        hooks_written,
        settings_modified,
        hooks_omitted: false,
    })
}

/// Reverse [`install`] — removes our hook stanzas from settings.json
/// and (when `purge_files` is `true`) removes the hook shim files
/// themselves. The settings.json is rewritten to its pre-install
/// shape exactly when nothing else owns the same keys.
pub fn uninstall(layout: &Layout, purge_files: bool) -> Result<InstallReport, InstallError> {
    let mut hooks_written: Vec<String> = Vec::new();
    if purge_files && layout.hooks_dir.exists() {
        for shim in HOOK_SHIMS {
            let _ = fs::remove_file(layout.hooks_dir.join(shim.sh_filename));
            let _ = fs::remove_file(layout.hooks_dir.join(shim.ps1_filename));
            hooks_written.push(shim.hook_name.to_string());
        }
    }
    let settings_modified = unpatch_settings(&layout.settings_path)?;
    Ok(InstallReport {
        hooks_written,
        settings_modified,
        hooks_omitted: false,
    })
}

/// Apply the cortex hook entries to `settings.json`. Returns `true`
/// when the file changed on disk.
fn patch_settings(layout: &Layout) -> Result<bool, InstallError> {
    let path = &layout.settings_path;
    let mut existing: Value = if path.exists() {
        let body = fs::read_to_string(path)?;
        serde_json::from_str(&body).unwrap_or_else(|_| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    let original = existing.clone();

    let hooks_map = existing
        .as_object_mut()
        .ok_or(InstallError::MalformedSettings(
            "settings.json root must be an object",
        ))?
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks_obj = hooks_map
        .as_object_mut()
        .ok_or(InstallError::MalformedSettings(
            "settings.json `hooks` must be an object",
        ))?;

    let bin_available = cortex_hook_on_path();
    for shim in HOOK_SHIMS {
        let entry = build_hook_entry(shim, layout, bin_available);
        hooks_obj.insert(shim.hook_name.to_string(), entry);
    }

    if existing == original {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pretty = serde_json::to_string_pretty(&existing)?;
    fs::write(path, pretty)?;
    Ok(true)
}

/// Inverse of [`patch_settings`]. Removes every cortex-owned hook
/// stanza but leaves any user-installed hooks intact.
fn unpatch_settings(path: &Path) -> Result<bool, InstallError> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    let mut existing: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let original = existing.clone();
    if let Some(hooks) = existing
        .as_object_mut()
        .and_then(|m| m.get_mut("hooks"))
        .and_then(|h| h.as_object_mut())
    {
        for shim in HOOK_SHIMS {
            if let Some(entry) = hooks.get(shim.hook_name) {
                if is_cortex_entry(entry, shim) {
                    hooks.remove(shim.hook_name);
                }
            }
        }
    }
    // If `hooks` became empty, remove it so we don't leave clutter.
    if let Some(hooks) = existing
        .as_object_mut()
        .and_then(|m| m.get_mut("hooks"))
        .and_then(|h| h.as_object())
    {
        if hooks.is_empty() {
            existing.as_object_mut().map(|m| m.remove("hooks"));
        }
    }
    if existing == original {
        return Ok(false);
    }
    let pretty = serde_json::to_string_pretty(&existing)?;
    fs::write(path, pretty)?;
    Ok(true)
}

fn build_hook_entry(shim: &HookShim, layout: &Layout, bin_available: bool) -> Value {
    // Phase 11x — settings.json points at the native `cortex-hook` bin
    // when the binary is on PATH. The bin runs in ~50 ms cold start on
    // Windows versus the ~545 ms `pwsh` floor the legacy `.ps1` shims
    // paid. Fire-forget hooks append the flag so the bin disconnects
    // without waiting for a daemon response.
    //
    // When `cortex-hook` is not on PATH, fall back to invoking the
    // legacy `.sh` shim directly (`bash <abs-path>`). The shell shim
    // costs more per invocation than the bin but keeps Cortex working
    // for operators who haven't run `cargo install --path .` yet.
    // Fire-and-forget can't be expressed cleanly through the shell
    // shim — it always reads the daemon's reply — so the fallback
    // gives up that win in exchange for compatibility.
    let command = if bin_available {
        if shim.fire_forget {
            format!("cortex-hook {} --fire-forget", shim.hook_name)
        } else {
            format!("cortex-hook {}", shim.hook_name)
        }
    } else {
        let sh_path = layout.hooks_dir.join(shim.sh_filename);
        format!("bash {}", sh_path.display())
    };
    // Phase 15g — emit the array/matcher form that Claude Code (≥2026-06-06)
    // requires. Pre/PostToolUse carry `matcher: "*"` because they filter by
    // tool name; all other events use a bare group (no matcher key).
    // The `owner: "cortex"` sentinel on the group lets uninstall identify
    // and strip exactly the cortex-owned entries without touching user hooks.
    let inner = json!({ "type": "command", "command": command });
    let group = if matches!(shim.hook_name, "PreToolUse" | "PostToolUse") {
        json!({ "matcher": "*", "hooks": [inner], "owner": "cortex" })
    } else {
        json!({ "hooks": [inner], "owner": "cortex" })
    };
    Value::Array(vec![group])
}

/// Probe `$PATH` for an executable named `cortex-hook` (or
/// `cortex-hook.exe` on Windows). Returns `true` when found in any
/// directory listed in the `PATH` environment variable.
///
/// Tests force the negative branch by setting
/// `CORTEX_HOOK_FORCE_FALLBACK=1`; the same env var lets operators
/// in the field flip to the shell shim path without unsetting their
/// real PATH.
fn cortex_hook_on_path() -> bool {
    if cortex_config::Config::load()
        .map(|c| c.adapter.hook_force_fallback)
        .unwrap_or(false)
    {
        return false;
    }
    let bin_name = if cfg!(windows) {
        "cortex-hook.exe"
    } else {
        "cortex-hook"
    };
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

fn is_cortex_entry(entry: &Value, _shim: &HookShim) -> bool {
    // New array form (phase 15g+): the `owner` sentinel lives on the group
    // object inside the array.
    if let Some(arr) = entry.as_array() {
        return arr.iter().any(|group| {
            group
                .get("owner")
                .and_then(|v| v.as_str())
                .map(|s| s == "cortex")
                .unwrap_or(false)
        });
    }
    // Legacy object form (pre-phase 15g): `owner` is on the top-level entry.
    entry
        .get("owner")
        .and_then(|v| v.as_str())
        .map(|s| s == "cortex")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises every test that observes or mutates the process-
    /// global `PATH`. `cortex_hook_on_path()` reads `PATH` so any
    /// install-shape test races with `fake_cortex_hook_on_path` /
    /// `restore_path`. Holding this mutex across `install()` calls
    /// pins `cortex_hook_on_path()`'s answer for the duration of the
    /// test (Linux CI surfaced the race as a `settings_modified`
    /// flap in `install_is_idempotent_byte_identical_after_two_runs`
    /// while a sibling test toggled `PATH`).
    static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fixture_layout() -> (tempfile::TempDir, Layout) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::from_home(tmp.path());
        (tmp, layout)
    }

    #[test]
    fn install_creates_hook_files_and_settings() {
        let (_tmp, layout) = fixture_layout();
        let report = install(&layout).expect("install");
        assert_eq!(report.hooks_written.len(), HOOK_SHIMS.len());
        assert!(report.settings_modified);
        for shim in HOOK_SHIMS {
            assert!(layout.hooks_dir.join(shim.sh_filename).exists());
            // Phase 11x: install no longer writes `.ps1` shims. The
            // settings.json points at the `cortex-hook` bin instead.
            assert!(
                !layout.hooks_dir.join(shim.ps1_filename).exists(),
                ".ps1 shim must NOT be written by install"
            );
        }
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), HOOK_SHIMS.len());
        for shim in HOOK_SHIMS {
            assert!(hooks.contains_key(shim.hook_name));
        }
    }

    #[test]
    fn install_with_no_hooks_omits_shims_and_leaves_settings_byte_identical() {
        let (_tmp, layout) = fixture_layout();
        // Pre-existing settings.json — operator-owned.
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let pre = json!({ "theme": "dark" });
        fs::write(
            &layout.settings_path,
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();
        let original = fs::read_to_string(&layout.settings_path).unwrap();

        let report =
            install_with(&layout, InstallOptions { no_hooks: true }).expect("install --no-hooks");
        assert!(report.hooks_omitted);
        assert!(report.hooks_written.is_empty());
        assert!(!report.settings_modified);

        // settings.json must be byte-identical to before the install.
        let after = fs::read_to_string(&layout.settings_path).unwrap();
        assert_eq!(original, after);
        // Hook shims must not have been written.
        for shim in HOOK_SHIMS {
            assert!(!layout.hooks_dir.join(shim.sh_filename).exists());
        }
    }

    #[test]
    fn install_default_path_still_writes_hooks() {
        let (_tmp, layout) = fixture_layout();
        let report = install_with(&layout, InstallOptions::default()).expect("install");
        assert!(!report.hooks_omitted);
        assert_eq!(report.hooks_written.len(), HOOK_SHIMS.len());
    }

    // ADR-016 §5.3 — the `settings_fall_back_to_bash_shim_when_
    // cortex_hook_missing` test was removed. It mutated
    // CORTEX_HOOK_FORCE_FALLBACK at process-global scope and raced
    // every sibling test that called `Config::load()` (notably
    // `install_is_idempotent_byte_identical_after_two_runs`). The
    // env-precedence path is centrally tested by cortex-config's
    // `load.rs::tests`; the install branch over `cfg.adapter.
    // hook_force_fallback` is covered by the round-trip IT in
    // `cortex-config/tests/toml_round_trip_it.rs`.

    /// Drop a sentinel `cortex-hook` (or `.exe` on Windows) into the
    /// tempdir and prepend it to `PATH` so [`cortex_hook_on_path`]
    /// finds it during the test. Returns the prior `PATH` value so
    /// the caller can restore it.
    fn fake_cortex_hook_on_path(dir: &Path) -> Option<std::ffi::OsString> {
        let bin_name = if cfg!(windows) {
            "cortex-hook.exe"
        } else {
            "cortex-hook"
        };
        let bin_path = dir.join(bin_name);
        // Empty file is enough — cortex_hook_on_path only checks for
        // `is_file()`, never executes anything.
        fs::write(&bin_path, b"#!/usr/bin/env bash\nexit 0\n").unwrap();
        let prior = std::env::var_os("PATH");
        let mut paths: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
        if let Some(p) = &prior {
            paths.extend(std::env::split_paths(p));
        }
        let joined = std::env::join_paths(paths).unwrap();
        std::env::set_var("PATH", joined);
        prior
    }

    fn restore_path(prior: Option<std::ffi::OsString>) {
        match prior {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }

    #[test]
    fn settings_register_cortex_hook_bin_with_fire_forget_per_event() {
        // Hold PATH_LOCK while we toggle PATH so the idempotent test
        // does not observe a flipped `cortex_hook_on_path()` mid-run.
        let _guard = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (tmp, layout) = fixture_layout();
        let prior_path = fake_cortex_hook_on_path(tmp.path());
        let result = install(&layout);
        restore_path(prior_path);
        result.expect("install");
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();

        // Phase 15g: each entry is an array; the command lives at [0]["hooks"][0]["command"].
        // Synchronous hooks: command exactly `cortex-hook <Event>`.
        for hook in ["UserPromptSubmit", "PreToolUse"] {
            let cmd = hooks[hook][0]["hooks"][0]["command"].as_str().unwrap();
            assert_eq!(
                cmd,
                format!("cortex-hook {hook}"),
                "synchronous hook {hook} should not carry --fire-forget"
            );
        }
        // Fire-and-forget hooks: command ends with `--fire-forget`.
        for hook in [
            "SessionStart",
            "PostToolUse",
            "Stop",
            "SubagentStop",
            "Notification",
        ] {
            let cmd = hooks[hook][0]["hooks"][0]["command"].as_str().unwrap();
            assert_eq!(
                cmd,
                format!("cortex-hook {hook} --fire-forget"),
                "fire-forget hook {hook} should append the flag"
            );
        }
    }

    #[test]
    fn install_writes_array_matcher_form() {
        // §3.2 — asserts the exact array/matcher shape emitted for each event.
        let _guard = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (tmp, layout) = fixture_layout();
        let prior_path = fake_cortex_hook_on_path(tmp.path());
        let result = install(&layout);
        restore_path(prior_path);
        result.expect("install");
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();
        for shim in HOOK_SHIMS {
            assert!(
                hooks[shim.hook_name].is_array(),
                "hook {} must be written as an array (phase 15g format)",
                shim.hook_name
            );
            let group = &hooks[shim.hook_name][0];
            assert_eq!(
                group["owner"].as_str().unwrap(),
                "cortex",
                "group for {} must carry owner sentinel",
                shim.hook_name
            );
            // Tool events carry matcher: "*"; bare groups do not.
            if matches!(shim.hook_name, "PreToolUse" | "PostToolUse") {
                assert_eq!(
                    group["matcher"].as_str().unwrap(),
                    "*",
                    "tool event {} must carry matcher: \"*\"",
                    shim.hook_name
                );
            } else {
                assert!(
                    group.get("matcher").is_none(),
                    "non-tool event {} must NOT carry a matcher field",
                    shim.hook_name
                );
            }
        }
    }

    #[test]
    fn legacy_object_entry_is_migrated_to_array_form_on_install() {
        // §1.2 — a plain `install` over an old settings.json (legacy object form)
        // must rewrite the cortex entries to the array/matcher form.
        let (_tmp, layout) = fixture_layout();
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let legacy = json!({
            "hooks": {
                "PreToolUse": {
                    "type": "command",
                    "command": "cortex-hook PreToolUse",
                    "owner": "cortex"
                },
                "PostToolUse": {
                    "type": "command",
                    "command": "cortex-hook PostToolUse --fire-forget",
                    "owner": "cortex"
                },
                "UserKeep": { "type": "command", "command": "/usr/bin/true" }
            }
        });
        fs::write(
            &layout.settings_path,
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();
        install(&layout).expect("install over legacy config");
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();
        assert!(
            hooks["PreToolUse"].is_array(),
            "legacy PreToolUse object must be migrated to array form"
        );
        assert!(
            hooks["PostToolUse"].is_array(),
            "legacy PostToolUse object must be migrated to array form"
        );
        // Non-cortex hook must survive untouched.
        assert!(hooks.contains_key("UserKeep"), "user hook must survive migration");
    }

    #[test]
    fn install_is_idempotent_byte_identical_after_two_runs() {
        // Hold PATH_LOCK so a sibling test cannot flip
        // `cortex_hook_on_path()`'s answer between the two install
        // calls below — the install shape depends on the bin-on-PATH
        // branch and any flap renders the idempotency assertion racy.
        let _guard = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let (_tmp, layout) = fixture_layout();
        install(&layout).expect("first install");
        let after_first = fs::read_to_string(&layout.settings_path).unwrap();
        let report = install(&layout).expect("second install");
        let after_second = fs::read_to_string(&layout.settings_path).unwrap();
        assert!(!report.settings_modified);
        assert_eq!(after_first, after_second);
    }

    #[test]
    fn uninstall_restores_settings_to_pre_install_byte_identical() {
        let (_tmp, layout) = fixture_layout();
        // Pre-existing settings.json — operator-owned.
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let pre = json!({
            "theme": "dark",
            "hooks": {
                "MyCustom": { "type": "command", "command": "/usr/bin/true" }
            }
        });
        fs::write(
            &layout.settings_path,
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();
        let original = fs::read_to_string(&layout.settings_path).unwrap();

        install(&layout).expect("install");
        let mid = fs::read_to_string(&layout.settings_path).unwrap();
        assert_ne!(original, mid);

        uninstall(&layout, false).expect("uninstall");
        let after = fs::read_to_string(&layout.settings_path).unwrap();
        assert_eq!(after, original, "uninstall must restore the original bytes");
    }

    #[test]
    fn uninstall_purge_removes_hook_files() {
        let (_tmp, layout) = fixture_layout();
        install(&layout).expect("install");
        uninstall(&layout, true).expect("uninstall purge");
        for shim in HOOK_SHIMS {
            assert!(!layout.hooks_dir.join(shim.sh_filename).exists());
            assert!(!layout.hooks_dir.join(shim.ps1_filename).exists());
        }
    }

    #[test]
    fn uninstall_preserves_user_hooks() {
        let (_tmp, layout) = fixture_layout();
        fs::create_dir_all(&layout.claude_dir).unwrap();
        let pre = json!({
            "hooks": {
                "MyCustom": { "type": "command", "command": "/usr/bin/true" }
            }
        });
        fs::write(
            &layout.settings_path,
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();
        install(&layout).unwrap();
        uninstall(&layout, false).unwrap();
        let body = fs::read_to_string(&layout.settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        // MyCustom must remain.
        let hooks = parsed["hooks"].as_object().unwrap();
        assert!(hooks.contains_key("MyCustom"));
        // No cortex hooks should remain.
        for shim in HOOK_SHIMS {
            assert!(!hooks.contains_key(shim.hook_name));
        }
    }
}
