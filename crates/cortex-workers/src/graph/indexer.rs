//! Phase23b — incremental graph indexer: diff resolver + change classifier.
//!
//! Three public surfaces:
//!
//! 1. [`git_diff`] — shells out to `git diff <from>..<to> --name-status` and
//!    returns a typed list of [`FileChange`] entries. When `from == to` the
//!    list is empty immediately (no subprocess).
//! 2. [`parse_name_status`] — pure parser for the `--name-status` line format;
//!    exposed so unit tests can drive it without a real git repo.
//! 3. [`classify_diff`] / [`ChangeTier`] — maps the change set onto one of four
//!    action tiers (NOOP → PARTIAL_UPDATE → ARCHITECTURE_UPDATE → FULL_UPDATE)
//!    using configurable thresholds from [`IndexerConfig`].

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

// ── File-change types ──────────────────────────────────────────────────────

/// Status of a single file in a `git diff --name-status` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// The file was newly created (A).
    Added,
    /// The file was modified in-place (M).
    Modified,
    /// The file was removed (D).
    Deleted,
    /// The file was moved; `old_path` is its previous location (R).
    Renamed {
        /// Path the file was at before the rename.
        old_path: String,
    },
}

/// One entry from a `git diff --name-status` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Current path (post-diff) of the file.
    pub path: String,
    /// What happened to the file.
    pub status: FileStatus,
}

impl FileChange {
    /// Returns `true` when this change adds a new file.
    pub fn is_addition(&self) -> bool {
        matches!(self.status, FileStatus::Added)
    }

    /// Returns `true` when this change removes a file.
    pub fn is_deletion(&self) -> bool {
        matches!(self.status, FileStatus::Deleted)
    }

    /// Returns `true` when this change moves a file.
    pub fn is_rename(&self) -> bool {
        matches!(self.status, FileStatus::Renamed { .. })
    }

    /// Returns the pre-rename path, or `None` for non-rename entries.
    pub fn old_path(&self) -> Option<&str> {
        match &self.status {
            FileStatus::Renamed { old_path } => Some(old_path),
            _ => None,
        }
    }
}

// ── git diff resolver ──────────────────────────────────────────────────────

/// Failure modes from [`git_diff`].
#[derive(Debug, thiserror::Error)]
pub enum GitDiffError {
    /// The `git` binary was not found on `PATH`.
    #[error("git binary not found: {0}")]
    GitBinary(#[source] std::io::Error),
    /// `git diff` exited non-zero.
    #[error("git diff failed (exit {code}): {stderr}")]
    GitFailed {
        /// Exit code.
        code: i32,
        /// Verbatim stderr.
        stderr: String,
    },
}

/// Run `git diff <from>..<to> --name-status` in `repo_root` and return a
/// typed list of changed files.
///
/// When `from == to` the function returns `Ok(Vec::new())` immediately
/// without spawning a subprocess (§2.2 equal-hash no-op).
pub fn git_diff(repo_root: &Path, from: &str, to: &str) -> Result<Vec<FileChange>, GitDiffError> {
    if from == to {
        return Ok(Vec::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["diff", "--name-status", &format!("{from}..{to}")])
        .output()
        .map_err(GitDiffError::GitBinary)?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(GitDiffError::GitFailed { code, stderr });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_name_status(&stdout))
}

/// Parse the tab-separated output of `git diff --name-status`.
///
/// Each non-empty line has the form:
/// ```text
/// A\tpath
/// M\tpath
/// D\tpath
/// R90\told_path\tnew_path
/// ```
/// Unrecognised status letters (T, U, B …) are silently skipped.
/// Exposed so unit tests can drive the parser without a real git process.
pub fn parse_name_status(output: &str) -> Vec<FileChange> {
    let mut changes = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let status_code = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let first_path = match parts.next() {
            Some(p) => p.to_string(),
            None => continue,
        };

        if status_code.starts_with('R') {
            let new_path = match parts.next() {
                Some(p) => p.to_string(),
                None => continue,
            };
            changes.push(FileChange {
                path: new_path,
                status: FileStatus::Renamed {
                    old_path: first_path,
                },
            });
        } else if status_code.starts_with('C') {
            let new_path = match parts.next() {
                Some(p) => p.to_string(),
                None => first_path,
            };
            changes.push(FileChange {
                path: new_path,
                status: FileStatus::Added,
            });
        } else {
            let status = match status_code.chars().next() {
                Some('A') => FileStatus::Added,
                Some('M') => FileStatus::Modified,
                Some('D') => FileStatus::Deleted,
                _ => continue,
            };
            changes.push(FileChange {
                path: first_path,
                status,
            });
        }
    }
    changes
}

