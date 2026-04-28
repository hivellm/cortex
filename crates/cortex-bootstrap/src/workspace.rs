//! Workspace orchestration — drive `cortex-bootstrap` against a
//! TOML-listed set of repos in one invocation.
//!
//! Phase4b §1: the existing CLI already accepts multiple `repo_roots`
//! positional args, but listing all 17 Hive repo paths on the command
//! line is brittle. The workspace TOML carries the same data in a
//! source-controlled file plus optional metadata (per-entry config
//! override) and a single pre-flight verifier that surfaces every
//! configuration mistake at once instead of failing on the first one.
//!
//! ```toml
//! [[repo]]
//! id = "Cortex"
//! path = "E:/HiveLLM/Cortex"
//!
//! [[repo]]
//! id = "Vectorizer"
//! path = "E:/HiveLLM/Vectorizer"
//! config = "cortex.toml"   # optional; defaults to <path>/cortex.toml
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One repo entry in a workspace TOML.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceRepo {
    /// Logical repo identifier — must match the repo's
    /// `cortex.toml` `[cortex] id` (or the directory name when no
    /// override is set). Used as the checkpoint key, so it must be
    /// stable across invocations.
    pub id: String,
    /// Absolute (or repo-rooted) path to the git checkout.
    pub path: PathBuf,
    /// Optional `cortex.toml` override path. When `None` the
    /// runner falls back to `<path>/cortex.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<PathBuf>,
}

/// Top-level workspace TOML shape.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WorkspaceConfig {
    /// Repos in declaration order — orchestrator iterates this list
    /// in order so the operator can scope `--only` / `--skip`
    /// against a stable index.
    #[serde(default, rename = "repo")]
    pub repos: Vec<WorkspaceRepo>,
}

/// Failure modes raised while loading or verifying a workspace.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// I/O while reading the TOML file.
    #[error("workspace io: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse failure.
    #[error("workspace parse: {0}")]
    Parse(#[from] toml::de::Error),
    /// One or more entries had structural problems. Each line in the
    /// inner vec is a human-readable failure description; the
    /// orchestrator prints them all together so the operator fixes
    /// every issue in one pass.
    #[error("workspace preflight failed:\n  - {}", .0.join("\n  - "))]
    Preflight(Vec<String>),
}

/// Read a workspace TOML from disk.
pub fn load_workspace(path: &Path) -> Result<WorkspaceConfig, WorkspaceError> {
    let body = std::fs::read_to_string(path)?;
    let cfg: WorkspaceConfig = toml::from_str(&body)?;
    Ok(cfg)
}

