//! Behavioral tests for the hook launchers that hand lifecycle events to
//! Legacy `cortex-hook` adapter shims and standalone Cortex Codex plugin
//! launchers (`packages/cortex-codex-plugin/scripts/cortex-hook-launcher` and, on
//! Windows, `cortex-hook-launcher.cmd` run through CMD and PowerShell).
//!
//! `hook_drift.rs` only greps anchor strings. These tests run the real scripts under
//! bash in a sandbox — `HOME`/`USERPROFILE` point at a tempdir, `PATH` holds only
//! bash's own directories plus test directories — with fake `cortex-hook` scripts
//! that append their identity to a record file. That proves which binary the
//! discovery ladder picks, which arguments and origin it forwards, and that every
//! miss fails open with `{}` and exit 0.
//!
//! Without bash (set `CORTEX_TEST_BASH` to override the lookup) the shell tests print
//! the reason and return early; the `hooks.json` contract tests always run.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn claude_hooks_dir() -> PathBuf {
    workspace_root().join("crates/cortex-adapter-claude-code/hooks")
}

fn codex_plugin_dir() -> PathBuf {
    workspace_root().join("packages/cortex-codex-plugin")
}

/// Claude Code shims: file, event, synchronous.
const CLAUDE_SHIMS: &[(&str, &str, bool)] = &[
    ("cortex-session-start.sh", "SessionStart", true),
    ("cortex-user-prompt.sh", "UserPromptSubmit", true),
    ("cortex-pre-tool.sh", "PreToolUse", true),
    ("cortex-post-tool.sh", "PostToolUse", false),
    ("cortex-stop.sh", "Stop", false),
    ("cortex-subagent-stop.sh", "SubagentStop", false),
    ("cortex-notification.sh", "Notification", false),
];

/// Codex plugin events: event, synchronous. PreToolUse is not registered: the
/// daemon answers it with `{}`, so it only added latency per tool call.
const CODEX_EVENTS: &[(&str, bool)] = &[
    ("SessionStart", true),
    ("UserPromptSubmit", true),
    ("PostToolUse", false),
    ("Stop", false),
    ("SubagentStop", false),
];

/// Arguments the Codex launchers hand to cortex-hook.
fn codex_args(event: &str, _synchronous: bool) -> String {
    format!("hook --client codex --event {event}")
}

fn find_bash() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CORTEX_TEST_BASH") {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var) {
                candidates.push(PathBuf::from(dir).join("Git").join("bin").join("bash.exe"));
            }
        }
        candidates.push(PathBuf::from("C:/Program Files/Git/bin/bash.exe"));
    } else {
        for path in [
            "/bin/bash",
            "/usr/bin/bash",
            "/usr/local/bin/bash",
            "/opt/homebrew/bin/bash",
        ] {
            candidates.push(PathBuf::from(path));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Directories bash itself needs on PATH (`cat`, `printf`, `sh`).
fn bash_dirs(bash: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = bash.parent() {
        dirs.push(dir.to_path_buf());
        if cfg!(windows) {
            // Git for Windows: <git>/bin/bash.exe, coreutils under <git>/usr/bin.
            if let Some(git) = dir.parent() {
                dirs.push(git.join("usr").join("bin"));
            }
        }
    }
    if !cfg!(windows) {
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/bin"));
    }
    let mut seen = BTreeSet::new();
    dirs.retain(|d| d.is_dir() && seen.insert(d.clone()));
    dirs
}

/// Forward-slash form; Git Bash accepts `C:/...` and Unix paths are unchanged.
fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    /// Lines `identity|args|CORTEX_HOOK_ORIGIN|CORTEX_API_URL`, one per fake invoked.
    record: Vec<String>,
}

struct Sandbox {
    bash: PathBuf,
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    path_dir: PathBuf,
    record: PathBuf,
}

impl Sandbox {
    fn new(bash: PathBuf) -> Self {
        fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).expect("create workspace test temp root");
        let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("workspace tempdir");
        let root = tmp.path().to_path_buf();
        // A space in HOME proves the shims quote every candidate path.
        let home = root.join("sandbox home");
        let cwd = root.join("checkout");
        let path_dir = root.join("on-path");
        for dir in [&home, &cwd, &path_dir] {
            fs::create_dir_all(dir).expect("create sandbox dir");
        }
        let record = root.join("record.log");
        Self {
            bash,
            _tmp: tmp,
            root,
            home,
            cwd,
            path_dir,
            record,
        }
    }

    /// Writes a fake `cortex-hook` that records its identity, argv, origin and API URL.
    fn fake(&self, dir: &Path, name: &str, identity: &str) -> PathBuf {
        fs::create_dir_all(dir).expect("create fake dir");
        let path = dir.join(name);
        let body = format!(
            "#!/bin/sh\n\
             printf '%s|%s|%s|%s\\n' '{identity}' \"$*\" \"${{CORTEX_HOOK_ORIGIN:-}}\" \
             \"${{CORTEX_API_URL:-}}\" >> \"$CORTEX_FAKE_RECORD\"\n\
             cat >/dev/null\n\
             printf '{{\"fake\":\"{identity}\"}}\\n'\n"
        );
        fs::write(&path, body).expect("write fake cortex-hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake");
        }
        path
    }

    fn sandbox_path(&self) -> OsString {
        let mut dirs = vec![self.path_dir.clone()];
        dirs.extend(bash_dirs(&self.bash));
        std::env::join_paths(dirs).expect("join PATH")
    }

    fn run(&self, script: &Path, args: &[&str], envs: &[(&str, String)]) -> Run {
        let _ = fs::remove_file(&self.record);
        let mut command = Command::new(&self.bash);
        command
            .arg(script)
            .args(args)
            .env_clear()
            .env("PATH", self.sandbox_path())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CORTEX_FAKE_RECORD", slash(&self.record))
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for var in ["SYSTEMROOT", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(var) {
                command.env(var, value);
            }
        }
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn bash");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"{\"session_id\":\"launcher-test\",\"hook_event_name\":\"x\"}\n")
            .expect("write stdin");
        let output = child.wait_with_output().expect("wait for bash");
        let record = fs::read_to_string(&self.record)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        Run {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            record,
        }
    }

    /// The sandbox must not see a real `cortex-hook` through bash's own directories.
    fn ambient_cortex_hook(&self) -> Option<String> {
        let probe = self.root.join("probe.sh");
        fs::write(&probe, "command -v cortex-hook || true\n").expect("write probe");
        let run = self.run(&probe, &[], &[]);
        let found = run.stdout.trim().to_owned();
        (!found.is_empty()).then_some(found)
    }
}

