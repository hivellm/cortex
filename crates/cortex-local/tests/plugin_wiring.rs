//! Packaged launchers exercise the real standalone executable, including spaced paths.
use cortex_local::{Authority, CaptureRequest, LocalStore};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
    "Stop",
    "SubagentStop",
];
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn bash() -> PathBuf {
    if let Some(path) = std::env::var_os("CORTEX_TEST_BASH") {
        return path.into();
    }
    if cfg!(windows) {
        PathBuf::from("C:/Program Files/Git/bin/bash.exe")
    } else {
        PathBuf::from("/bin/bash")
    }
}
fn tools_path(shell: &Path) -> Vec<PathBuf> {
    let mut paths = vec![shell.parent().unwrap().to_owned()];
    if cfg!(windows) {
        paths.push(shell.parent().unwrap().parent().unwrap().join("usr/bin"));
        paths.push(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"));
    } else {
        paths.push(PathBuf::from("/usr/bin"));
    }
    paths
}
fn invoke(
    mut command: Command,
    home: &Path,
    repo: &Path,
    paths: Vec<PathBuf>,
    event: &str,
) -> Value {
    command
        .env_clear()
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CORTEX_HOME", home)
        .env("TEMP", home)
        .env("TMP", home)
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(value) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", value);
    }
    if cfg!(windows) {
        command.env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
    }
    let program = command.get_program().to_owned();
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(json!({"cwd":repo,"session_id":"native-session","turn_id":event,"event_id":event,"prompt":"build","last_assistant_message":"done","tool_response":{"ok":true}}).to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{program:?} {event}: expected one JSON response: {error}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn packaged_posix_launchers_capture_with_one_binary_from_spaced_custom_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("custom home with spaces");
    let repo = temp.path().join("repo with spaces");
    fs::create_dir_all(home.join("bin")).unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("cortex.toml"), "[cortex]\nid='Cortex'\n").unwrap();
    let binary = if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    };
    fs::copy(env!("CARGO_BIN_EXE_cortex"), home.join("bin").join(binary)).unwrap();
    let store = LocalStore::open(home.join("local")).unwrap();
    store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            title: "Build evidence".into(),
            body: "Run tests".into(),
            authority: Authority::Accepted,
            ..Default::default()
        })
        .unwrap();
    let shell = bash();
    assert!(shell.is_file(), "Bash required for packaged POSIX hooks");
    let claude_files = [
        "session-start",
        "user-prompt",
        "post-tool",
        "stop",
        "subagent-stop",
    ];
    for client in ["codex", "claude-code"] {
        for (index, event) in EVENTS.iter().enumerate() {
            let mut command = Command::new(&shell);
            if client == "codex" {
                command
                    .arg(
                        workspace()
                            .join("packages/cortex-codex-plugin/scripts/cortex-hook-launcher"),
                    )
                    .arg(event);
            } else {
                command.arg(workspace().join(format!(
                    "packages/cortex-claude-plugin/hooks/cortex-{}.sh",
                    claude_files[index]
                )));
            }
            let response = invoke(command, &home, &repo, tools_path(&shell), event);
            if index < 2 {
                assert!(response["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .contains("Build evidence"));
            } else {
                assert_eq!(response, json!({}));
            }
        }
    }
    let records = store.recent("Cortex", 100, true).unwrap();
    assert_eq!(records.len(), 9);
    for client in ["codex", "claude-code"] {
        assert_eq!(
            records
                .iter()
                .filter(|r| r.metadata["client"] == client)
                .count(),
            4
        );
    }
}

#[cfg(windows)]
#[test]
fn command_windows_uses_real_binary_and_spaced_plugin_root() {
    use std::os::windows::process::CommandExt;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("custom cortex home");
    let repo = temp.path().join("repo");
    let plugin = temp.path().join("plugin root with spaces");
    for path in [home.join("bin"), repo.clone(), plugin.join("scripts")] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(repo.join("cortex.toml"), "[cortex]\nid='Cortex'\n").unwrap();
    fs::copy(env!("CARGO_BIN_EXE_cortex"), home.join("bin/cortex.exe")).unwrap();
    fs::copy(
        workspace().join("packages/cortex-codex-plugin/scripts/cortex-hook-launcher.cmd"),
        plugin.join("scripts/cortex-hook-launcher.cmd"),
    )
    .unwrap();
    let store = LocalStore::open(home.join("local")).unwrap();
    store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            title: "Build evidence".into(),
            body: "Run tests".into(),
            authority: Authority::Accepted,
            ..Default::default()
        })
        .unwrap();
    let hooks: Value = serde_json::from_slice(
        &fs::read(workspace().join("packages/cortex-codex-plugin/hooks/hooks.json")).unwrap(),
    )
    .unwrap();
    for event in EVENTS {
        let raw = hooks["hooks"][event][0]["hooks"][0]["commandWindows"]
            .as_str()
            .unwrap()
            .replace("${PLUGIN_ROOT}", &plugin.to_string_lossy());
        let mut cmd = Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
        cmd.raw_arg(format!("/C \"{raw}\""));
        // Codex 0.162 selects PowerShell on this host, even for commandWindows.
        let powershell_path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("pwsh.exe"))
            .find(|path| path.is_file())
            .expect("PowerShell 7 required for the real Codex Windows hook regression");
        let mut powershell = Command::new(powershell_path);
        powershell.args(["-NoProfile", "-Command", &raw]);
        for command in [cmd, powershell] {
            let response = invoke(command, &home, &repo, tools_path(&bash()), event);
            if matches!(event, "SessionStart" | "UserPromptSubmit") {
                assert!(response["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .contains("Build evidence"));
            } else {
                assert_eq!(response, json!({}));
            }
        }
    }
    let records = store.recent("Cortex", 100, true).unwrap();
    assert_eq!(records.len(), 5, "{records:#?}");
}

#[test]
fn skills_only_advertise_implemented_mcp_names_and_matching_version() {
    let names: std::collections::BTreeSet<String> = cortex_local::mcp::descriptors()
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    for (plugin, manifest) in [
        ("cortex-codex-plugin", ".codex-plugin/plugin.json"),
        ("cortex-claude-plugin", ".claude-plugin/plugin.json"),
    ] {
        let root = workspace().join("packages").join(plugin);
        let value: Value = serde_json::from_slice(&fs::read(root.join(manifest)).unwrap()).unwrap();
        assert_eq!(value["version"], "0.4.2");
        let skill = fs::read_to_string(root.join("skills/cortex-memory/SKILL.md")).unwrap();
        for token in skill
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|token| token.starts_with("cortex_"))
        {
            assert!(names.contains(token), "unsupported tool {token}");
        }
        for name in &names {
            assert!(skill.contains(name), "undocumented tool {name}");
        }
    }
}