/// Verify that every entry in `cfg`:
///
/// - has a non-empty `id`;
/// - has a unique `id` within the workspace (duplicates are rejected up-front);
/// - points at an existing path that is a git checkout (`<path>/.git` exists);
/// - has a readable `cortex.toml` (either at `<path>/cortex.toml` or
///   at the explicit `config` override).
///
/// Returns `Ok(())` when every check passes; otherwise [`WorkspaceError::Preflight`]
/// carrying every failure description so the operator sees the full list at once.
pub fn preflight(cfg: &WorkspaceConfig) -> Result<(), WorkspaceError> {
    let mut errors: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for (idx, repo) in cfg.repos.iter().enumerate() {
        if repo.id.trim().is_empty() {
            errors.push(format!("entry #{idx}: `id` is empty"));
            continue;
        }
        if !seen.insert(repo.id.as_str()) {
            errors.push(format!("duplicate id `{}` (entry #{idx})", repo.id));
        }
        if !repo.path.exists() {
            errors.push(format!(
                "`{}`: path `{}` does not exist",
                repo.id,
                repo.path.display()
            ));
            continue;
        }
        let git_dir = repo.path.join(".git");
        if !git_dir.exists() {
            errors.push(format!(
                "`{}`: path `{}` is not a git checkout (no .git/)",
                repo.id,
                repo.path.display()
            ));
            // Don't `continue` — the cortex.toml check below is still
            // useful diagnostically, but skip the cortex.toml read
            // when the directory clearly isn't a repo.
            continue;
        }
        let cortex_toml = match &repo.config {
            Some(rel) if rel.is_absolute() => rel.clone(),
            Some(rel) => repo.path.join(rel),
            None => repo.path.join("cortex.toml"),
        };
        if !cortex_toml.exists() {
            errors.push(format!(
                "`{}`: cortex.toml not found at `{}`",
                repo.id,
                cortex_toml.display()
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkspaceError::Preflight(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_repo(dir: &Path, with_git: bool, with_toml: bool) {
        std::fs::create_dir_all(dir).unwrap();
        if with_git {
            std::fs::create_dir_all(dir.join(".git")).unwrap();
        }
        if with_toml {
            std::fs::write(dir.join("cortex.toml"), "[cortex]\nid = \"x\"\n").unwrap();
        }
    }

    #[test]
    fn load_workspace_round_trips_two_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ws.toml");
        std::fs::write(
            &path,
            r#"
[[repo]]
id = "A"
path = "C:/HiveLLM/A"

[[repo]]
id = "B"
path = "C:/HiveLLM/B"
config = "alt.toml"
"#,
        )
        .unwrap();
        let cfg = load_workspace(&path).unwrap();
        assert_eq!(cfg.repos.len(), 2);
        assert_eq!(cfg.repos[0].id, "A");
        assert!(cfg.repos[0].config.is_none());
        assert_eq!(cfg.repos[1].config.as_deref(), Some(Path::new("alt.toml")));
    }

    #[test]
    fn preflight_passes_when_every_entry_is_well_formed() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("A");
        let b = tmp.path().join("B");
        write_repo(&a, true, true);
        write_repo(&b, true, true);
        let cfg = WorkspaceConfig {
            repos: vec![
                WorkspaceRepo {
                    id: "A".into(),
                    path: a,
                    config: None,
                },
                WorkspaceRepo {
                    id: "B".into(),
                    path: b,
                    config: None,
                },
            ],
        };
        preflight(&cfg).unwrap();
    }

    #[test]
    fn preflight_collects_every_failure_in_one_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("Good");
        write_repo(&good, true, true);
        let no_git = tmp.path().join("NoGit");
        write_repo(&no_git, false, true);
        let no_toml = tmp.path().join("NoToml");
        write_repo(&no_toml, true, false);
        let absent = tmp.path().join("DoesNotExist");

        let cfg = WorkspaceConfig {
            repos: vec![
                WorkspaceRepo {
                    id: "Good".into(),
                    path: good,
                    config: None,
                },
                WorkspaceRepo {
                    id: "Good".into(), // duplicate id
                    path: tmp.path().join("Good"),
                    config: None,
                },
                WorkspaceRepo {
                    id: "".into(), // empty id
                    path: tmp.path().join("Good"),
                    config: None,
                },
                WorkspaceRepo {
                    id: "Absent".into(),
                    path: absent,
                    config: None,
                },
                WorkspaceRepo {
                    id: "NoGit".into(),
                    path: no_git,
                    config: None,
                },
                WorkspaceRepo {
                    id: "NoToml".into(),
                    path: no_toml,
                    config: None,
                },
            ],
        };
        let err = preflight(&cfg).unwrap_err();
        let WorkspaceError::Preflight(lines) = err else {
            panic!("expected Preflight error");
        };
        // 5 failures: empty id, duplicate id, absent path, no .git, no cortex.toml.
        assert_eq!(
            lines.len(),
            5,
            "expected 5 preflight failures, got {lines:?}"
        );
        let blob = lines.join("\n");
        assert!(blob.contains("`id` is empty"));
        assert!(blob.contains("duplicate id `Good`"));
        assert!(blob.contains("does not exist"));
        assert!(blob.contains("not a git checkout"));
        assert!(blob.contains("cortex.toml not found"));
    }
}
