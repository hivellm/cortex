//! Phase35 §1.15 — `cortex-ops setup`, `install-binaries` and
//! `doctor-install` driven through the built binary.
//!
//! The unit tests next to each module cover the planning and copy logic;
//! these cover the CLI contract: dry run unless `--apply`, Cortex home
//! resolution (`--home` / `--to` / `--bin-dir`, then `CORTEX_HOME`, then the
//! user profile), the token never reaching stdout/stderr, and the
//! `doctor-install` exit status. Every child runs with `CORTEX_HOME`,
//! `HOME`, `USERPROFILE`, `CLAUDE_CONFIG_DIR` and the working directory
//! pointed at a tempdir, so the real `~/.cortex`, `~/.claude` and PATH
//! binaries are never read or written.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_cortex-ops");
const BINARIES: [&str; 4] = [
    "cortex-api",
    "cortex-adapter-claude",
    "cortex-hook",
    "cortex-mcp-server",
];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// A `cortex-ops` command isolated inside `sandbox`: no Cortex home, user
/// profile or Claude settings from the host leak in.
fn ops(sandbox: &Path, args: &[&str]) -> Command {
    let profile = sandbox.join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    let mut command = Command::new(BIN);
    command
        .args(args)
        .current_dir(sandbox)
        .env_remove("CORTEX_HOME")
        .env("HOME", &profile)
        .env("USERPROFILE", &profile)
        .env("CLAUDE_CONFIG_DIR", profile.join(".claude"));
    command
}

fn run(command: &mut Command) -> (Output, String) {
    let output = command.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output, text)
}

fn runtime_toml(path: &Path, token: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!(
            "[runtime]\nprofile = \"daily\"\n[dashboard]\napi_url = \"http://127.0.0.1:17000\"\napi_token = \"{token}\"\n[rulebook]\nroot = \"E:/repo/.rulebook\"\n"
        ),
    )
    .unwrap();
}

fn build_dir(root: &Path, marker: &str) -> PathBuf {
    let dir = root.join(format!("build-{marker}"));
    std::fs::create_dir_all(&dir).unwrap();
    for name in BINARIES {
        std::fs::write(dir.join(exe(name)), format!("{name}-{marker}")).unwrap();
    }
    dir
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn should_write_nothing_when_setup_runs_without_apply() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("home");
    let from = sandbox.path().join("runtime.toml");
    runtime_toml(&from, "synthetic-cli-token-dry");

    let (output, text) = run(&mut ops(
        sandbox.path(),
        &["setup", "--home", s(&home), "--from", s(&from)],
    ));
    assert!(output.status.success(), "{text}");
    assert!(text.contains("dry run: nothing written"), "{text}");
    assert!(text.contains("Create: "), "{text}");
    assert!(!home.exists());
    assert!(!text.contains("synthetic-cli-token-dry"), "{text}");
}

#[test]
fn should_write_and_back_up_credential_without_printing_tokens_when_setup_applies() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("home");
    let from = sandbox.path().join("runtime.toml");
    runtime_toml(&from, "synthetic-cli-token-old");

    let (output, text) = run(&mut ops(
        sandbox.path(),
        &["setup", "--home", s(&home), "--from", s(&from), "--apply"],
    ));
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("dry run"), "{text}");
    assert!(!text.contains("synthetic-cli-token-old"), "{text}");
    let credential = home.join("api-token");
    assert_eq!(
        std::fs::read_to_string(&credential).unwrap(),
        "synthetic-cli-token-old\n"
    );
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(!config.contains("synthetic-cli-token-old"));
    assert!(config.contains("root = \"E:/repo/.rulebook\""));

    runtime_toml(&from, "synthetic-cli-token-new");
    let (output, text) = run(&mut ops(
        sandbox.path(),
        &["setup", "--home", s(&home), "--from", s(&from), "--apply"],
    ));
    assert!(output.status.success(), "{text}");
    assert!(!text.contains("synthetic-cli-token-old"), "{text}");
    assert!(!text.contains("synthetic-cli-token-new"), "{text}");
    assert!(text.contains("Unchanged: "), "{text}");
    let backup_line = text
        .lines()
        .find_map(|line| line.strip_prefix("backup: "))
        .unwrap_or_else(|| panic!("no backup line: {text}"));
    let backup = PathBuf::from(backup_line.trim());
    assert_eq!(backup.parent().unwrap(), home);
    assert!(backup
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("api-token.bak-"));
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        "synthetic-cli-token-old\n"
    );
    assert_eq!(
        std::fs::read_to_string(&credential).unwrap(),
        "synthetic-cli-token-new\n"
    );
}

