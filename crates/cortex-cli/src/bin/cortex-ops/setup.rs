//! Phase35 §1.3 — `cortex-ops setup`: write the user runtime configuration
//! (`<Cortex home>/config.toml`) and the API credential
//! (`<Cortex home>/api-token`) so binaries need no client-supplied paths,
//! environment blocks or tokens.
//!
//! Dry-run by default; `--apply` writes. `--from` migrates an existing
//! runtime TOML (for example `.cortex-state/daily-runtime/runtime.toml`):
//! `dashboard.api_token` moves into the credential file and every other value
//! is copied verbatim. Re-running with the same input changes nothing. Any
//! file that would change is first copied to a timestamped backup whose
//! SHA-256 is verified before the original is replaced. The token is never
//! printed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

const CONFIG_HEADER: &str = "# Cortex user runtime configuration, written by `cortex-ops setup`.\n# The API token lives in the sibling `api-token` file, never here.\n";
const DEFAULT_API_URL: &str = "http://127.0.0.1:17000";

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Create,
    Unchanged,
    Replace,
    Keep,
}

#[derive(Debug)]
struct Planned {
    path: PathBuf,
    contents: Option<Vec<u8>>,
    secret: bool,
    action: Action,
}

#[derive(Debug)]
struct Plan {
    home: PathBuf,
    config: Planned,
    credential: Planned,
}

pub(super) fn setup(apply: bool, from: Option<PathBuf>, home: Option<PathBuf>) -> ExitCode {
    let Some(home) = home.or_else(cortex_config::cortex_home) else {
        eprintln!("setup: no Cortex home; set CORTEX_HOME or pass --home");
        return ExitCode::FAILURE;
    };
    let result = build_plan(&home, from.as_deref()).and_then(|plan| {
        print_plan(&plan, from.as_deref());
        if apply {
            apply_plan(
                &plan,
                &chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
            )
        } else {
            println!("dry run: nothing written; re-run with --apply to write");
            Ok(())
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("setup: {error}");
            ExitCode::FAILURE
        }
    }
}

fn build_plan(home: &Path, from: Option<&Path>) -> Result<Plan, String> {
    let mut table = match from {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            raw.parse::<toml::Table>()
                .map_err(|_| format!("{} is not valid TOML", path.display()))?
        }
        None => default_table(),
    };
    let token = take_token(&mut table)?;
    let body = toml::to_string(&table).map_err(|e| format!("serialize config: {e}"))?;
    let config_text = format!("{CONFIG_HEADER}{body}");
    let parsed: cortex_config::Config =
        toml::from_str(&config_text).map_err(|e| format!("migrated config is invalid: {e}"))?;
    parsed
        .validate()
        .map_err(|e| format!("migrated config is invalid: {e}"))?;

    let config_path = home.join("config.toml");
    let credential_path = home.join(cortex_config::API_TOKEN_FILE);
    let config_bytes = config_text.into_bytes();
    let config_action = compare(&config_path, &config_bytes)?;
    let (credential_bytes, credential_action) = match token {
        Some(token) => {
            let bytes = format!("{token}\n").into_bytes();
            let action = compare(&credential_path, &bytes)?;
            (Some(bytes), action)
        }
        None => (None, Action::Keep),
    };
    Ok(Plan {
        home: home.to_path_buf(),
        config: Planned {
            path: config_path,
            contents: Some(config_bytes),
            secret: false,
            action: config_action,
        },
        credential: Planned {
            path: credential_path,
            contents: credential_bytes,
            secret: true,
            action: credential_action,
        },
    })
}

fn default_table() -> toml::Table {
    let mut runtime = toml::Table::new();
    runtime.insert("profile".into(), "daily".into());
    let mut dashboard = toml::Table::new();
    dashboard.insert("api_url".into(), DEFAULT_API_URL.into());
    let mut table = toml::Table::new();
    table.insert("runtime".into(), runtime.into());
    table.insert("dashboard".into(), dashboard.into());
    table
}