/// Builds a sandbox, or prints why the shell tests are skipped.
fn sandbox(test: &str) -> Option<Sandbox> {
    let Some(bash) = find_bash() else {
        eprintln!("skipping {test}: bash not found (set CORTEX_TEST_BASH to a bash executable)");
        return None;
    };
    let sandbox = Sandbox::new(bash);
    if let Some(found) = sandbox.ambient_cortex_hook() {
        eprintln!("skipping {test}: bash's own PATH already provides cortex-hook at {found}");
        return None;
    }
    Some(sandbox)
}

fn assert_fail_open(run: &Run, context: &str) {
    assert_eq!(
        run.code,
        Some(0),
        "{context}: exit code; stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.stdout, "{}\n",
        "{context}: stdout; stderr: {}",
        run.stderr
    );
    assert!(
        run.record.is_empty(),
        "{context}: no binary may run, got {:?}",
        run.record
    );
}

fn assert_invoked(run: &Run, identity: &str, args: &str, context: &str) -> Vec<String> {
    assert_eq!(
        run.code,
        Some(0),
        "{context}: exit code; stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.record.len(),
        1,
        "{context}: exactly one binary must run, got {:?}; stderr: {}",
        run.record,
        run.stderr
    );
    let fields: Vec<String> = run.record[0].split('|').map(str::to_owned).collect();
    assert_eq!(fields[0], identity, "{context}: wrong binary picked");
    assert_eq!(fields[1], args, "{context}: forwarded arguments");
    assert_eq!(
        run.stdout,
        format!("{{\"fake\":\"{identity}\"}}\n"),
        "{context}: the launcher must exec the binary and pass its stdout through"
    );
    fields
}

fn shim_args(event: &str, synchronous: bool) -> String {
    if synchronous {
        event.to_owned()
    } else {
        format!("{event} --fire-forget")
    }
}

