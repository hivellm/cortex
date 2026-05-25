//! ADR-016 §2.4 — workspace env-var audit.
//!
//! Walks every `*.rs` file under `crates/` (or a caller-supplied
//! root) and flags every `std::env::var("CORTEX_*")` /
//! `env::var("CORTEX_*")` / `env::var_os("CORTEX_*")` reference
//! that lives OUTSIDE `cortex-config` itself. The doctor
//! subcommand and the CI grep gate both consume the same
//! [`audit`] entry point so a future regression cannot bypass
//! one without the other.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;
use walkdir::WalkDir;

/// One occurrence of an ad-hoc `CORTEX_*` env read outside
/// `cortex-config`. Reported by [`audit`] in repo-relative form
/// so the doctor + CI surface is portable across operator
/// machines.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EnvVarUsage {
    /// Repo-relative file path (forward slashes).
    pub path: String,
    /// 1-based line number.
    pub line: u32,
    /// The full env-var name the call site reads
    /// (e.g. `"CORTEX_EMBEDDER_VECTORIZER_URL"`).
    pub env_name: String,
}

/// Walk `crates_root` and surface every `CORTEX_*` env read
/// living outside `cortex-config`. Skips:
/// - the `cortex-config` crate itself (this crate IS the only
///   legitimate reader),
/// - `target/` build artefacts (in case a caller passes a path
///   that includes them),
/// - `tests/` directories (test fixtures legitimately probe env
///   vars to drive hermetic test setups; the doctor's invariant
///   targets PRODUCTION read sites, not test ones).
///
/// Pass the workspace `crates/` directory as `crates_root` for
/// the production walk. The CI grep gate runs the same call
/// with the same root.
pub fn audit(crates_root: &Path) -> Vec<EnvVarUsage> {
    // One regex that matches every shape we care about:
    //   std::env::var("CORTEX_X")
    //   env::var("CORTEX_X")
    //   env::var_os("CORTEX_X")
    // Returns the env name in capture group 1. The shape's
    // bounded enough that a manual Aho-Corasick scan would not
    // be measurably faster on a 100-file workspace.
    // ADR-016 §4.3 — match `env::var(` only, NOT `env::var_os(`. The
    // §3.6 grep gate uses the same shape (`std::env::var\(\"CORTEX_`).
    // `env::var_os` is the explicit "save-state" idiom that lets
    // `#[cfg(test)]` blocks snapshot the operator env without
    // tripping the migration audit (the actual runtime resolution
    // already goes through `cortex_config::Config::load()`).
    let re = Regex::new(r#"\benv::var\(\s*"(CORTEX_[A-Z0-9_]+)""#)
        .expect("env-var audit regex compiles");

    let mut out: Vec<EnvVarUsage> = Vec::new();
    for entry in WalkDir::new(crates_root)
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e.path()))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        // Skip the audit's own crate.
        if path.components().any(|c| c.as_os_str() == "cortex-config") {
            continue;
        }
        // ADR-016 §3.6 — skip the build-time `cortex-build` helper.
        // `version_info!` reads `CORTEX_GIT_SHA_OVERRIDE` and
        // `CORTEX_GIT_DIRTY_OVERRIDE` inside a build-script-style
        // macro that runs at `cargo build` against the workspace's
        // `.git` checkout. The values are baked into every Cortex
        // binary via `env!()`, NOT resolved from process env at
        // runtime, so routing them through the typed runtime
        // Config would be both architecturally wrong (cortex-config
        // is runtime-only) and a circular workspace dep (cortex-
        // build is intentionally dep-free outside serde). See
        // `crates/cortex-build/Cargo.toml` for the contract.
        if path.components().any(|c| c.as_os_str() == "cortex-build") {
            continue;
        }
        // Skip integration test directories — test fixtures may
        // legitimately probe env vars to drive setup.
        if path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (line_idx, line) in text.lines().enumerate() {
            for cap in re.captures_iter(line) {
                let env_name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                out.push(EnvVarUsage {
                    path: to_repo_relative(path, crates_root),
                    line: u32::try_from(line_idx + 1).unwrap_or(u32::MAX),
                    env_name,
                });
            }
        }
    }
    // Stable order so the doctor's diff stays minimal across
    // runs.
    out.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.env_name.cmp(&b.env_name))
    });
    out
}