fn take_token(table: &mut toml::Table) -> Result<Option<String>, String> {
    let Some(dashboard) = table.get_mut("dashboard") else {
        return Ok(None);
    };
    let dashboard = dashboard
        .as_table_mut()
        .ok_or("[dashboard] is not a table")?;
    match dashboard.remove("api_token") {
        None => Ok(None),
        Some(toml::Value::String(token)) => {
            let token = token.trim();
            if token.chars().any(char::is_control) {
                return Err("dashboard.api_token contains control characters".into());
            }
            Ok((!token.is_empty()).then(|| token.to_string()))
        }
        Some(_) => Err("dashboard.api_token is not a string".into()),
    }
}

fn compare(path: &Path, wanted: &[u8]) -> Result<Action, String> {
    match std::fs::read(path) {
        Ok(existing) if existing == wanted => Ok(Action::Unchanged),
        Ok(_) => Ok(Action::Replace),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Action::Create),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn print_plan(plan: &Plan, from: Option<&Path>) {
    println!("cortex home: {}", plan.home.display());
    match from {
        Some(path) => println!("source: {}", path.display()),
        None => println!("source: built-in daily defaults"),
    }
    for item in [&plan.config, &plan.credential] {
        let detail = match (&item.contents, item.secret) {
            (Some(bytes), false) => format!("sha256 {}", sha256_hex(bytes)),
            (Some(_), true) => "secret, user-only access".into(),
            (None, _) => "no token in source; existing file untouched".into(),
        };
        println!("{:?}: {} ({detail})", item.action, item.path.display());
    }
    if let Some(bytes) = &plan.config.contents {
        println!("--- config.toml ---\n{}", String::from_utf8_lossy(bytes));
    }
}

fn apply_plan(plan: &Plan, stamp: &str) -> Result<(), String> {
    create_home(&plan.home)?;
    for item in [&plan.config, &plan.credential] {
        let Some(contents) = &item.contents else {
            continue;
        };
        match item.action {
            Action::Unchanged | Action::Keep => continue,
            Action::Replace => {
                let backup = backup(&item.path, stamp, item.secret)?;
                println!("backup: {}", backup.display());
            }
            Action::Create => {}
        }
        write_atomic(&item.path, contents, item.secret)?;
        println!("wrote: {}", item.path.display());
    }
    Ok(())
}

fn create_home(home: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(home)
        .map_err(|e| format!("create {}: {e}", home.display()))
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn backup(path: &Path, stamp: &str, secret: bool) -> Result<PathBuf, String> {
    let original = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let target = sibling(path, &format!(".bak-{stamp}"));
    write_new(&target, &original, secret)?;
    let copied = std::fs::read(&target).map_err(|e| format!("read {}: {e}", target.display()))?;
    if sha256_hex(&copied) != sha256_hex(&original) {
        return Err(format!("backup {} failed verification", target.display()));
    }
    Ok(target)
}

fn write_atomic(path: &Path, contents: &[u8], secret: bool) -> Result<(), String> {
    let temporary = sibling(path, ".tmp");
    write_new(&temporary, contents, secret)?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("replace {}: {e}", path.display())
    })
}

/// Create `path` (which must not exist) with `contents`; secrets are
/// restricted to the current user before any byte is written on Unix, and
/// before the file is renamed into place on Windows.
fn write_new(path: &Path, contents: &[u8], secret: bool) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if secret {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    // A failed open created nothing, so it must not remove `path`: in
    // `backup` that is an earlier backup with the same stamp.
    let mut file = options
        .open(path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    let result = file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("write {}: {e}", path.display()))
        .and_then(|()| {
            if secret {
                restrict_to_user(path)
            } else {
                Ok(())
            }
        });
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[cfg(windows)]
fn restrict_to_user(path: &Path) -> Result<(), String> {
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(name)) if !domain.is_empty() && !name.is_empty() => {
            format!("{domain}\\{name}")
        }
        _ => return Err("cannot determine the current Windows user".into()),
    };
    let status = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{user}:F"))
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("run icacls: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("icacls could not restrict {}", path.display()))
    }
}