#[test]
fn should_pick_each_discovery_tier_in_order_when_higher_tiers_are_absent() {
    let Some(sb) = sandbox("claude shim discovery order") else {
        return;
    };
    let env_bin = sb.fake(&sb.root.join("env-bin"), "cortex-hook", "env-bin");
    let option_dir = sb.root.join("option dir");
    let option_bin = sb.fake(&option_dir, "cortex-hook", "option-dir");
    let path_bin = sb.fake(&sb.path_dir, "cortex-hook", "path");
    let home_bin = sb.fake(&sb.home.join(".cortex/bin"), "cortex-hook", "home-bin");
    let legacy_bin = sb.fake(&sb.home.join(".cortex"), "cortex-hook", "legacy");

    for &(file, event, synchronous) in CLAUDE_SHIMS {
        let script = claude_hooks_dir().join(file);
        let args = shim_args(event, synchronous);
        let full = [
            ("CORTEX_HOOK_BIN", slash(&env_bin)),
            ("CLAUDE_PLUGIN_OPTION_BIN_DIR", slash(&option_dir)),
        ];

        let run = sb.run(&script, &[], &full);
        assert_invoked(
            &run,
            "env-bin",
            &args,
            &format!("{file}: absolute CORTEX_HOOK_BIN"),
        );

        let run = sb.run(&script, &[], &full[1..]);
        assert_invoked(
            &run,
            "option-dir",
            &args,
            &format!("{file}: absolute bin_dir"),
        );

        fs::rename(&option_bin, sb.root.join("option-parked")).expect("park option bin");
        let run = sb.run(&script, &[], &[]);
        assert_invoked(&run, "path", &args, &format!("{file}: PATH"));

        fs::rename(&path_bin, sb.root.join("path-parked")).expect("park path bin");
        let run = sb.run(&script, &[], &[]);
        assert_invoked(&run, "home-bin", &args, &format!("{file}: ~/.cortex/bin"));

        fs::rename(&home_bin, sb.root.join("home-parked")).expect("park home bin");
        let run = sb.run(&script, &[], &[]);
        assert_invoked(&run, "legacy", &args, &format!("{file}: legacy ~/.cortex"));

        fs::rename(&legacy_bin, sb.root.join("legacy-parked")).expect("park legacy bin");
        let run = sb.run(&script, &[], &[]);
        assert_fail_open(&run, &format!("{file}: nothing installed"));

        // Restore every tier for the next shim.
        fs::rename(sb.root.join("option-parked"), &option_bin).expect("restore");
        fs::rename(sb.root.join("path-parked"), &path_bin).expect("restore");
        fs::rename(sb.root.join("home-parked"), &home_bin).expect("restore");
        fs::rename(sb.root.join("legacy-parked"), &legacy_bin).expect("restore");
    }
}

#[test]
fn should_ignore_relative_overrides_when_a_checkout_ships_a_binary() {
    let Some(sb) = sandbox("claude shim relative overrides") else {
        return;
    };
    // A repository-provided binary reachable only through a relative path.
    sb.fake(&sb.cwd.join("evil"), "cortex-hook", "evil");
    sb.fake(&sb.path_dir, "cortex-hook", "path");

    for &(file, event, synchronous) in CLAUDE_SHIMS {
        let script = claude_hooks_dir().join(file);
        let args = shim_args(event, synchronous);
        for relative in ["evil/cortex-hook", "./evil/cortex-hook"] {
            let run = sb.run(&script, &[], &[("CORTEX_HOOK_BIN", relative.to_owned())]);
            assert_invoked(
                &run,
                "path",
                &args,
                &format!("{file}: CORTEX_HOOK_BIN={relative}"),
            );
        }
        for relative in ["evil", "./evil", "evil/"] {
            let run = sb.run(
                &script,
                &[],
                &[("CLAUDE_PLUGIN_OPTION_BIN_DIR", relative.to_owned())],
            );
            assert_invoked(&run, "path", &args, &format!("{file}: bin_dir={relative}"));
        }
    }
}

#[test]
fn should_fail_open_without_fallback_when_an_absolute_override_is_missing() {
    let Some(sb) = sandbox("claude shim missing override") else {
        return;
    };
    // Every lower tier is installed; an explicit override must still not fall back.
    sb.fake(&sb.path_dir, "cortex-hook", "path");
    sb.fake(&sb.home.join(".cortex/bin"), "cortex-hook", "home-bin");
    sb.fake(&sb.home.join(".cortex"), "cortex-hook", "legacy");
    let empty_dir = sb.root.join("empty option dir");
    fs::create_dir_all(&empty_dir).expect("create empty dir");

    for &(file, _, _) in CLAUDE_SHIMS {
        let script = claude_hooks_dir().join(file);
        let missing = slash(&sb.root.join("missing/cortex-hook"));
        let run = sb.run(&script, &[], &[("CORTEX_HOOK_BIN", missing)]);
        assert_fail_open(&run, &format!("{file}: missing CORTEX_HOOK_BIN"));

        let run = sb.run(
            &script,
            &[],
            &[("CLAUDE_PLUGIN_OPTION_BIN_DIR", slash(&empty_dir))],
        );
        assert_fail_open(&run, &format!("{file}: bin_dir without cortex-hook"));
    }
}

