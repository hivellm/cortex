//! Per-repo runner — orchestrates walker → emitter → publisher with
//! checkpoint updates, redaction, and per-batch tracing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;

use crate::checkpoint::Checkpoint;
use crate::config::CortexSection;
use crate::emitter::{
    emit_for_file, emit_turn_historical, BootstrapEvent, BOOTSTRAP_STREAM,
};
use crate::git::walk_commits;
use crate::metrics::Metrics;
use crate::publisher::Publisher;
use crate::walker::{walk_repo, FileClass, WalkEntry};

/// One run's outcome for a single repo.
#[derive(Debug, Clone, Default)]
pub struct RepoRunReport {
    /// Repo identifier.
    pub repo_id: String,
    /// Total events published.
    pub events_published: u64,
    /// Files dropped by the walker (any reason).
    pub files_dropped: u64,
    /// Commits walked through `git log`.
    pub commits_walked: u64,
    /// Wall-clock duration in seconds.
    pub duration_secs: f64,
    /// `true` when the run completed without an error.
    pub completed: bool,
}

/// Configuration block carried through the runner — a strict subset
/// of the CLI / `cortex.toml` shape, narrowed to what the runner
/// itself needs.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Repo identifier (override or directory name).
    pub repo_id: String,
    /// Stream the publisher targets.
    pub stream: String,
    /// `--since` override applied to the git walker.
    pub since: Option<String>,
    /// `--dry-run` — when `true` the runner walks + emits but never
    /// publishes (used by `--estimate` callers as well).
    pub dry_run: bool,
}

/// Drive one repo through the bootstrap pipeline.
///
/// Walks files, runs every accepted file through the emitter, walks
/// git commits, publishes each event, and updates the checkpoint as
/// it goes.
#[allow(clippy::too_many_arguments)]
pub async fn run_repo(
    repo_root: &Path,
    runner_cfg: &RunnerConfig,
    repo_cfg: &CortexSection,
    publisher: Arc<dyn Publisher>,
    metrics: Arc<Metrics>,
    checkpoint: &mut Checkpoint,
    last_file: Option<String>,
    last_git_ref: Option<String>,
) -> Result<RepoRunReport> {
    let started = Instant::now();
    let repo_id = runner_cfg.repo_id.clone();
    let stream = if runner_cfg.stream.is_empty() {
        BOOTSTRAP_STREAM.to_string()
    } else {
        runner_cfg.stream.clone()
    };
    // One Session node per repo run. Every artifact / turn / etc.
    // emitted by this invocation gets stamped with the same id so
    // the graph writer collapses them under a single Session
    // instead of one synthetic session per event.
    let session_id = ulid::Ulid::new().to_string();

    {
        let p = checkpoint.repo_mut(&repo_id);
        p.status = "in_progress".into();
    }

    // ---- file walk + emit ----
    let entries = walk_repo(repo_root, repo_cfg);
    let mut files_dropped: u64 = 0;
    let mut events_published: u64 = 0;
    let mut resume_filter_active = last_file.is_some();
    // Per-repo error tolerance. A single oversized JSON or a
    // transient Synap blip used to abort the entire repo (and
    // every repo after it on the CLI line). The bootstrap now
    // tolerates up to 5% publish failures before re-arming the
    // hard error path; below that ratio the error is logged +
    // counted and the walk continues. The 20-event minimum keeps
    // the ratio from tripping on the very first publish failure
    // of a small repo.
    let mut publishes_attempted: u64 = 0;
    let mut publishes_failed: u64 = 0;
    const PUBLISH_FAILURE_RATIO_LIMIT: f64 = 0.05;
    const PUBLISH_FAILURE_FLOOR: u64 = 20;
    let abort_on_failure = |attempted: u64, failed: u64| -> bool {
        if attempted < PUBLISH_FAILURE_FLOOR {
            return false;
        }
        (failed as f64) / (attempted as f64) > PUBLISH_FAILURE_RATIO_LIMIT
    };
    for entry in &entries {
        match entry {
            WalkEntry::Dropped { reason, rel_path } => {
                files_dropped += 1;
                metrics.incr_files_dropped(&repo_id, reason);
                let _ = rel_path; // already in trace via metric label
            }
            WalkEntry::Accepted {
                rel_path,
                size_bytes,
                ..
            } => {
                if resume_filter_active {
                    if let Some(ref last) = last_file {
                        if rel_path <= last {
                            // Already processed in a previous run; skip.
                            continue;
                        } else {
                            resume_filter_active = false;
                        }
                    }
                }
                metrics.incr_files_walked(&repo_id);
                metrics.incr_bytes_processed(&repo_id, *size_bytes);
                let body = match read_body(&entry_path(entry)) {
                    Ok(b) => b,
                    Err(e) => {
                        metrics.incr_errors(&repo_id, "read_file");
                        tracing::warn!(error = %e, path = %rel_path, "failed to read file");
                        continue;
                    }
                };
                let evt = match emit_for_file(&repo_id, &session_id, None, entry, &body, &stream)
                {
                    Some(e) => e,
                    None => continue,
                };
                metrics.incr_redactions(u64::from(evt.redactions));
                publishes_attempted += 1;
                if let Err(e) = publish(&publisher, &stream, &evt, &metrics, runner_cfg.dry_run)
                    .await
                {
                    metrics.incr_errors(&repo_id, "publish");
                    publishes_failed += 1;
                    tracing::warn!(error = %e, path = %rel_path, repo = %repo_id, "publish skipped");
                    if abort_on_failure(publishes_attempted, publishes_failed) {
                        return Err(anyhow::anyhow!(
                            "publish failure ratio exceeded for {repo_id}: {publishes_failed}/{publishes_attempted} (last: {rel_path}: {e})"
                        ));
                    }
                    continue;
                }
                events_published += 1;
                metrics.incr_events_emitted(&repo_id, &evt.kind);
                let p = checkpoint.repo_mut(&repo_id);
                p.events_emitted += 1;
                p.files_walked += 1;
                p.last_file = Some(rel_path.clone());
            }
        }
    }

    // ---- git walk + emit ----
    let mut commits_walked: u64 = 0;
    if repo_cfg.git.include_commits {
        let since = runner_cfg.since.as_deref().or(repo_cfg.git.since.as_deref());
        match walk_commits(repo_root, since) {
            Ok(commits) => {
                let mut resume_git = last_git_ref.is_some();
                for c in &commits {
                    if resume_git {
                        if let Some(ref last) = last_git_ref {
                            if c.sha == *last {
                                resume_git = false;
                                continue;
                            } else {
                                continue;
                            }
                        }
                    }
                    commits_walked += 1;
                    metrics.incr_commits_walked(&repo_id);
                    let evt = emit_turn_historical(&repo_id, &session_id, c, &stream);
                    metrics.incr_redactions(u64::from(evt.redactions));
                    publishes_attempted += 1;
                    if let Err(e) = publish(&publisher, &stream, &evt, &metrics, runner_cfg.dry_run)
                        .await
                    {
                        metrics.incr_errors(&repo_id, "publish");
                        publishes_failed += 1;
                        tracing::warn!(
                            error = %e,
                            commit = %c.sha,
                            repo = %repo_id,
                            "publish skipped"
                        );
                        if abort_on_failure(publishes_attempted, publishes_failed) {
                            return Err(anyhow::anyhow!(
                                "publish failure ratio exceeded for {repo_id}: {publishes_failed}/{publishes_attempted} (last commit: {}: {e})",
                                c.sha
                            ));
                        }
                        continue;
                    }
                    events_published += 1;
                    metrics.incr_events_emitted(&repo_id, &evt.kind);
                    let p = checkpoint.repo_mut(&repo_id);
                    p.events_emitted += 1;
                    p.commits_walked += 1;
                    p.last_git_ref = Some(c.sha.clone());
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, repo = %repo_id, "git walk skipped");
                metrics.incr_errors(&repo_id, "git_walk");
            }
        }
    }

    let duration_secs = started.elapsed().as_secs_f64();
    metrics.observe_repo_duration(&repo_id, duration_secs);
    {
        let p = checkpoint.repo_mut(&repo_id);
        p.status = "done".into();
    }

    let report = RepoRunReport {
        repo_id: repo_id.clone(),
        events_published,
        files_dropped,
        commits_walked,
        duration_secs,
        completed: true,
    };

    tracing::info!(
        repo = %report.repo_id,
        events = report.events_published,
        files_dropped = report.files_dropped,
        commits_walked = report.commits_walked,
        duration_s = report.duration_secs,
        outcome = "ok",
        "bootstrap repo complete"
    );

    Ok(report)
}