#[test]
fn should_use_builtin_defaults_and_ignore_cwd_runtime_toml_when_from_is_absent() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("home");
    runtime_toml(
        &sandbox
            .path()
            .join(".cortex-state/daily-runtime/runtime.toml"),
        "synthetic-cli-token-cwd",
    );

    let (output, text) = run(&mut ops(
        sandbox.path(),
        &["setup", "--home", s(&home), "--apply"],
    ));
    assert!(output.status.success(), "{text}");
    assert!(text.contains("source: built-in daily defaults"), "{text}");
    assert!(!home.join("api-token").exists());
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("profile = \"daily\""), "{config}");
    assert!(!config.contains("E:/repo/.rulebook"), "{config}");
}

#[test]
fn should_resolve_home_from_cortex_home_when_home_flag_is_absent() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("from-env");
    let (output, text) = run(ops(sandbox.path(), &["setup", "--apply"]).env("CORTEX_HOME", &home));
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains(&format!("cortex home: {}", home.display())),
        "{text}"
    );
    assert!(home.join("config.toml").is_file());
    assert!(!sandbox.path().join("profile/.cortex").exists());
}

#[test]
fn should_fall_back_to_user_profile_when_cortex_home_is_unset() {
    let sandbox = tempfile::tempdir().unwrap();
    let (output, text) = run(&mut ops(sandbox.path(), &["setup"]));
    assert!(output.status.success(), "{text}");
    let expected = sandbox.path().join("profile").join(".cortex");
    assert!(
        text.contains(&format!("cortex home: {}", expected.display())),
        "{text}"
    );
    assert!(!expected.exists());
}

#[test]
fn should_fail_when_no_cortex_home_can_be_resolved() {
    let sandbox = tempfile::tempdir().unwrap();
    let from = build_dir(sandbox.path(), "a");
    for (args, needle) in [
        (vec!["setup"], "setup: no Cortex home"),
        (
            vec!["install-binaries", "--from", s(&from)],
            "install-binaries: no Cortex home",
        ),
        (vec!["doctor-install"], "doctor-install: no Cortex home"),
    ] {
        let (output, text) = run(ops(sandbox.path(), &args)
            .env_remove("HOME")
            .env_remove("USERPROFILE"));
        assert_eq!(output.status.code(), Some(1), "{args:?}: {text}");
        assert!(text.contains(needle), "{text}");
    }
}

#[test]
fn should_write_nothing_when_install_binaries_runs_without_apply() {
    let sandbox = tempfile::tempdir().unwrap();
    let from = build_dir(sandbox.path(), "a");
    let to = sandbox.path().join("bin");
    let (output, text) = run(&mut ops(
        sandbox.path(),
        &["install-binaries", "--from", s(&from), "--to", s(&to)],
    ));
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("dry run: nothing written; re-run with --apply to install"),
        "{text}"
    );
    assert_eq!(text.matches("create: ").count(), BINARIES.len(), "{text}");
    assert!(!to.exists());
}

#[test]
fn should_fail_install_binaries_when_build_dir_lacks_a_binary() {
    let sandbox = tempfile::tempdir().unwrap();
    let from = sandbox.path().join("empty-build");
    std::fs::create_dir_all(&from).unwrap();
    let to = sandbox.path().join("bin");
    let (output, text) = run(&mut ops(
        sandbox.path(),
        &[
            "install-binaries",
            "--from",
            s(&from),
            "--to",
            s(&to),
            "--apply",
        ],
    ));
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("install-binaries: "), "{text}");
    assert!(!to.exists());
}