#[test]
fn should_accept_exe_names_and_trailing_separators_when_using_absolute_overrides() {
    let Some(sb) = sandbox("claude shim exe names") else {
        return;
    };
    let script = claude_hooks_dir().join("cortex-pre-tool.sh");
    let option_dir = sb.root.join("option dir");
    let exe = sb.fake(&option_dir, "cortex-hook.exe", "option-exe");

    for suffix in ["", "/"] {
        let dir = format!("{}{suffix}", slash(&option_dir));
        let run = sb.run(
            &script,
            &[],
            &[("CLAUDE_PLUGIN_OPTION_BIN_DIR", dir.clone())],
        );
        assert_invoked(&run, "option-exe", "PreToolUse", &format!("bin_dir={dir}"));
    }
    if cfg!(windows) {
        // Windows-style absolute paths, as Claude Code passes them from plugin userConfig.
        let native_dir = format!("{}\\", option_dir.display());
        let run = sb.run(
            &script,
            &[],
            &[("CLAUDE_PLUGIN_OPTION_BIN_DIR", native_dir)],
        );
        assert_invoked(&run, "option-exe", "PreToolUse", "bin_dir with backslashes");
        let native_bin = exe.display().to_string();
        let run = sb.run(&script, &[], &[("CORTEX_HOOK_BIN", native_bin)]);
        assert_invoked(
            &run,
            "option-exe",
            "PreToolUse",
            "CORTEX_HOOK_BIN with backslashes",
        );
    }

    // `.exe` names in the user directories, for installs copied from a Windows build.
    let home_exe = sb.fake(&sb.home.join(".cortex/bin"), "cortex-hook.exe", "home-exe");
    let run = sb.run(&script, &[], &[]);
    assert_invoked(
        &run,
        "home-exe",
        "PreToolUse",
        "~/.cortex/bin/cortex-hook.exe",
    );
    fs::remove_file(&home_exe).expect("remove home exe");
    sb.fake(&sb.home.join(".cortex"), "cortex-hook.exe", "legacy-exe");
    let run = sb.run(&script, &[], &[]);
    assert_invoked(
        &run,
        "legacy-exe",
        "PreToolUse",
        "~/.cortex/cortex-hook.exe",
    );
}

#[test]
fn should_export_plugin_origin_and_api_url_only_when_running_under_the_plugin_root() {
    let Some(sb) = sandbox("claude shim plugin origin") else {
        return;
    };
    sb.fake(&sb.path_dir, "cortex-hook", "path");
    let api = (
        "CLAUDE_PLUGIN_OPTION_API_URL",
        "http://127.0.0.1:17999".to_owned(),
    );
    let plugin_root = (
        "CLAUDE_PLUGIN_ROOT",
        slash(&workspace_root().join("packages/cortex-claude-plugin")),
    );

    for &(file, event, synchronous) in CLAUDE_SHIMS {
        let script = claude_hooks_dir().join(file);
        let args = shim_args(event, synchronous);

        let run = sb.run(&script, &[], &[plugin_root.clone(), api.clone()]);
        let fields = assert_invoked(&run, "path", &args, &format!("{file}: plugin"));
        assert_eq!(
            fields[2], "plugin",
            "{file}: CORTEX_HOOK_ORIGIN under the plugin"
        );
        assert_eq!(
            fields[3], "http://127.0.0.1:17999",
            "{file}: api_url userConfig"
        );

        // An empty api_url defers to ~/.cortex/config.toml: nothing is exported.
        let run = sb.run(
            &script,
            &[],
            &[
                plugin_root.clone(),
                ("CLAUDE_PLUGIN_OPTION_API_URL", String::new()),
            ],
        );
        let fields = assert_invoked(&run, "path", &args, &format!("{file}: empty api_url"));
        assert_eq!(fields[2], "plugin");
        assert_eq!(
            fields[3], "",
            "{file}: empty api_url must not export CORTEX_API_URL"
        );

        // Standalone settings install: no origin, so the duplicate guard stays off.
        let run = sb.run(&script, &[], std::slice::from_ref(&api));
        let fields = assert_invoked(&run, "path", &args, &format!("{file}: standalone"));
        assert_eq!(
            fields[2], "",
            "{file}: standalone install must not claim plugin origin"
        );
        assert_eq!(
            fields[3], "",
            "{file}: plugin api_url ignored outside the plugin"
        );
    }
}

#[test]
fn should_fail_open_before_discovery_when_the_adapter_is_disabled() {
    let Some(sb) = sandbox("claude shim disabled") else {
        return;
    };
    let bin = sb.fake(&sb.path_dir, "cortex-hook", "path");
    for &(file, _, _) in CLAUDE_SHIMS {
        let run = sb.run(
            &claude_hooks_dir().join(file),
            &[],
            &[
                ("CORTEX_ADAPTER_DISABLE", "1".to_owned()),
                ("CORTEX_HOOK_BIN", slash(&bin)),
            ],
        );
        assert_fail_open(&run, &format!("{file}: CORTEX_ADAPTER_DISABLE=1"));
    }
}