// ── Change classifier ──────────────────────────────────────────────────────

/// Action tier assigned by [`classify_diff`].
///
/// Ordered from least to most disruptive; comparisons work as expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeTier {
    /// No structural changes. Fingerprint still advances; graph is untouched.
    Noop,
    /// Localized changes — re-extract and re-embed only the changed files.
    PartialUpdate,
    /// Directory structure changed or many files modified — re-run
    /// architecture-level analysis and invalidate topology summaries.
    ArchitectureUpdate,
    /// Massive or broad change — full re-index of the repo.
    FullUpdate,
}

impl ChangeTier {
    /// Whether this tier requires re-running architecture-level analysis.
    pub fn rerun_architecture(self) -> bool {
        self >= ChangeTier::ArchitectureUpdate
    }

    /// Whether this tier requires invalidating all cached synthesis.
    pub fn invalidate_all(self) -> bool {
        self == ChangeTier::FullUpdate
    }
}

/// Per-repo thresholds for the change classifier. Defaults match UA's values.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Structural file count at which the tier escalates to
    /// [`ChangeTier::ArchitectureUpdate`]. Default: 10 (UA default).
    pub arch_threshold_files: usize,
    /// Structural file count at which the tier escalates to
    /// [`ChangeTier::FullUpdate`] unconditionally. Default: 30 (UA default).
    pub full_threshold_files: usize,
    /// Fraction of total repo files (0.0–1.0) at which the tier escalates to
    /// [`ChangeTier::FullUpdate`]. Default: 0.50 (UA default).
    pub full_threshold_pct: f32,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            arch_threshold_files: 10,
            full_threshold_files: 30,
            full_threshold_pct: 0.50,
        }
    }
}

/// Returns `true` when the file is considered "structural" (source code or
/// config) rather than "cosmetic" (pure documentation).
///
/// Cosmetic extensions: `.md`, `.txt`, `.rst`, `.adoc`, `.asciidoc`.
/// Everything else — source, config, binaries, data — is structural so we err
/// toward triggering re-analysis rather than silently missing a change.
fn is_structural_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    !matches!(ext, "md" | "txt" | "rst" | "adoc" | "asciidoc")
}

/// Returns `true` when structural additions, deletions, or renames span more
/// than one distinct top-level directory — a proxy for "directory structure
/// changed" per the UA `dirsAddedOrRemoved` heuristic.
fn structural_dirs_changed(changes: &[FileChange]) -> bool {
    let dirs: HashSet<&str> = changes
        .iter()
        .filter(|c| {
            is_structural_file(&c.path)
                && matches!(
                    c.status,
                    FileStatus::Added | FileStatus::Deleted | FileStatus::Renamed { .. }
                )
        })
        .map(|c| c.path.split('/').next().unwrap_or(""))
        .collect();
    dirs.len() > 1
}

/// Classify a parsed diff into one of four [`ChangeTier`]s.
///
/// # Arguments
///
/// - `changes` — output of [`parse_name_status`] or [`git_diff`].
/// - `total_repo_files` — number of tracked files in the repo (used for the
///   percentage threshold). Pass `0` to disable the percentage check.
/// - `config` — per-repo thresholds; use [`IndexerConfig::default`] to get
///   UA-compatible values.
pub fn classify_diff(
    changes: &[FileChange],
    total_repo_files: usize,
    config: &IndexerConfig,
) -> ChangeTier {
    if changes.is_empty() {
        return ChangeTier::Noop;
    }

    let structural_count = changes
        .iter()
        .filter(|c| is_structural_file(&c.path))
        .count();

    if structural_count == 0 {
        return ChangeTier::Noop;
    }

    if structural_count >= config.full_threshold_files {
        return ChangeTier::FullUpdate;
    }
    if total_repo_files > 0 {
        let pct = structural_count as f32 / total_repo_files as f32;
        if pct >= config.full_threshold_pct {
            return ChangeTier::FullUpdate;
        }
    }

    if structural_count >= config.arch_threshold_files || structural_dirs_changed(changes) {
        return ChangeTier::ArchitectureUpdate;
    }

    ChangeTier::PartialUpdate
}

// ── git HEAD helper ────────────────────────────────────────────────────────