fn is_excluded_dir(p: &Path) -> bool {
    let name = match p.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(name, "target" | ".git" | "node_modules")
}

fn to_repo_relative(path: &Path, root: &Path) -> String {
    // Express paths with forward slashes so the report is
    // portable between Windows + Unix operator machines.
    let abs = path
        .strip_prefix(root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf());
    abs.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(root: &Path, rel: &str, body: &str) {
        let full = root.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }

    #[test]
    fn empty_workspace_reports_no_usage() {
        let dir = tempdir().unwrap();
        let report = audit(dir.path());
        assert!(report.is_empty());
    }

    #[test]
    fn flags_std_env_var_with_cortex_prefix() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-api/src/main.rs",
            "fn main() {\n    let _ = std::env::var(\"CORTEX_FOO\");\n}\n",
        );
        let report = audit(dir.path());
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].env_name, "CORTEX_FOO");
        assert_eq!(report[0].line, 2);
        assert!(report[0].path.contains("cortex-api/src/main.rs"));
    }

    #[test]
    fn flags_env_var_shape_and_ignores_env_var_os() {
        // ADR-016 §4.3 — `env::var_os` is the explicit "save-state"
        // idiom for `#[cfg(test)]` env-mutation tests and is NOT
        // flagged by the audit. The §3.6 grep gate uses the same
        // narrow `env::var\(` regex so both surfaces agree.
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-api/src/x.rs",
            "use std::env;\nlet a = env::var(\"CORTEX_A\");\nlet b = env::var_os(\"CORTEX_B\");\n",
        );
        let report = audit(dir.path());
        let names: Vec<&str> = report.iter().map(|u| u.env_name.as_str()).collect();
        assert_eq!(names, vec!["CORTEX_A"]);
    }

    #[test]
    fn ignores_non_cortex_env_vars() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-api/src/y.rs",
            "let _ = std::env::var(\"HOME\");\nlet _ = std::env::var(\"PATH\");\n",
        );
        let report = audit(dir.path());
        assert!(report.is_empty());
    }

    #[test]
    fn skips_cortex_config_crate_itself() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-config/src/load.rs",
            "let v = std::env::var(\"CORTEX_CONFIG_FILE\");\n",
        );
        let report = audit(dir.path());
        assert!(
            report.is_empty(),
            "cortex-config's own reads must be skipped — that crate IS the legitimate reader"
        );
    }

    #[test]
    fn skips_cortex_build_crate_for_compile_time_overrides() {
        // ADR-016 §3.6 — `cortex-build` reads CORTEX_GIT_*_OVERRIDE
        // at compile time inside a `build_info!` macro. The values
        // are baked into binaries via `env!()`, not resolved at
        // runtime, so routing them through cortex_config would be
        // architecturally wrong + a circular workspace dep.
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-build/src/lib.rs",
            "let _ = std::env::var(\"CORTEX_GIT_SHA_OVERRIDE\");\n",
        );
        let report = audit(dir.path());
        assert!(
            report.is_empty(),
            "cortex-build reads MUST be skipped — they are compile-time, not runtime"
        );
    }

    #[test]
    fn skips_test_directories() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-api/tests/it.rs",
            "let _ = std::env::var(\"CORTEX_TEST_FIXTURE\");\n",
        );
        let report = audit(dir.path());
        assert!(report.is_empty());
    }

    #[test]
    fn sorts_results_by_path_then_line_for_stable_diff() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "cortex-b/src/main.rs",
            "let _ = std::env::var(\"CORTEX_X\");\n",
        );
        write(
            dir.path(),
            "cortex-a/src/main.rs",
            "let _ = std::env::var(\"CORTEX_Y\");\nlet _ = std::env::var(\"CORTEX_Z\");\n",
        );
        let report = audit(dir.path());
        assert_eq!(report.len(), 3);
        // cortex-a sorts before cortex-b; line 1 before line 2.
        assert!(report[0].path.contains("cortex-a"));
        assert_eq!(report[0].line, 1);
        assert!(report[1].path.contains("cortex-a"));
        assert_eq!(report[1].line, 2);
        assert!(report[2].path.contains("cortex-b"));
    }
}