fn codex_launcher() -> PathBuf {
    codex_plugin_dir().join("scripts/cortex-hook-launcher")
}

#[test]
fn should_pick_codex_discovery_tiers_in_order_and_fail_open_when_none_exist() {
    let sb = sandbox("codex discovery").expect("Bash test environment required");
    let path_bin = sb.fake(&sb.path_dir, "cortex", "path");
    let custom = sb.root.join("custom cortex home");
    let custom_bin = sb.fake(&custom.join("bin"), "cortex", "custom");
    let home_bin = sb.fake(&sb.home.join(".cortex/bin"), "cortex", "home");
    let envs = [("CORTEX_HOME", slash(&custom))];
    for &(event, synchronous) in CODEX_EVENTS {
        let run = sb.run(&codex_launcher(), &[event], &envs);
        let fields = assert_invoked(&run, "path", &codex_args(event, synchronous), event);
        assert_eq!(fields[2], "");
    }
    fs::rename(path_bin, sb.root.join("parked-path")).unwrap();
    let run = sb.run(&codex_launcher(), &["Stop"], &envs);
    assert_invoked(&run, "custom", &codex_args("Stop", false), "custom home");
    fs::rename(custom_bin, sb.root.join("parked-custom")).unwrap();
    let run = sb.run(&codex_launcher(), &["Stop"], &envs);
    assert_invoked(&run, "home", &codex_args("Stop", false), "user home");
    fs::rename(home_bin, sb.root.join("parked-home")).unwrap();
    for &(event, _) in CODEX_EVENTS {
        assert_fail_open(&sb.run(&codex_launcher(), &[event], &envs), event);
    }
}

#[test]
fn should_fail_open_without_running_the_binary_when_the_codex_event_is_unknown() {
    let sb = sandbox("unknown codex event").expect("Bash test environment required");
    sb.fake(&sb.path_dir, "cortex", "path");
    for args in [
        &[][..],
        &["PreToolUse"][..],
        &["Notification"][..],
        &["sessionstart"][..],
    ] {
        assert_fail_open(&sb.run(&codex_launcher(), args, &[]), "unsupported event");
    }
}

#[test]
fn standalone_launch_failures_discard_partial_output_and_return_one_json() {
    let sb = sandbox("failed standalone launch").expect("Bash test environment required");
    let fake = sb.fake(&sb.path_dir, "cortex", "failed");
    fs::write(
        &fake,
        "#!/bin/sh\ncat >/dev/null\nprintf 'partial output\\n'\nexit 3\n",
    )
    .unwrap();
    assert_fail_open(
        &sb.run(&codex_launcher(), &["Stop"], &[]),
        "Codex failed executable",
    );
    let script = workspace_root().join("packages/cortex-claude-plugin/hooks/cortex-stop.sh");
    assert_fail_open(&sb.run(&script, &[], &[]), "Claude failed executable");
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn hook_entries<'a>(groups: &'a Value, file: &str, event: &str) -> Vec<&'a Value> {
    let groups = groups
        .as_array()
        .unwrap_or_else(|| panic!("{file}: {event} must hold a list of groups"));
    assert!(!groups.is_empty(), "{file}: {event} has no groups");
    groups
        .iter()
        .flat_map(|group| {
            let hooks = group["hooks"]
                .as_array()
                .unwrap_or_else(|| panic!("{file}: {event} group without a hooks list"));
            assert!(!hooks.is_empty(), "{file}: {event} group with no hooks");
            hooks.iter()
        })
        .collect()
}