/// Return the current `HEAD` SHA for `repo_root` via `git rev-parse HEAD`.
/// Returns `None` when the repo is bare, detached, or git is unavailable.
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
    let sha = sha.trim().to_string();
    if sha.len() < 7 {
        return None;
    }
    Some(sha)
}

// ── Merge operations (§4.2–4.5) ───────────────────────────────────────────

/// Error from a merge/close operation.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    /// SQLite / storage operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] cortex_storage::MetadataError),
    /// Nexus Cypher operation failed.
    #[error("nexus error: {0}")]
    Nexus(#[from] crate::graph::nexus_client::GraphClientError),
}

/// What the merge produced: the caller uses this to schedule
/// re-extraction (§4.4) and re-embedding (§4.5).
#[derive(Debug)]
pub struct ReindexPlan {
    /// File paths that were structurally changed and must be re-extracted.
    pub changed_paths: Vec<String>,
    /// Graph node IDs that were bitemporal-closed by this merge. The caller
    /// should re-embed vectors for nodes with the same natural keys after
    /// re-extraction upserts them with fresh `valid_from`.
    pub closed_node_ids: Vec<String>,
    /// Rename pairs `(old_path, new_path)` that were rebind (identity preserved).
    pub rename_rebinds: Vec<(String, String)>,
    /// Classification tier this merge was triggered by.
    pub tier: ChangeTier,
}

/// Compute what the merge needs to do from the diff alone (no I/O).
/// The caller passes this to [`execute_merge`] which does the actual
/// index reads + Nexus writes.
pub fn plan_merge(diff: &[FileChange], config: &IndexerConfig, total_files: usize) -> ReindexPlan {
    let tier = classify_diff(diff, total_files, config);
    let mut plan = ReindexPlan {
        tier,
        changed_paths: Vec::new(),
        closed_node_ids: Vec::new(),
        rename_rebinds: Vec::new(),
    };

    for change in diff {
        match &change.status {
            FileStatus::Added | FileStatus::Modified => {
                plan.changed_paths.push(change.path.clone());
            }
            FileStatus::Deleted => {
                plan.changed_paths.push(change.path.clone());
            }
            FileStatus::Renamed { old_path } => {
                plan.rename_rebinds
                    .push((old_path.clone(), change.path.clone()));
                plan.changed_paths.push(change.path.clone());
            }
        }
    }

    plan
}

/// Apply one rename rebind to the file_node_index and issue a Cypher update
/// that changes the `file_path` property on all affected nodes in Nexus.
///
/// Called by [`execute_merge`] for each `R`-type entry in the diff.
pub async fn rebind_rename(
    store: &cortex_storage::MetadataStore,
    nexus: &crate::graph::nexus_client::LiveNexusClient,
    repo_id: &str,
    old_path: &str,
    new_path: &str,
) -> Result<usize, MergeError> {
    let nodes = store.file_node_index_get(repo_id, old_path)?;
    if nodes.is_empty() {
        return Ok(0);
    }

    let node_ids: Vec<String> = nodes.into_iter().map(|(id, _)| id).collect();
    for chunk in node_ids.chunks(64) {
        let ids_json = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".to_string());
        let cypher = format!("MATCH (n) WHERE n.id IN {ids_json} SET n.file_path = '{new_path}'");
        nexus.execute_with_retry(&cypher, None).await?;
    }

    let count = store.file_node_index_rename(repo_id, old_path, new_path)?;
    Ok(count)
}

/// Bitemporal-close all nodes associated with `file_path` in Nexus and
/// remove them from the file_node_index. Returns the list of closed node IDs.
///
/// "Close" = set `valid_to = valid_to_ts` and `lifecycle = 'superseded'`
/// on the node; no rows are deleted (history preserved per ADR-018).
/// Dangling edges (both endpoints closed) are pruned by the existing
/// [`crate::graph::stale_sweeper::StaleEdgeSweeper`] on its next cycle.
pub async fn close_nodes_for_file(
    store: &cortex_storage::MetadataStore,
    nexus: &crate::graph::nexus_client::LiveNexusClient,
    repo_id: &str,
    file_path: &str,
    valid_to_ts: &str,
) -> Result<Vec<String>, MergeError> {
    let nodes = store.file_node_index_get(repo_id, file_path)?;
    if nodes.is_empty() {
        return Ok(Vec::new());
    }

    let node_ids: Vec<String> = nodes.into_iter().map(|(id, _)| id).collect();
    for chunk in node_ids.chunks(64) {
        let ids_json = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".to_string());
        let cypher = format!(
            "MATCH (n) WHERE n.id IN {ids_json} \
             SET n.valid_to = '{valid_to_ts}', n.lifecycle = 'superseded'"
        );
        nexus.execute_with_retry(&cypher, None).await?;
    }

    store.file_node_index_remove_file(repo_id, file_path)?;
    Ok(node_ids)
}

