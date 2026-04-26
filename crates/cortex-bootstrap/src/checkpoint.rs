//! Checkpoint file — `.cortex-bootstrap.state.json` per spec 09
//! §Checkpoint file. Atomic write-rename every flush so a Ctrl-C
//! mid-run is always recoverable via `--resume`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Per-repo progress carried in the checkpoint file. Mirrors spec 09
/// §Checkpoint file fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoProgress {
    /// Files walked so far.
    #[serde(default)]
    pub files_walked: u64,
    /// Total files the walker plans to visit (set during the discovery
    /// pass so progress bars have a denominator).
    #[serde(default)]
    pub files_total: u64,
    /// Commits walked so far.
    #[serde(default)]
    pub commits_walked: u64,
    /// Total commits the git walker plans to visit.
    #[serde(default)]
    pub commits_total: u64,
    /// Total events emitted to Synap for this repo.
    #[serde(default)]
    pub events_emitted: u64,
    /// `pending` | `in_progress` | `done` | `failed`.
    #[serde(default = "default_status")]
    pub status: String,
    /// Last file the walker emitted from. `--resume` resumes after it.
    #[serde(default)]
    pub last_file: Option<String>,
    /// Last git ref the commit walker emitted from. `--resume` resumes
    /// after it.
    #[serde(default)]
    pub last_git_ref: Option<String>,
}

fn default_status() -> String {
    "pending".into()
}

/// Top-level checkpoint shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Schema version. Bump when the on-disk shape changes.
    #[serde(default = "default_version")]
    pub version: u32,
    /// RFC-3339 timestamp the run began.
    pub started_at: String,
    /// Per-repo progress, keyed by repo `id`.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoProgress>,
}

fn default_version() -> u32 {
    1
}

impl Checkpoint {
    /// Build a fresh checkpoint stamped with the current time.
    pub fn new(now_rfc3339: String) -> Self {
        Self {
            version: 1,
            started_at: now_rfc3339,
            repos: BTreeMap::new(),
        }
    }

    /// Convenience getter that creates a `pending` repo entry on
    /// first access.
    pub fn repo_mut(&mut self, repo_id: &str) -> &mut RepoProgress {
        self.repos.entry(repo_id.to_string()).or_default()
    }

    /// Whether the repo's checkpoint marks it `done`.
    pub fn is_repo_done(&self, repo_id: &str) -> bool {
        self.repos
            .get(repo_id)
            .map(|r| r.status == "done")
            .unwrap_or(false)
    }
}

/// Failure modes raised while reading or writing the checkpoint.
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Filesystem I/O failure.
    #[error("checkpoint io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialise / parse failure.
    #[error("checkpoint json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Read a checkpoint from `path`. Missing file returns a fresh
/// instance — `--resume` semantics expect "no checkpoint" to mean
/// "new run".
pub fn load_or_default(path: &Path) -> Result<Checkpoint, CheckpointError> {
    if !path.exists() {
        return Ok(Checkpoint::new(chrono::Utc::now().to_rfc3339()));
    }
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

/// Atomic write — first to `<path>.tmp`, then rename over the target.
/// Spec 09 §Checkpoint file: "Written atomically (write-rename)".
pub fn write_atomic(path: &Path, checkpoint: &Checkpoint) -> Result<(), CheckpointError> {
    let mut tmp = PathBuf::from(path);
    let mut file_name = tmp
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(".tmp");
    tmp.set_file_name(file_name);

    let body = serde_json::to_vec_pretty(checkpoint)?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".cortex-bootstrap.state.json");
        let mut cp = Checkpoint::new("2026-04-26T00:00:00Z".into());
        let r = cp.repo_mut("Vectorizer");
        r.files_walked = 100;
        r.events_emitted = 250;
        r.status = "in_progress".into();
        r.last_file = Some("src/lib.rs".into());
        write_atomic(&path, &cp).expect("write");
        let loaded = load_or_default(&path).expect("load");
        let v = loaded.repos.get("Vectorizer").expect("repo recorded");
        assert_eq!(v.files_walked, 100);
        assert_eq!(v.events_emitted, 250);
        assert_eq!(v.status, "in_progress");
        assert_eq!(v.last_file.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("absent.json");
        let cp = load_or_default(&path).expect("default");
        assert_eq!(cp.version, 1);
        assert!(cp.repos.is_empty());
    }

    #[test]
    fn is_repo_done_reads_status() {
        let mut cp = Checkpoint::new("now".into());
        cp.repo_mut("R").status = "done".into();
        assert!(cp.is_repo_done("R"));
        assert!(!cp.is_repo_done("Other"));
    }
}