#[test]
fn should_declare_documented_codex_events_with_both_commands_when_reading_hooks_json() {
    let plugin = codex_plugin_dir();
    let hooks_json = read_json(&plugin.join("hooks/hooks.json"));
    let hooks = hooks_json["hooks"]
        .as_object()
        .expect("codex hooks.json: hooks object");

    let declared: BTreeSet<&str> = hooks.keys().map(String::as_str).collect();
    let documented: BTreeSet<&str> = CODEX_EVENTS.iter().map(|(event, _)| *event).collect();
    assert_eq!(
        declared, documented,
        "Codex hook events drifted from the documented set"
    );

    let launcher_text = fs::read_to_string(codex_launcher()).expect("read codex launcher");
    for &(event, _) in CODEX_EVENTS {
        for hook in hook_entries(&hooks[event], "codex hooks.json", event) {
            assert_eq!(hook["type"], "command", "{event}: hook type");
            let command = hook["command"]
                .as_str()
                .unwrap_or_else(|| panic!("{event}: missing `command`"));
            let windows = hook["commandWindows"]
                .as_str()
                .unwrap_or_else(|| panic!("{event}: missing `commandWindows`"));

            let script = command
                .split('"')
                .nth(1)
                .unwrap_or_else(|| panic!("{event}: command must quote its script: {command}"));
            let relative = script
                .strip_prefix("${PLUGIN_ROOT}/")
                .unwrap_or_else(|| panic!("{event}: script must live under ${{PLUGIN_ROOT}}"));
            assert!(
                plugin.join(relative).is_file(),
                "{event}: referenced script {relative} does not exist"
            );
            assert!(
                command.ends_with(&format!("\" {event}")),
                "{event}: launcher must receive its own event name: {command}"
            );
            // Codex can select PowerShell on Windows; explicitly enter CMD
            // so the batch invocation also works from a CMD parent.
            assert_eq!(
                windows,
                format!("cmd.exe /d /c call \"${{PLUGIN_ROOT}}\\scripts\\cortex-hook-launcher.cmd\" {event}"),
                "{event}: commandWindows"
            );
            let timeout = hook["timeout"].as_u64().expect("timeout in seconds");
            assert!(
                (1..=5).contains(&timeout),
                "{event}: cortex-hook is bounded at 1.5 s; timeout {timeout}"
            );
        }
    }
    assert!(plugin.join("scripts/cortex-hook-launcher.cmd").is_file());

    assert!(launcher_text.contains("hook --client codex --event"));
    let cmd_text = fs::read_to_string(plugin.join("scripts/cortex-hook-launcher.cmd")).unwrap();
    assert!(cmd_text.contains("hook --client codex --event"));
    assert!(!cmd_text.contains("--fire-forget"));
    for &(event, _) in CODEX_EVENTS {
        assert!(cmd_text.contains(&format!("if \"%EVENT%\"==\"{event}\" goto :resolve")));
    }

    assert!(
        cmd_text.contains("\r\n") && !cmd_text.replace("\r\n", "").contains('\n'),
        "cmd launcher must use CRLF line endings"
    );
}

#[test]
fn should_declare_the_documented_mcp_form_and_a_bumped_version_in_the_codex_plugin() {
    let plugin = codex_plugin_dir();
    let manifest = read_json(&plugin.join(".codex-plugin/plugin.json"));
    assert_eq!(manifest["name"], "cortex");
    // Codex caches installs per version; any plugin change needs a bump.
    assert_eq!(manifest["version"], "0.4.2");
    assert_eq!(manifest["mcpServers"], "./mcp.json");
    let mcp = read_json(&plugin.join("mcp.json"));
    assert_eq!(
        mcp,
        serde_json::json!({"mcpServers": {"cortex": {
            "type": "stdio", "command": "cortex", "args": ["mcp"]
        }}})
    );
}

#[test]
fn should_reference_existing_shims_for_matching_events_when_reading_claude_hooks_json() {
    let plugin = workspace_root().join("packages/cortex-claude-plugin");
    let hooks_json = read_json(&plugin.join("hooks/hooks.json"));
    let hooks = hooks_json["hooks"]
        .as_object()
        .expect("claude hooks.json: hooks object");

    let declared: BTreeSet<&str> = hooks.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = CODEX_EVENTS.iter().map(|(event, _)| *event).collect();
    assert_eq!(declared, expected, "Claude plugin hook events drifted");

    for &(file, event, _) in CLAUDE_SHIMS
        .iter()
        .filter(|(_, event, _)| !["PreToolUse", "Notification"].contains(event))
    {
        for hook in hook_entries(&hooks[event], "claude hooks.json", event) {
            assert_eq!(
                hook["command"].as_str(),
                Some(format!("bash \"${{CLAUDE_PLUGIN_ROOT}}/hooks/{file}\"").as_str()),
                "{event}: command"
            );
        }
        let body =
            fs::read_to_string(plugin.join("hooks").join(file)).expect("read standalone shim");
        assert!(body.contains(&format!("event=\"{event}\"")));
        assert!(body.contains("hook --client claude-code --event"));
    }
}

/// `cortex-hook-launcher.cmd` compatibility under CMD after plugin-root substitution.
/// The standalone plugin tests also exercise Codex's observed PowerShell runtime.
/// The fake `cortex-hook.exe` is a tiny program compiled with `rustc` that
/// records `identity|argv|CORTEX_HOOK_ORIGIN|stdin` and prints `{"fake":…}`.
#[cfg(windows)]
mod windows_cmd_launcher {
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::{fs, io::Write};