/// Execute the full merge for a parsed diff: close changed files' nodes,
/// rebind renames, and return a [`ReindexPlan`] with paths/IDs that need
/// re-extraction (§4.4) and re-embedding (§4.5).
///
/// The caller is responsible for running the re-extraction pipeline on
/// `plan.changed_paths` and the re-embedding pipeline on `plan.closed_node_ids`
/// after upsert completes.
pub async fn execute_merge(
    store: &cortex_storage::MetadataStore,
    nexus: &crate::graph::nexus_client::LiveNexusClient,
    repo_id: &str,
    diff: &[FileChange],
    config: &IndexerConfig,
    total_files: usize,
    valid_to_ts: &str,
) -> Result<ReindexPlan, MergeError> {
    let mut plan = plan_merge(diff, config, total_files);

    for change in diff {
        match &change.status {
            FileStatus::Renamed { old_path } => {
                rebind_rename(store, nexus, repo_id, old_path, &change.path).await?;
            }
            FileStatus::Added => {
                // Added files have no prior nodes to close; extraction will
                // populate them fresh. No close needed.
            }
            FileStatus::Modified | FileStatus::Deleted => {
                let closed =
                    close_nodes_for_file(store, nexus, repo_id, &change.path, valid_to_ts).await?;
                plan.closed_node_ids.extend(closed);
            }
        }
    }

    Ok(plan)
}

// ── Scheduler gating + SessionStart trigger (§5) ──────────────────────────

/// Outcome of a [`check_and_reindex`] call.
#[derive(Debug)]
pub struct ReindexOutcome {
    /// Whether any graph work was triggered (false = fingerprint already current).
    pub did_reindex: bool,
    /// The HEAD SHA that was indexed (fingerprint advanced to this value).
    pub indexed_sha: String,
    /// The change tier determined for this run.
    pub tier: ChangeTier,
    /// Plan produced by the merge phase (empty when `did_reindex` is false).
    pub plan: Option<ReindexPlan>,
}

/// Build the JSON trigger messages to publish onto `cortex.consolidator.triggers`
/// based on the change tier (§5.1).
///
/// - `NOOP` / `PARTIAL_UPDATE` → no triggers (per-file changes do not
///   warrant re-synthesizing architecture-level summaries).
/// - `ARCHITECTURE_UPDATE` → one `nightly_topic` trigger for the repo
///   (re-clusters and re-synthesizes topic cards).
/// - `FULL_UPDATE` → same as `ARCHITECTURE_UPDATE` (the existing nightly
///   topic trigger is the correct escalation surface).
pub fn consolidation_triggers_for_reindex(
    repo_id: &str,
    tier: ChangeTier,
) -> Vec<serde_json::Value> {
    match tier {
        ChangeTier::Noop | ChangeTier::PartialUpdate => vec![],
        ChangeTier::ArchitectureUpdate | ChangeTier::FullUpdate => {
            vec![serde_json::json!({ "kind": "nightly_topic", "repo": repo_id })]
        }
    }
}