#[cfg(not(windows))]
fn restrict_to_user(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "synthetic-setup-token-77";

    fn source(dir: &Path) -> PathBuf {
        let path = dir.join("runtime.toml");
        std::fs::write(
            &path,
            format!(
                "[runtime]\nprofile = \"daily\"\n[dashboard]\napi_url = \"http://127.0.0.1:17000\"\napi_token = \"{SECRET}\"\n[rulebook]\nroot = \"E:/repo/.rulebook\"\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn dry_run_plan_writes_nothing_and_splits_token() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let plan = build_plan(&home, Some(&source(dir.path()))).unwrap();
        assert!(!home.exists());
        assert_eq!(plan.config.action, Action::Create);
        assert_eq!(plan.credential.action, Action::Create);
        let config = String::from_utf8(plan.config.contents.clone().unwrap()).unwrap();
        assert!(!config.contains(SECRET));
        assert!(config.contains("root = \"E:/repo/.rulebook\""));
        assert!(!format!("{plan:?}").contains(&format!("\"{SECRET}")));
    }

    #[test]
    fn apply_is_idempotent_and_loadable() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let from = source(dir.path());
        apply_plan(&build_plan(&home, Some(&from)).unwrap(), "s1").unwrap();
        let mut config =
            cortex_config::Config::load_from(&home.join("config.toml"), |_| None).unwrap();
        assert!(config.is_daily());
        assert_eq!(config.dashboard.api_token, None);
        config
            .apply_api_token_file(Some(&home.join(cortex_config::API_TOKEN_FILE)))
            .unwrap();
        assert_eq!(config.dashboard.api_token.as_deref(), Some(SECRET));

        let again = build_plan(&home, Some(&from)).unwrap();
        assert_eq!(again.config.action, Action::Unchanged);
        assert_eq!(again.credential.action, Action::Unchanged);
        apply_plan(&again, "s2").unwrap();
        let names: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn changed_files_are_backed_up_before_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "# operator edit\n").unwrap();
        let plan = build_plan(&home, None).unwrap();
        assert_eq!(plan.config.action, Action::Replace);
        assert_eq!(plan.credential.action, Action::Keep);
        apply_plan(&plan, "s3").unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml.bak-s3")).unwrap(),
            "# operator edit\n"
        );
        assert!(!home.join(cortex_config::API_TOKEN_FILE).exists());
        let config = cortex_config::Config::load_from(&home.join("config.toml"), |_| None).unwrap();
        assert!(config.is_daily());
        assert_eq!(config.dashboard.api_url.as_deref(), Some(DEFAULT_API_URL));
    }

    #[test]
    fn invalid_sources_are_rejected_without_echoing_values() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        for (body, needle) in [
            (format!("api_token = \"{SECRET}\" ["), "not valid TOML"),
            ("[dashboard]\napi_token = 7\n".into(), "not a string"),
            ("schema_version = \"v9\"\n".into(), "invalid"),
        ] {
            let path = dir.path().join("bad.toml");
            std::fs::write(&path, body).unwrap();
            let error = build_plan(&home, Some(&path)).unwrap_err();
            assert!(error.contains(needle), "{error}");
            assert!(!error.contains(SECRET));
        }
    }

    fn source_with_token(dir: &Path, name: &str, token: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("[runtime]\nprofile = \"daily\"\n[dashboard]\napi_token = \"{token}\"\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn should_back_up_existing_credential_when_token_changes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let old = source_with_token(dir.path(), "old.toml", "synthetic-old-token-1");
        let new = source_with_token(dir.path(), "new.toml", "synthetic-new-token-2");
        apply_plan(&build_plan(&home, Some(&old)).unwrap(), "s1").unwrap();

        let plan = build_plan(&home, Some(&new)).unwrap();
        assert_eq!(plan.config.action, Action::Unchanged);
        assert_eq!(plan.credential.action, Action::Replace);
        assert!(!format!("{plan:?}").contains("\"synthetic-new-token-2"));
        apply_plan(&plan, "s2").unwrap();

        let credential = home.join(cortex_config::API_TOKEN_FILE);
        let backup = sibling(&credential, ".bak-s2");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "synthetic-old-token-1\n"
        );
        assert_eq!(
            std::fs::read_to_string(&credential).unwrap(),
            "synthetic-new-token-2\n"
        );
        assert!(!home.join("config.toml.bak-s2").exists());
        assert!(!sibling(&credential, ".tmp").exists());
    }

    #[test]
    fn should_keep_existing_credential_when_source_token_is_blank() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let first = source_with_token(dir.path(), "first.toml", "synthetic-kept-token");
        apply_plan(&build_plan(&home, Some(&first)).unwrap(), "s1").unwrap();
        let blank = source_with_token(dir.path(), "blank.toml", "   ");
        let plan = build_plan(&home, Some(&blank)).unwrap();
        assert_eq!(plan.credential.action, Action::Keep);
        apply_plan(&plan, "s2").unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(cortex_config::API_TOKEN_FILE)).unwrap(),
            "synthetic-kept-token\n"
        );
    }

    #[test]
    fn should_reject_token_with_control_characters_without_echoing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.toml");
        std::fs::write(&path, "[dashboard]\napi_token = \"synthetic\\u0007bell\"\n").unwrap();
        let error = build_plan(&dir.path().join("home"), Some(&path)).unwrap_err();
        assert!(error.contains("control characters"), "{error}");
        assert!(!error.contains("synthetic"));
    }

    #[test]
    fn should_fail_backup_and_keep_original_when_backup_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "current").unwrap();
        std::fs::write(sibling(&path, ".bak-s"), "earlier backup").unwrap();
        let error = backup(&path, "s", false).unwrap_err();
        assert!(error.contains(".bak-s"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "current");
        assert_eq!(
            std::fs::read_to_string(sibling(&path, ".bak-s")).unwrap(),
            "earlier backup"
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_make_credential_backup_user_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        std::fs::write(&path, "synthetic-old-token\n").unwrap();
        let target = backup(&path, "s", true).unwrap();
        let mode = std::fs::metadata(target).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[cfg(windows)]
    #[test]
    fn should_restrict_secret_to_current_user_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        write_new(&path, b"synthetic-acl-token\n", true).unwrap();
        let backup_path = backup(&path, "s", true).unwrap();
        let user = format!(
            "{}\\{}",
            std::env::var("USERDOMAIN").unwrap(),
            std::env::var("USERNAME").unwrap()
        )
        .to_lowercase();
        for file in [&path, &backup_path] {
            let output = std::process::Command::new("icacls")
                .arg(file)
                .output()
                .unwrap();
            assert!(output.status.success());
            let acl = String::from_utf8_lossy(&output.stdout).to_lowercase();
            // icacls prints `<path> <first ACE>` then one ACE per line, then
            // a localized summary line; keep the lines that carry an ACE.
            let aces: Vec<&str> = acl
                .lines()
                .map(|line| line.trim_start_matches(&*file.display().to_string().to_lowercase()))
                .map(str::trim)
                .filter(|line| line.contains(":("))
                .collect();
            assert_eq!(aces, vec![format!("{user}:(f)")], "{acl}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn credential_is_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        apply_plan(&build_plan(&home, Some(&source(dir.path()))).unwrap(), "s4").unwrap();
        let mode = std::fs::metadata(home.join(cortex_config::API_TOKEN_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