#[test]
fn should_install_into_cortex_home_bin_and_pass_doctor_when_on_path() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("cortex-home");
    let bin = home.join("bin");
    let build = build_dir(sandbox.path(), "a");
    let elsewhere = sandbox.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();

    let (output, text) = run(ops(
        sandbox.path(),
        &["install-binaries", "--from", s(&build), "--apply"],
    )
    .env("CORTEX_HOME", &home)
    .env("PATH", &elsewhere));
    assert!(output.status.success(), "{text}");
    assert_eq!(
        text.matches("installed: ").count(),
        BINARIES.len(),
        "{text}"
    );
    assert!(text.contains("is not on PATH"), "{text}");
    for name in BINARIES {
        assert_eq!(
            std::fs::read_to_string(bin.join(exe(name))).unwrap(),
            format!("{name}-a")
        );
    }

    let path = std::env::join_paths([elsewhere.clone(), bin.clone()]).unwrap();
    let (output, text) = run(ops(
        sandbox.path(),
        &["doctor-install", "--json", "--build", s(&build)],
    )
    .env("CORTEX_HOME", &home)
    .env("PATH", &path));
    assert_eq!(output.status.code(), Some(0), "{text}");
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(findings.len(), BINARIES.len(), "{text}");
    assert!(findings.iter().all(|f| f["status"] == "ok"), "{text}");

    let (output, text) = run(ops(
        sandbox.path(),
        &["install-binaries", "--from", s(&build), "--apply"],
    )
    .env("CORTEX_HOME", &home)
    .env("PATH", &path));
    assert!(output.status.success(), "{text}");
    assert_eq!(
        text.matches("unchanged: ").count(),
        BINARIES.len(),
        "{text}"
    );
    assert!(!text.contains("is not on PATH"), "{text}");
}

#[test]
fn should_exit_2_when_doctor_finds_missing_stale_or_double_install() {
    let sandbox = tempfile::tempdir().unwrap();
    let bin = sandbox.path().join("bin");
    let build = build_dir(sandbox.path(), "a");
    let (output, text) = run(&mut ops(
        sandbox.path(),
        &[
            "install-binaries",
            "--from",
            s(&build),
            "--to",
            s(&bin),
            "--apply",
        ],
    ));
    assert!(output.status.success(), "{text}");
    let empty = sandbox.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    let (output, text) =
        run(ops(sandbox.path(), &["doctor-install", "--bin-dir", s(&bin)]).env("PATH", &empty));
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert_eq!(text.matches("missing").count(), BINARIES.len(), "{text}");

    let newer = build_dir(sandbox.path(), "b");
    let (output, text) = run(ops(
        sandbox.path(),
        &["doctor-install", "--bin-dir", s(&bin), "--build", s(&newer)],
    )
    .env("PATH", &bin));
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert_eq!(
        text.matches("differs from the build directory").count(),
        BINARIES.len(),
        "{text}"
    );

    let claude = sandbox.path().join("profile").join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        serde_json::json!({
            "enabledPlugins": {"cortex@hivellm-cortex": true},
            "hooks": {"Stop": [{"hooks": [{"command": "cortex-hook Stop"}]}]}
        })
        .to_string(),
    )
    .unwrap();
    let (output, text) = run(ops(
        sandbox.path(),
        &["doctor-install", "--bin-dir", s(&bin), "--json"],
    )
    .env("PATH", &bin));
    assert_eq!(output.status.code(), Some(2), "{text}");
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let statuses: Vec<_> = findings.iter().map(|f| f["status"].clone()).collect();
    assert_eq!(statuses.iter().filter(|s| **s == "ok").count(), 4, "{text}");
    assert!(statuses.contains(&"double_install".into()), "{text}");
}

#[test]
fn should_resolve_binaries_through_path_with_empty_and_missing_entries() {
    let sandbox = tempfile::tempdir().unwrap();
    let bin = sandbox.path().join("my bin");
    let build = build_dir(sandbox.path(), "a");
    let (output, text) = run(&mut ops(
        sandbox.path(),
        &[
            "install-binaries",
            "--from",
            s(&build),
            "--to",
            s(&bin),
            "--apply",
        ],
    ));
    assert!(output.status.success(), "{text}");

    // An empty entry and a nonexistent entry ahead of the install dir must
    // not hide it (the sandbox cwd holds no binaries). Windows also accepts
    // a quoted entry, which is how a directory with a `;` would be written.
    let missing = sandbox.path().join("does-not-exist");
    let path = if cfg!(windows) {
        format!("{};;\"{}\"", missing.display(), bin.display())
    } else {
        format!("{}::{}", missing.display(), bin.display())
    };
    let (output, text) = run(ops(
        sandbox.path(),
        &["doctor-install", "--bin-dir", s(&bin), "--json"],
    )
    .env("PATH", &path));
    assert_eq!(output.status.code(), Some(0), "{text}");
    let findings: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(findings.iter().all(|f| f["status"] == "ok"), "{text}");
}
