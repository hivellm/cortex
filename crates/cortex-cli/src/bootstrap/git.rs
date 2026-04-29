//! Git log walker — shells out to `git log` and parses one commit per
//! record. Avoids a `git2` dependency in favour of the same CLI every
//! operator already has installed.
//!
//! Spec 09 §Git log walker: `git log --all --diff-filter=AMD
//! --name-only --format='%H|%at|%ae|%s|%b'`. One event per commit;
//! merge commits emit a single squashed-style summary against their
//! nearest non-merge parent.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Parsed record of one git commit walked from the repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitRecord {
    /// 40-char SHA.
    pub sha: String,
    /// Author timestamp in seconds since epoch.
    pub author_ts: i64,
    /// Author email.
    pub author_email: String,
    /// Subject line.
    pub subject: String,
    /// Commit body (may be empty).
    pub body: String,
    /// Files changed (relative paths, forward-slash).
    pub files_changed: Vec<String>,
}

impl CommitRecord {
    /// Spec 09 §Synthetic event shape: `evidence.diff_summary`. The
    /// CLI's `git log --name-only` format does not carry per-file
    /// insertion counts, so we report a coarse summary keyed on file
    /// count. Phase-2 will pull `--shortstat` when richer numbers are
    /// needed.
    pub fn diff_summary(&self) -> String {
        format!("{} files changed", self.files_changed.len())
    }
}

/// Failure modes raised while walking the git log.
#[derive(Debug, thiserror::Error)]
pub enum GitWalkError {
    /// `.git` directory missing — the repo isn't a git checkout.
    #[error("repo `{0}` is not a git checkout (.git missing)")]
    NotAGitRepo(PathBuf),
    /// `git` binary missing or not on `PATH`.
    #[error("git binary not found: {0}")]
    GitBinary(#[source] std::io::Error),
    /// `git log` exited non-zero. Carries stderr verbatim.
    #[error("git log failed: {0}")]
    GitLogFailed(String),
}

/// Walk every commit in `repo_root` and return one [`CommitRecord`]
/// per commit, newest-first. `since` mirrors `git log --since` /
/// rev-spec semantics — pass `None` to walk all history.
/// Read the repo's current `HEAD` SHA via `git rev-parse HEAD`.
///
/// Phase4b §3 — the workspace orchestrator stamps this onto the
/// checkpoint's `last_git_ref` so a re-run can detect "this repo
/// is already up-to-date" and bypass the walker. Returns `None`
/// when the repo is detached or git fails (the caller treats both
/// as "no checkpoint match" and proceeds with the walk).
pub fn current_head_sha(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim();
    if sha.is_empty() || sha.len() < 7 {
        return None;
    }
    Some(sha.to_string())
}

/// Walk every commit reachable from `--all` in `repo_root`, optionally
/// constrained by a `since` revision range. Returns one
/// [`CommitRecord`] per commit in `git log` order (newest first).
pub fn walk_commits(repo_root: &Path, since: Option<&str>) -> Result<Vec<CommitRecord>, GitWalkError> {
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        return Err(GitWalkError::NotAGitRepo(repo_root.to_path_buf()));
    }
    // Two-record framing: `\x1e` between commits, `\x1f` between
    // header and body. Falls back gracefully on systems whose `git`
    // is configured to translate either character.
    const REC_SEP: char = '\x1e';
    const FIELD_SEP: char = '\x1f';
    let format = format!("{REC_SEP}%H{FIELD_SEP}%at{FIELD_SEP}%ae{FIELD_SEP}%s{FIELD_SEP}%b");

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_root)
        .arg("log")
        .arg("--all")
        .arg("--diff-filter=AMD")
        .arg("--name-only")
        .arg(format!("--format={format}"));
    if let Some(since) = since {
        cmd.arg(format!("--since={since}"));
    }

    let output = cmd.output().map_err(GitWalkError::GitBinary)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(GitWalkError::GitLogFailed(stderr));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_log(&stdout))
}

/// Parse the framed `git log` output produced by [`walk_commits`].
///
/// Exposed so unit tests can drive the parser without invoking `git`.
pub fn parse_log(stdout: &str) -> Vec<CommitRecord> {
    const REC_SEP: char = '\x1e';
    const FIELD_SEP: char = '\x1f';
    let mut commits = Vec::new();
    for raw in stdout.split(REC_SEP).skip(1) {
        // First five fields delimited by FIELD_SEP, then files on
        // their own lines until the next REC_SEP. Bodies may contain
        // newlines, so split on FIELD_SEP greedily and reattach.
        let mut parts = raw.splitn(5, FIELD_SEP);
        let sha = parts.next().unwrap_or_default().trim().to_string();
        if sha.is_empty() {
            continue;
        }
        let author_ts: i64 = parts
            .next()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let author_email = parts.next().unwrap_or_default().trim().to_string();
        let subject = parts.next().unwrap_or_default().trim().to_string();
        let rest = parts.next().unwrap_or_default();

        // `rest` is `{body}\n\n{file}\n{file}\n...`. The body ends at
        // the first blank line (since `--name-only` separates files
        // with a blank line from the body). When the body is empty
        // the very first line is already a file.
        let mut body_lines: Vec<&str> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        let mut in_body = !rest.starts_with('\n');
        for line in rest.split('\n') {
            if in_body {
                if line.trim().is_empty() && !body_lines.is_empty() {
                    in_body = false;
                    continue;
                }
                if line.trim().is_empty() && body_lines.is_empty() {
                    in_body = false;
                    continue;
                }
                body_lines.push(line);
            } else if !line.trim().is_empty() {
                files.push(line.replace('\\', "/"));
            }
        }
        let body = body_lines.join("\n").trim_end_matches('\n').to_string();
        commits.push(CommitRecord {
            sha,
            author_ts,
            author_email,
            subject,
            body,
            files_changed: files,
        });
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_commits() {
        // Construct the framed format manually — same shape `git log`
        // produces with the spec-09 `--format` argument.
        let raw = "\x1eabc123\x1f1700000000\x1fa@b.c\x1ffeat: refactor hnsw\x1fLong body line 1\nLong body line 2\n\nsrc/index/hnsw/mod.rs\nsrc/lib.rs\n\x1edef456\x1f1700100000\x1fc@d.e\x1ffix: ef_search\x1f\nsrc/index/hnsw/mod.rs\n";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "abc123");
        assert_eq!(commits[0].subject, "feat: refactor hnsw");
        assert_eq!(commits[0].author_email, "a@b.c");
        assert_eq!(commits[0].body, "Long body line 1\nLong body line 2");
        assert_eq!(
            commits[0].files_changed,
            vec![
                "src/index/hnsw/mod.rs".to_string(),
                "src/lib.rs".to_string(),
            ]
        );
        assert_eq!(commits[1].sha, "def456");
        assert_eq!(commits[1].body, "");
        assert_eq!(
            commits[1].files_changed,
            vec!["src/index/hnsw/mod.rs".to_string()]
        );
    }

    #[test]
    fn diff_summary_reports_file_count() {
        let c = CommitRecord {
            sha: "x".into(),
            author_ts: 0,
            author_email: String::new(),
            subject: String::new(),
            body: String::new(),
            files_changed: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(c.diff_summary(), "3 files changed");
    }

    #[test]
    fn missing_git_dir_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = walk_commits(tmp.path(), None).expect_err("must error");
        assert!(matches!(err, GitWalkError::NotAGitRepo(_)));
    }
}
