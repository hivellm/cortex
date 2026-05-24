//! ADR-016 §4 — `cortex-ops doctor-config-audit` — walks the
//! workspace via `cortex_config::audit` and reports every
//! ad-hoc `std :: env :: var ("CORTEX_*")` call site living outside
//! `cortex-config`. Exit code `0` when the report is empty
//! (every knob bound through the typed `Config`), `2` otherwise.
//!
//! Acts as the CI grep gate's user-facing counterpart — both
//! surfaces call the same library entry point so a regression
//! that adds a new ad-hoc env read fails both in lockstep.

use std::path::PathBuf;
use std::process::ExitCode;

use cortex_config::{audit, EnvVarUsage};
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Serialize)]
struct ReportEnvelope {
    crates_root: String,
    total: usize,
    failed: bool,
    usages: Vec<EnvVarUsage>,
}

pub(super) fn doctor_config_audit(crates_root: Option<String>, json_output: bool) -> ExitCode {
    let root = match resolve_crates_root(crates_root) {
        Some(p) => p,
        None => {
            eprintln!(
                "doctor-config-audit: could not resolve crates/ root; pass --crates-root explicitly"
            );
            return ExitCode::FAILURE;
        }
    };
    if !root.exists() {
        eprintln!(
            "doctor-config-audit: crates root {} not found",
            root.display()
        );
        return ExitCode::FAILURE;
    }

    let usages = audit(&root);
    let envelope = ReportEnvelope {
        crates_root: root.display().to_string(),
        total: usages.len(),
        failed: !usages.is_empty(),
        usages,
    };

    if json_output {
        match serde_json::to_string_pretty(&json!(envelope)) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-config-audit: serialise: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render_text(&envelope);
    }

    if envelope.failed {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn resolve_crates_root(arg: Option<String>) -> Option<PathBuf> {
    if let Some(p) = arg {
        return Some(PathBuf::from(p));
    }
    // Walk upward from cwd looking for a `crates/` directory.
    let mut cursor = std::env::current_dir().ok()?;
    for _ in 0..6 {
        let candidate = cursor.join("crates");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !cursor.pop() {
            break;
        }
    }
    None
}

fn render_text(r: &ReportEnvelope) {
    println!("cortex-ops doctor-config-audit");
    println!("crates_root:  {}", r.crates_root);
    println!("total_usages: {}", r.total);
    if !r.failed {
        println!();
        println!("OK — every CORTEX_* env read lives inside cortex-config.");
        return;
    }
    println!();
    println!("FAILED — ad-hoc env reads outside cortex-config:");
    println!();
    // Tabular: <path>:<line>  <env_name>
    let path_width = r
        .usages
        .iter()
        .map(|u| u.path.len() + 1 + u.line.to_string().len())
        .max()
        .unwrap_or(0);
    for u in &r.usages {
        let location = format!("{}:{}", u.path, u.line);
        println!("  {:<width$}  {}", location, u.env_name, width = path_width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(root: &std::path::Path, rel: &str, body: &str) {
        let full = root.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }

    #[test]
    fn empty_crates_root_exits_success() {
        let dir = tempdir().unwrap();
        let code = doctor_config_audit(Some(dir.path().display().to_string()), true);
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    }

    #[test]
    fn ad_hoc_read_exits_with_code_2() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-api/src/main.rs",
            "let _ = std::env::var(\"CORTEX_X\");\n",
        );
        let code = doctor_config_audit(Some(dir.path().display().to_string()), true);
        // ExitCode::from(2) prints as "ExitCode(unix_exit_status(2))" or
        // "ExitCode(2)" depending on platform — compare against the
        // same constructor instead of relying on Debug.
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::from(2)));
    }
}