    use super::{codex_args, codex_plugin_dir, read_json, CODEX_EVENTS};

    const FAKE_SOURCE: &str = r#"
use std::io::{Read, Write};
fn main() {
    let identity = std::env::current_exe().ok()
        .and_then(|exe| std::fs::read_to_string(exe.with_extension("identity")).ok())
        .unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let origin = std::env::var("CORTEX_HOOK_ORIGIN").unwrap_or_default();
    let record = std::env::var("CORTEX_FAKE_RECORD").expect("record path");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(record).expect("open");
    writeln!(file, "{}|{}|{}|{}", identity.trim(), args.join(" "), origin, stdin.trim()).expect("write");
    println!("{{\"fake\":\"{}\"}}", identity.trim());
}
"#;

    /// Compiles the fake once per test binary; `None` (with a reason) when
    /// `rustc` is unavailable.
    fn fake_exe() -> Option<&'static Path> {
        static FAKE: OnceLock<Option<PathBuf>> = OnceLock::new();
        FAKE.get_or_init(|| {
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("codex-cmd-launcher-fake");
            fs::create_dir_all(&dir).ok()?;
            let source = dir.join("fake_hook.rs");
            fs::write(&source, FAKE_SOURCE).ok()?;
            let exe = dir.join("fake_hook.exe");
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let built = Command::new(rustc)
                .args(["--edition", "2021", "-o"])
                .arg(&exe)
                .arg(&source)
                .status()
                .is_ok_and(|status| status.success());
            built.then_some(exe)
        })
        .as_deref()
    }

    struct Cmd {
        _tmp: tempfile::TempDir,
        plugin_root: PathBuf,
        home: PathBuf,
        cwd: PathBuf,
        path_dir: PathBuf,
        record: PathBuf,
        fake: &'static Path,
    }

    struct CmdRun {
        code: Option<i32>,
        stdout: String,
        stderr: String,
        record: Vec<String>,
    }

    impl Cmd {
        fn new(test: &str) -> Option<Self> {
            let Some(fake) = fake_exe() else {
                panic!("{test}: rustc must build the fake cortex.exe");
            };
            fs::create_dir_all(env!("CARGO_TARGET_TMPDIR"))
                .expect("create workspace test temp root");
            let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("workspace tempdir");
            let root = tmp.path().to_path_buf();
            // Spaces prove the quoting survives cmd /C and the launcher.
            let plugin_root = root.join("plugin root with space");
            let home = root.join("home with space");
            let cwd = root.join("checkout");
            let path_dir = root.join("on path");
            for dir in [&home, &cwd, &path_dir, &plugin_root.join("scripts")] {
                fs::create_dir_all(dir).expect("create dir");
            }
            fs::copy(
                codex_plugin_dir().join("scripts/cortex-hook-launcher.cmd"),
                plugin_root.join("scripts/cortex-hook-launcher.cmd"),
            )
            .expect("copy launcher");
            Some(Self {
                record: root.join("record.log"),
                _tmp: tmp,
                plugin_root,
                home,
                cwd,
                path_dir,
                fake,
            })
        }

        fn install(&self, dir: &Path, identity: &str) {
            fs::create_dir_all(dir).expect("create fake dir");
            fs::copy(self.fake, dir.join("cortex.exe")).expect("copy fake");
            fs::write(dir.join("cortex.identity"), identity).expect("identity");
        }

        /// Runs a CMD parent with its native outer quoting.
        fn run(&self, command: &str, stdin: &[u8]) -> CmdRun {
            let _ = fs::remove_file(&self.record);
            let comspec = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
            let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
            let system32 = Path::new(&system_root).join("System32");
            let mut child = Command::new(comspec)
                .raw_arg(format!("/C \"{command}\""))
                .env_clear()
                .env(
                    "PATH",
                    std::env::join_paths([&self.path_dir, &system32]).unwrap(),
                )
                .env("SystemRoot", &system_root)
                .env("USERPROFILE", &self.home)
                .env("TEMP", &self.home)
                .env("CORTEX_FAKE_RECORD", &self.record)
                .current_dir(&self.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn cmd.exe");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(stdin)
                .expect("write stdin");
            let output = child.wait_with_output().expect("wait for cmd.exe");
            CmdRun {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                record: fs::read_to_string(&self.record)
                    .map(|text| text.lines().map(str::to_owned).collect())
                    .unwrap_or_default(),
            }
        }

        /// The `commandWindows` entry for `event`, with `${PLUGIN_ROOT}` substituted.
        fn hook_command(&self, event: &str) -> String {
            let hooks = read_json(&codex_plugin_dir().join("hooks/hooks.json"));
            hooks["hooks"][event][0]["hooks"][0]["commandWindows"]
                .as_str()
                .unwrap_or_else(|| panic!("{event}: commandWindows"))
                .replace("${PLUGIN_ROOT}", &self.plugin_root.display().to_string())
        }

        fn launcher_command(&self, args: &str) -> String {
            let launcher = self.plugin_root.join("scripts/cortex-hook-launcher.cmd");
            format!("\"{}\" {args}", launcher.display())
        }
    }

    const PAYLOAD: &[u8] = b"{\"session_id\":\"launcher-test\",\"prompt\":\"a & b | c > d\"}\n";

    fn assert_cmd_invoked(run: &CmdRun, identity: &str, args: &str, context: &str) {
        assert_eq!(run.code, Some(0), "{context}: stderr {}", run.stderr);
        assert_eq!(run.stderr, "", "{context}: stderr");
        assert_eq!(
            run.stdout,
            format!("{{\"fake\":\"{identity}\"}}\n"),
            "{context}: only cortex-hook's reply may reach stdout"
        );
        let expected = format!(
            "{identity}|{args}||{}",
            String::from_utf8_lossy(PAYLOAD).trim()
        );
        assert_eq!(run.record, vec![expected], "{context}");
    }

    fn assert_cmd_fail_open(run: &CmdRun, context: &str) {
        assert_eq!(run.code, Some(0), "{context}: stderr {}", run.stderr);
        assert_eq!(run.stdout, "{}\r\n", "{context}: stdout");
        assert_eq!(run.stderr, "", "{context}: stderr");
        assert!(run.record.is_empty(), "{context}: ran {:?}", run.record);
    }

    #[test]
    fn should_run_every_command_windows_entry_through_cmd_quoting_with_a_spaced_plugin_root() {
        let Some(sb) = Cmd::new("codex commandWindows under cmd /C") else {
            return;
        };
        sb.install(&sb.path_dir, "path");
        // A binary planted in the session's working directory is never used.
        sb.install(&sb.cwd, "cwd");
        for &(event, synchronous) in CODEX_EVENTS {
            let run = sb.run(&sb.hook_command(event), PAYLOAD);
            assert_cmd_invoked(
                &run,
                "path",
                &codex_args(event, synchronous),
                &format!("commandWindows {event}"),
            );
        }
        // Codex may hand PLUGIN_ROOT over with forward slashes.
        let forward = sb.hook_command("Stop").replace(
            &sb.plugin_root.display().to_string(),
            &sb.plugin_root.display().to_string().replace('\\', "/"),
        );
        let run = sb.run(&forward, PAYLOAD);
        assert_cmd_invoked(
            &run,
            "path",
            "hook --client codex --event Stop",
            "forward slashes",
        );
    }

    #[test]
    fn should_prefer_path_then_fall_back_to_the_user_cortex_bin() {
        let Some(sb) = Cmd::new("codex cmd discovery order") else {
            return;
        };
        let home_bin = sb.home.join(".cortex").join("bin");
        sb.install(&home_bin, "home-bin");
        sb.install(&sb.path_dir, "path");
        let run = sb.run(&sb.launcher_command("UserPromptSubmit"), PAYLOAD);
        assert_cmd_invoked(
            &run,
            "path",
            "hook --client codex --event UserPromptSubmit",
            "PATH first",
        );

        fs::remove_file(sb.path_dir.join("cortex.exe")).expect("remove PATH fake");
        for &(event, synchronous) in CODEX_EVENTS {
            let run = sb.run(&sb.launcher_command(event), PAYLOAD);
            assert_cmd_invoked(
                &run,
                "home-bin",
                &codex_args(event, synchronous),
                &format!("{event}: %USERPROFILE%\\.cortex\\bin"),
            );
        }
    }

    #[test]
    fn should_print_empty_json_and_drain_stdin_without_a_binary_or_for_unknown_events() {
        let Some(sb) = Cmd::new("codex cmd fail-open") else {
            return;
        };
        let large = [b"{\"prompt\":\"".as_slice(), &[b'x'; 1024 * 1024], b"\"}\n"].concat();
        for &(event, _) in CODEX_EVENTS {
            let run = sb.run(&sb.launcher_command(event), &large);
            assert_cmd_fail_open(&run, &format!("{event}: nothing installed"));
        }
        sb.install(&sb.path_dir, "path");
        for args in ["", "PreToolUse", "Notification", "sessionstart", "\"\""] {
            let run = sb.run(&sb.launcher_command(args), PAYLOAD);
            assert_cmd_fail_open(&run, &format!("launcher args {args:?}"));
        }
    }
}