/// Staleness-check-and-reindex entry point (§5.2).
///
/// Compares `last_indexed_commit_hash` in storage against the current
/// `HEAD`. When equal, returns immediately with `did_reindex = false`
/// (idempotency gate — §5.3). When stale:
/// 1. Runs `git diff <last>..HEAD --name-status`.
/// 2. Classifies the diff.
/// 3. If tier is `Noop` (cosmetic-only), advances the fingerprint and returns.
/// 4. Otherwise calls [`execute_merge`] (bitemporal-close + rename rebind).
/// 5. Advances the fingerprint.
///
/// The caller is responsible for:
/// - Re-extracting `plan.changed_paths` through the existing analyzer.
/// - Re-embedding `plan.closed_node_ids` through the existing embedder.
/// - Publishing `consolidation_triggers_for_reindex(repo_id, plan.tier)` to
///   `cortex.consolidator.triggers`.
pub async fn check_and_reindex(
    store: &cortex_storage::MetadataStore,
    nexus: &crate::graph::nexus_client::LiveNexusClient,
    repo_root: &Path,
    repo_id: &str,
    config: &IndexerConfig,
    total_files: usize,
    now_rfc3339: &str,
) -> Result<ReindexOutcome, MergeError> {
    let head = match current_head_sha(repo_root) {
        Some(h) => h,
        None => {
            return Ok(ReindexOutcome {
                did_reindex: false,
                indexed_sha: String::new(),
                tier: ChangeTier::Noop,
                plan: None,
            });
        }
    };

    let last = store.get_indexed_commit_hash(repo_id)?;

    // §5.3 idempotency gate: fingerprint already current.
    if last.as_deref() == Some(head.as_str()) {
        return Ok(ReindexOutcome {
            did_reindex: false,
            indexed_sha: head,
            tier: ChangeTier::Noop,
            plan: None,
        });
    }

    let from = last.as_deref().unwrap_or("");
    let diff = match git_diff(repo_root, from, &head) {
        Ok(d) => d,
        Err(e) => {
            return Err(MergeError::Nexus(
                crate::graph::nexus_client::GraphClientError::Nexus(format!(
                    "git diff failed: {e}"
                )),
            ));
        }
    };

    let tier = classify_diff(&diff, total_files, config);

    // Cosmetic-only diff: advance fingerprint, no graph writes.
    if tier == ChangeTier::Noop {
        store.set_indexed_commit_hash(repo_id, &head, now_rfc3339)?;
        return Ok(ReindexOutcome {
            did_reindex: false,
            indexed_sha: head,
            tier: ChangeTier::Noop,
            plan: None,
        });
    }

    let plan = execute_merge(
        store,
        nexus,
        repo_id,
        &diff,
        config,
        total_files,
        now_rfc3339,
    )
    .await?;

    store.set_indexed_commit_hash(repo_id, &head, now_rfc3339)?;

    Ok(ReindexOutcome {
        did_reindex: true,
        indexed_sha: head,
        tier,
        plan: Some(plan),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_name_status ────────────────────────────────────────────

    #[test]
    fn parse_empty_output_returns_empty_vec() {
        assert!(parse_name_status("").is_empty());
        assert!(parse_name_status("\n\n").is_empty());
    }

    #[test]
    fn parse_added_modified_deleted() {
        let output = "A\tsrc/new.rs\nM\tsrc/lib.rs\nD\tsrc/old.rs\n";
        let changes = parse_name_status(output);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].path, "src/new.rs");
        assert!(changes[0].is_addition());
        assert_eq!(changes[1].path, "src/lib.rs");
        assert!(matches!(changes[1].status, FileStatus::Modified));
        assert_eq!(changes[2].path, "src/old.rs");
        assert!(changes[2].is_deletion());
    }

    #[test]
    fn parse_rename_extracts_old_and_new_path() {
        let output = "R90\told/path.rs\tnew/path.rs\n";
        let changes = parse_name_status(output);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_rename());
        assert_eq!(changes[0].path, "new/path.rs");
        assert_eq!(changes[0].old_path(), Some("old/path.rs"));
    }

    #[test]
    fn parse_copy_treated_as_added() {
        let output = "C100\tsrc/orig.rs\tsrc/copy.rs\n";
        let changes = parse_name_status(output);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/copy.rs");
        assert!(changes[0].is_addition());
    }

    #[test]
    fn parse_unknown_status_skipped() {
        let output = "T\tsrc/type-changed.rs\nM\tsrc/real.rs\n";
        let changes = parse_name_status(output);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/real.rs");
    }

    #[test]
    fn parse_mixed_diff_all_cases() {
        let output = "A\tsrc/a.rs\nM\tsrc/b.rs\nD\tsrc/c.rs\nR95\told.rs\tnew.rs\n";
        let changes = parse_name_status(output);
        assert_eq!(changes.len(), 4);
        assert!(changes[0].is_addition());
        assert!(matches!(changes[1].status, FileStatus::Modified));
        assert!(changes[2].is_deletion());
        assert!(changes[3].is_rename());
        assert_eq!(changes[3].old_path(), Some("old.rs"));
        assert_eq!(changes[3].path, "new.rs");
    }

    // ── git_diff equal-hash fast path ────────────────────────────────

    #[test]
    fn git_diff_same_hash_returns_empty_without_spawning() {
        let result = git_diff(Path::new("/does/not/exist"), "abc123", "abc123");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ── git_diff fixture-repo tests (§2.3) ───────────────────────────

    fn init_repo(dir: &std::path::Path) -> bool {
        let ok = |o: std::process::Output| o.status.success();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .map(ok)
                .unwrap_or(false)
        };
        git(&["init"])
            && git(&["config", "user.email", "test@example.com"])
            && git(&["config", "user.name", "Test User"])
    }

    fn commit_file(dir: &std::path::Path, path: &str, content: &str, msg: &str) -> Option<String> {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        std::fs::write(&full, content).ok()?;
        let stage = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["add", path])
            .output()
            .ok()?;
        if !stage.status.success() {
            return None;
        }
        let commit = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-m", msg])
            .output()
            .ok()?;
        if !commit.status.success() {
            return None;
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
    }

    fn delete_and_commit(dir: &std::path::Path, path: &str, msg: &str) -> Option<String> {
        let full = dir.join(path);
        std::fs::remove_file(&full).ok()?;
        let rm = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rm", path])
            .output()
            .ok()?;
        if !rm.status.success() {
            return None;
        }
        let commit = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-m", msg])
            .output()
            .ok()?;
        if !commit.status.success() {
            return None;
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
    }

    fn rename_and_commit(dir: &std::path::Path, old: &str, new: &str, msg: &str) -> Option<String> {
        let old_full = dir.join(old);
        let new_full = dir.join(new);
        std::fs::rename(&old_full, &new_full).ok()?;
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rm", old])
            .output();
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["add", new])
            .output();
        let commit = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-m", msg])
            .output()
            .ok()?;
        if !commit.status.success() {
            return None;
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
    }

    #[test]
    fn git_diff_detects_added_file() {
        let tmp = tempfile::tempdir().unwrap();
        if !init_repo(tmp.path()) {
            return;
        }
        let sha0 = commit_file(tmp.path(), "init.rs", "fn main() {}", "init");
        let sha1 = commit_file(tmp.path(), "new.rs", "pub fn f() {}", "add new.rs");
        let (Some(from), Some(to)) = (sha0, sha1) else {
            return;
        };
        let changes = git_diff(tmp.path(), &from, &to).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "new.rs");
        assert!(changes[0].is_addition());
    }

    #[test]
    fn git_diff_detects_modified_file() {
        let tmp = tempfile::tempdir().unwrap();
        if !init_repo(tmp.path()) {
            return;
        }
        let sha0 = commit_file(tmp.path(), "lib.rs", "fn a() {}", "init");
        let sha1 = commit_file(tmp.path(), "lib.rs", "fn a() { 1 }", "modify lib.rs");
        let (Some(from), Some(to)) = (sha0, sha1) else {
            return;
        };
        let changes = git_diff(tmp.path(), &from, &to).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].status, FileStatus::Modified));
    }

    #[test]
    fn git_diff_detects_deleted_file() {
        let tmp = tempfile::tempdir().unwrap();
        if !init_repo(tmp.path()) {
            return;
        }
        let sha0 = commit_file(tmp.path(), "gone.rs", "fn g() {}", "init");
        let sha1 = delete_and_commit(tmp.path(), "gone.rs", "delete gone.rs");
        let (Some(from), Some(to)) = (sha0, sha1) else {
            return;
        };
        let changes = git_diff(tmp.path(), &from, &to).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_deletion());
    }

    #[test]
    fn git_diff_detects_renamed_file() {
        let tmp = tempfile::tempdir().unwrap();
        if !init_repo(tmp.path()) {
            return;
        }
        let content = "pub fn alpha() -> u32 { 1 }\npub fn beta() -> u32 { 2 }\n".repeat(5);
        let sha0 = commit_file(tmp.path(), "old.rs", &content, "init");
        let sha1 = rename_and_commit(tmp.path(), "old.rs", "new.rs", "rename old to new");
        let (Some(from), Some(to)) = (sha0, sha1) else {
            return;
        };
        let changes = git_diff(tmp.path(), &from, &to).unwrap();
        assert!(!changes.is_empty());
        let has_new = changes.iter().any(|c| c.path == "new.rs");
        assert!(has_new, "new.rs must appear in diff");
    }

    // ── classify_diff ────────────────────────────────────────────────

    fn fc(path: &str, status: FileStatus) -> FileChange {
        FileChange {
            path: path.to_string(),
            status,
        }
    }

    #[test]
    fn classify_empty_diff_is_noop() {
        assert_eq!(
            classify_diff(&[], 100, &IndexerConfig::default()),
            ChangeTier::Noop
        );
    }

    #[test]
    fn classify_doc_only_changes_are_noop() {
        let changes = vec![
            fc("README.md", FileStatus::Modified),
            fc("CHANGELOG.md", FileStatus::Modified),
        ];
        assert_eq!(
            classify_diff(&changes, 100, &IndexerConfig::default()),
            ChangeTier::Noop
        );
    }

    #[test]
    fn classify_single_source_file_is_partial_update() {
        let changes = vec![fc("src/lib.rs", FileStatus::Modified)];
        assert_eq!(
            classify_diff(&changes, 100, &IndexerConfig::default()),
            ChangeTier::PartialUpdate
        );
    }

    #[test]
    fn classify_at_arch_threshold_escalates_to_architecture_update() {
        let config = IndexerConfig {
            arch_threshold_files: 3,
            full_threshold_files: 10,
            full_threshold_pct: 0.50,
        };
        let changes: Vec<FileChange> = (0..3)
            .map(|i| fc(&format!("src/f{i}.rs"), FileStatus::Modified))
            .collect();
        assert_eq!(
            classify_diff(&changes, 100, &config),
            ChangeTier::ArchitectureUpdate
        );
    }

    #[test]
    fn classify_below_arch_threshold_is_partial() {
        let config = IndexerConfig {
            arch_threshold_files: 3,
            full_threshold_files: 10,
            full_threshold_pct: 0.50,
        };
        let changes: Vec<FileChange> = (0..2)
            .map(|i| fc(&format!("src/f{i}.rs"), FileStatus::Modified))
            .collect();
        assert_eq!(
            classify_diff(&changes, 100, &config),
            ChangeTier::PartialUpdate
        );
    }

    #[test]
    fn classify_at_full_threshold_files_is_full_update() {
        let config = IndexerConfig {
            arch_threshold_files: 5,
            full_threshold_files: 10,
            full_threshold_pct: 0.90,
        };
        let changes: Vec<FileChange> = (0..10)
            .map(|i| fc(&format!("src/f{i}.rs"), FileStatus::Added))
            .collect();
        assert_eq!(
            classify_diff(&changes, 100, &config),
            ChangeTier::FullUpdate
        );
    }

    #[test]
    fn classify_pct_threshold_triggers_full_update() {
        let config = IndexerConfig {
            arch_threshold_files: 50,
            full_threshold_files: 100,
            full_threshold_pct: 0.50,
        };
        let changes: Vec<FileChange> = (0..6)
            .map(|i| fc(&format!("src/f{i}.rs"), FileStatus::Modified))
            .collect();
        assert_eq!(classify_diff(&changes, 10, &config), ChangeTier::FullUpdate);
    }

    #[test]
    fn classify_multi_dir_structural_change_is_architecture_update() {
        let changes = vec![
            fc("src/lib.rs", FileStatus::Added),
            fc("tests/integration.rs", FileStatus::Added),
        ];
        let config = IndexerConfig {
            arch_threshold_files: 100,
            full_threshold_files: 200,
            full_threshold_pct: 0.99,
        };
        assert_eq!(
            classify_diff(&changes, 1000, &config),
            ChangeTier::ArchitectureUpdate
        );
    }

    #[test]
    fn classify_single_dir_additions_below_threshold_is_partial() {
        let changes = vec![
            fc("src/a.rs", FileStatus::Added),
            fc("src/b.rs", FileStatus::Added),
        ];
        let config = IndexerConfig {
            arch_threshold_files: 10,
            full_threshold_files: 30,
            full_threshold_pct: 0.90,
        };
        assert_eq!(
            classify_diff(&changes, 1000, &config),
            ChangeTier::PartialUpdate
        );
    }

    #[test]
    fn change_tier_ordering_is_correct() {
        assert!(ChangeTier::Noop < ChangeTier::PartialUpdate);
        assert!(ChangeTier::PartialUpdate < ChangeTier::ArchitectureUpdate);
        assert!(ChangeTier::ArchitectureUpdate < ChangeTier::FullUpdate);
    }

    #[test]
    fn change_tier_rerun_architecture_fires_at_and_above_arch() {
        assert!(!ChangeTier::Noop.rerun_architecture());
        assert!(!ChangeTier::PartialUpdate.rerun_architecture());
        assert!(ChangeTier::ArchitectureUpdate.rerun_architecture());
        assert!(ChangeTier::FullUpdate.rerun_architecture());
    }

    #[test]
    fn change_tier_invalidate_all_only_fires_on_full() {
        assert!(!ChangeTier::Noop.invalidate_all());
        assert!(!ChangeTier::PartialUpdate.invalidate_all());
        assert!(!ChangeTier::ArchitectureUpdate.invalidate_all());
        assert!(ChangeTier::FullUpdate.invalidate_all());
    }

    #[test]
    fn classify_zero_total_files_disables_pct_check() {
        let config = IndexerConfig {
            arch_threshold_files: 50,
            full_threshold_files: 100,
            full_threshold_pct: 0.50,
        };
        let changes: Vec<FileChange> = (0..3)
            .map(|i| fc(&format!("src/f{i}.rs"), FileStatus::Added))
            .collect();
        assert_eq!(
            classify_diff(&changes, 0, &config),
            ChangeTier::PartialUpdate
        );
    }

    // ── plan_merge ───────────────────────────────────────────────────

    #[test]
    fn plan_merge_added_file_lands_in_changed_paths() {
        let diff = vec![fc("src/new.rs", FileStatus::Added)];
        let plan = plan_merge(&diff, &IndexerConfig::default(), 100);
        assert_eq!(plan.changed_paths, vec!["src/new.rs"]);
        assert!(plan.closed_node_ids.is_empty());
        assert!(plan.rename_rebinds.is_empty());
        assert_eq!(plan.tier, ChangeTier::PartialUpdate);
    }

    #[test]
    fn plan_merge_deleted_file_lands_in_changed_paths() {
        let diff = vec![fc("src/gone.rs", FileStatus::Deleted)];
        let plan = plan_merge(&diff, &IndexerConfig::default(), 100);
        assert!(plan.changed_paths.contains(&"src/gone.rs".to_string()));
        assert!(plan.rename_rebinds.is_empty());
    }

    #[test]
    fn plan_merge_rename_lands_in_rename_rebinds_and_changed_paths() {
        let diff = vec![FileChange {
            path: "new/path.rs".to_string(),
            status: FileStatus::Renamed {
                old_path: "old/path.rs".to_string(),
            },
        }];
        let plan = plan_merge(&diff, &IndexerConfig::default(), 100);
        assert_eq!(
            plan.rename_rebinds,
            vec![("old/path.rs".to_string(), "new/path.rs".to_string())]
        );
        assert!(plan.changed_paths.contains(&"new/path.rs".to_string()));
    }

    #[test]
    fn plan_merge_noop_diff_returns_noop_tier() {
        let plan = plan_merge(&[], &IndexerConfig::default(), 100);
        assert_eq!(plan.tier, ChangeTier::Noop);
        assert!(plan.changed_paths.is_empty());
    }

    #[test]
    fn plan_merge_doc_only_diff_returns_noop_tier() {
        let diff = vec![fc("CHANGELOG.md", FileStatus::Modified)];
        let plan = plan_merge(&diff, &IndexerConfig::default(), 100);
        assert_eq!(plan.tier, ChangeTier::Noop);
        assert!(plan.changed_paths.contains(&"CHANGELOG.md".to_string()));
    }

    // ── consolidation_triggers_for_reindex ───────────────────────────

    #[test]
    fn triggers_noop_returns_empty() {
        assert!(consolidation_triggers_for_reindex("cortex", ChangeTier::Noop).is_empty());
    }

    #[test]
    fn triggers_partial_returns_empty() {
        assert!(consolidation_triggers_for_reindex("cortex", ChangeTier::PartialUpdate).is_empty());
    }

    #[test]
    fn triggers_architecture_returns_nightly_topic() {
        let triggers = consolidation_triggers_for_reindex("cortex", ChangeTier::ArchitectureUpdate);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["kind"], "nightly_topic");
        assert_eq!(triggers[0]["repo"], "cortex");
    }

    #[test]
    fn triggers_full_returns_nightly_topic() {
        let triggers = consolidation_triggers_for_reindex("my-repo", ChangeTier::FullUpdate);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["kind"], "nightly_topic");
        assert_eq!(triggers[0]["repo"], "my-repo");
    }

    // ── current_head_sha ─────────────────────────────────────────────

    #[test]
    fn current_head_sha_returns_none_for_non_repo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(current_head_sha(tmp.path()).is_none());
    }
}