fn entry_path(entry: &WalkEntry) -> PathBuf {
    match entry {
        WalkEntry::Accepted { path, .. } => path.clone(),
        WalkEntry::Dropped { rel_path, .. } => PathBuf::from(rel_path),
    }
}

fn read_body(path: &Path) -> Result<String> {
    // `read_to_string` rejects non-UTF-8; we lossy-decode so binaries
    // that slipped past the extension filter still produce something
    // searchable rather than fail the whole repo.
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn publish(
    publisher: &Arc<dyn Publisher>,
    stream: &str,
    event: &BootstrapEvent,
    metrics: &Arc<Metrics>,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let started = Instant::now();
    publisher.publish_one(stream, event).await?;
    let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    metrics.observe_publish_latency(latency_ms);
    Ok(())
}

/// Helper: run multiple repos with bounded concurrency. Maps to
/// `--parallelism N`.
pub async fn run_repos_parallel<F>(
    parallelism: usize,
    items: Vec<F>,
) -> Vec<Result<RepoRunReport>>
where
    F: std::future::Future<Output = Result<RepoRunReport>> + Send + 'static,
{
    let n = parallelism.max(1);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(n));
    let mut handles = Vec::with_capacity(items.len());
    for fut in items {
        let permit_owner = semaphore.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit_owner.acquire_owned().await.ok();
            fut.await
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(r) => out.push(r),
            Err(e) => out.push(Err(anyhow::anyhow!("join: {e}"))),
        }
    }
    out
}

/// Count walked files per `FileClass`. Pure helper used by tests.
#[doc(hidden)]
pub fn count_classes(entries: &[WalkEntry]) -> std::collections::HashMap<FileClass, u64> {
    use std::collections::HashMap;
    let mut out: HashMap<FileClass, u64> = HashMap::new();
    for e in entries {
        if let WalkEntry::Accepted { class, .. } = e {
            *out.entry(*class).or_insert(0) += 1;
        }
    }
    out
}
