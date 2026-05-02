//! Per-repo runner — orchestrates walker → emitter → publisher with
//! checkpoint updates, redaction, and per-batch tracing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use cortex_storage::MetadataStore;
use sha2::{Digest, Sha256};

use super::checkpoint::Checkpoint;
use super::config::CortexSection;
use super::emitter::{
    emit_for_file_multi, emit_turn_historical, kind_passes_filter, BootstrapEvent, BOOTSTRAP_STREAM,
};
use super::git::walk_commits;
use super::metrics::Metrics;
use super::publisher::Publisher;
use super::walker::{walk_repo, FileClass, WalkEntry};

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
    /// phase10c — files whose redacted-body hash matched a prior
    /// `bootstrap_seen` ledger row. The walker computed events but
    /// suppressed publication so duplicates do not pile up in the
    /// downstream lane.
    pub files_suppressed: u64,
    /// `true` when the run completed without an error.
    pub completed: bool,
}

/// phase10c — opt-in dedup hook. The runner consults the ledger
/// before publishing each file's events; identical content_hash
/// since the last run is suppressed (only the `last_run_id` is
/// refreshed). Wrapped in an `Arc<Mutex<...>>` because
/// [`MetadataStore`]'s `rusqlite::Connection` is `Send` but not
/// `Sync`, and the runner may execute under a parallel
/// orchestrator (`run_repos_parallel`). Pass `None` to bypass
/// dedup entirely (existing call sites stay unchanged).
pub type DedupStore = Arc<Mutex<MetadataStore>>;

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
    /// Phase11e §5 — when non-empty, only publish events whose
    /// `kind` matches at least one of the listed family tokens
    /// (`decisions`, `turns`, `memory`, `analyses`, `laws`,
    /// `knowledge`, `learnings`, `code`, `docs`, `artifacts`). An
    /// empty Vec replays every kind (the legacy default). The
    /// filter applies after `emit_for_file_multi` and after the
    /// per-commit `emit_turn_historical` so the walker still does
    /// its full pass — only publication is gated.
    pub kind_filter: Vec<String>,
}

/// Drive one repo through the bootstrap pipeline.
///
/// Walks files, runs every accepted file through the emitter, walks
/// git commits, publishes each event, and updates the checkpoint as
/// it goes.
///
/// Calls [`run_repo_with_dedup`] with `dedup = None`. Existing
/// callers keep their signatures unchanged.
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
    run_repo_with_dedup(
        repo_root,
        runner_cfg,
        repo_cfg,
        publisher,
        metrics,
        checkpoint,
        last_file,
        last_git_ref,
        None,
    )
    .await
}

/// phase10c — same as [`run_repo`] but also consults a dedup
/// ledger. The walker computes the redacted-body
/// `content_hash = sha256(body)` for every accepted file. Before
/// publishing the file's events, it looks up `bootstrap_seen(repo,
/// path)`; if the stored hash matches, it suppresses publication
/// (counted as `files_suppressed`) and refreshes the ledger's
/// `last_run_id`. If the hash changed (or the row is absent), it
/// publishes as usual and upserts the ledger.
#[allow(clippy::too_many_arguments)]
pub async fn run_repo_with_dedup(
    repo_root: &Path,
    runner_cfg: &RunnerConfig,
    repo_cfg: &CortexSection,
    publisher: Arc<dyn Publisher>,
    metrics: Arc<Metrics>,
    checkpoint: &mut Checkpoint,
    last_file: Option<String>,
    last_git_ref: Option<String>,
    dedup: Option<DedupStore>,
) -> Result<RepoRunReport> {
    let started = Instant::now();
    // phase10d — canonical repo casing is **lowercase**. Walker
    // emission stamps the lowercase form so `scope.repo: "Cortex"`
    // and `scope.repo: "cortex"` resolve to the same rows. The
    // original-case directory name is kept as `repo_label` for
    // diagnostics + the dashboard's display column.
    let repo_label = runner_cfg.repo_id.clone();
    let repo_id = canonical_repo(&runner_cfg.repo_id);
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
    let mut files_suppressed: u64 = 0;
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
                // phase10c — file-level dedup. The audit caught the
                // walker re-emitting every file under fresh ULIDs on
                // every run (26 decisions for 2 ADRs on disk). We
                // hash the raw body bytes (deterministic across
                // runs because the file contents are the input to
                // every downstream emit + redact pass) and consult
                // the `bootstrap_seen` ledger. Identical hash → skip
                // publication and only refresh `last_run_id` so the
                // ledger reflects the most recent walk.
                let body_hash = sha256_of_body(&body);
                if let Some(store) = dedup.as_ref() {
                    if let Some(prev) = lookup_seen(store, &repo_id, rel_path) {
                        if prev.content_hash == body_hash {
                            files_suppressed += 1;
                            // Refresh the ledger so observers can
                            // tell which files were re-walked vs.
                            // truly stale.
                            upsert_seen(store, &repo_id, rel_path, &body_hash, Some(&session_id));
                            // Suppressed paths still extend the
                            // checkpoint cursor so a resume points
                            // past them on the next run.
                            let p = checkpoint.repo_mut(&repo_id);
                            p.last_file = Some(rel_path.clone());
                            continue;
                        }
                    }
                }
                // `emit_for_file_multi` returns a Vec — most file
                // classes still produce a single event, but spec docs
                // (`.rulebook/specs/**/*.md`) fan out into one
                // `law.imported` per `## ` section. The publish loop
                // handles N events per walked file so the
                // per-section laws all reach Synap.
                let mut events =
                    emit_for_file_multi(&repo_id, &session_id, None, entry, &body, &stream);
                // Phase11e §5 — apply the user-supplied kind filter
                // BEFORE the publish loop so the walker still does
                // its full pass (the dedup ledger needs the full
                // file walk to maintain `bootstrap_seen` accurately)
                // but only the requested kinds reach the wire.
                if !runner_cfg.kind_filter.is_empty() {
                    events.retain(|evt| kind_passes_filter(&runner_cfg.kind_filter, &evt.kind));
                }
                if events.is_empty() {
                    continue;
                }
                let mut any_published = false;
                for evt in &events {
                    metrics.incr_redactions(u64::from(evt.redactions));
                    publishes_attempted += 1;
                    if let Err(e) =
                        publish(&publisher, &stream, evt, &metrics, runner_cfg.dry_run).await
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
                    any_published = true;
                }
                if any_published {
                    let p = checkpoint.repo_mut(&repo_id);
                    p.events_emitted += events.len() as u64;
                    p.files_walked += 1;
                    p.last_file = Some(rel_path.clone());
                    // phase10c — record the (repo, path,
                    // content_hash) tuple so the next run can
                    // suppress duplicate publications for this
                    // file. Best-effort: a ledger write failure is
                    // logged but does NOT abort the run (the
                    // dedup is opportunistic; the publish already
                    // succeeded).
                    if let Some(store) = dedup.as_ref() {
                        upsert_seen(store, &repo_id, rel_path, &body_hash, Some(&session_id));
                    }
                }
            }
        }
    }

    // ---- git walk + emit ----
    let mut commits_walked: u64 = 0;
    if repo_cfg.git.include_commits {
        let since = runner_cfg
            .since
            .as_deref()
            .or(repo_cfg.git.since.as_deref());
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
                    // Phase11e §5 — apply the same kind filter to
                    // the per-commit `turn.historical` events.
                    if !runner_cfg.kind_filter.is_empty()
                        && !kind_passes_filter(&runner_cfg.kind_filter, &evt.kind)
                    {
                        continue;
                    }
                    metrics.incr_redactions(u64::from(evt.redactions));
                    publishes_attempted += 1;
                    if let Err(e) =
                        publish(&publisher, &stream, &evt, &metrics, runner_cfg.dry_run).await
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
        files_suppressed,
        completed: true,
    };

    tracing::info!(
        repo = %report.repo_id,
        repo_label = %repo_label,
        events = report.events_published,
        files_dropped = report.files_dropped,
        commits_walked = report.commits_walked,
        files_suppressed = report.files_suppressed,
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
pub async fn run_repos_parallel<F>(parallelism: usize, items: Vec<F>) -> Vec<Result<RepoRunReport>>
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

/// phase10c — pre-flight summary surfaced when the dedup ledger
/// is empty AND the live lane already carries far more rows than
/// the disk has files. Indicates that a previous (pre-phase10c)
/// run accumulated duplicates the user can clean up with
/// `cortex-ops bootstrap dedup`. Counts are caller-supplied so
/// this helper stays pure and trivially testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateLanePreflight {
    /// `true` when at least one class crossed the 2× threshold.
    pub likely_duplicates: bool,
    /// Per-class `(disk_count, lane_count)` pairs that exceeded
    /// the threshold. Empty when `likely_duplicates = false`.
    pub flagged: Vec<DuplicateClassFinding>,
}

/// One `(class, disk_count, lane_count)` tuple flagged by the
/// pre-flight check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateClassFinding {
    /// Symbolic class label (`decision` / `law` / `analysis`).
    pub class: &'static str,
    /// Number of files of this class on disk.
    pub disk_count: u64,
    /// Number of rows of this class in the live lane.
    pub lane_count: u64,
}

/// phase10c — pure pre-flight check. Returns
/// `likely_duplicates = true` when the dedup ledger is empty AND
/// at least one of the (decision / law / analysis) lane row
/// counts is > 2× the matching disk file count. The runner /
/// bootstrap bin pass the inputs via their already-on-hand
/// walkers + lane probes.
///
/// `ledger_empty` is the result of
/// [`MetadataStore::bootstrap_seen_count`] == 0; `disk` carries
/// the walker's per-class counts; `lane` carries the dashboard /
/// query lane's per-class counts. Pure — no side effects, no
/// I/O.
pub fn preflight_likely_duplicates(
    ledger_empty: bool,
    disk: &PerClassCounts,
    lane: &PerClassCounts,
) -> DuplicateLanePreflight {
    if !ledger_empty {
        return DuplicateLanePreflight {
            likely_duplicates: false,
            flagged: Vec::new(),
        };
    }
    let mut flagged = Vec::new();
    for (class, disk_count, lane_count) in [
        ("decision", disk.decision, lane.decision),
        ("law", disk.law, lane.law),
        ("analysis", disk.analysis, lane.analysis),
    ] {
        // Threshold: lane > 2 * disk AND disk > 0 (a 0-disk repo
        // is a misconfiguration, not a duplicate explosion). Use
        // `>` strictly so exactly-2× does not trip.
        if disk_count > 0 && lane_count > 2 * disk_count {
            flagged.push(DuplicateClassFinding {
                class,
                disk_count,
                lane_count,
            });
        }
    }
    DuplicateLanePreflight {
        likely_duplicates: !flagged.is_empty(),
        flagged,
    }
}

/// phase10d — canonical lowercase form for `repo` identifiers.
/// Every Cortex surface (walker emit, lane projection, scope
/// filter, dashboard wire shape) agrees on `to_ascii_lowercase`
/// so case mismatches between the bootstrap walker (which
/// historically used the on-disk directory casing — `Cortex`,
/// `Vectorizer`) and the orchestrator (which lowercased on
/// scope-resolve) stop dropping legitimate scoped queries on the
/// floor.
pub fn canonical_repo(repo: &str) -> String {
    repo.to_ascii_lowercase()
}

/// Per-class file / row counts used by
/// [`preflight_likely_duplicates`]. Defaults to all zeros so test
/// fixtures can populate one field at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerClassCounts {
    /// `:Decision` / `decision.imported` rows.
    pub decision: u64,
    /// `:Law` / `law.imported` rows.
    pub law: u64,
    /// `:Analysis` / `analysis.imported` rows.
    pub analysis: u64,
}

/// phase10c — `sha256:<hex>` over the raw file body. The walker
/// uses this as the dedup key. The body is the input to the
/// emitter's redaction + canonical-hash pipeline, so two runs that
/// see identical on-disk bytes produce identical downstream
/// `content_hash`es and identical ledger lookups.
fn sha256_of_body(body: &str) -> String {
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// phase10c — read the dedup ledger for `(repo, path)`. Logs and
/// returns `None` on any error so the runner falls back to the
/// publish path rather than aborting.
fn lookup_seen(
    store: &DedupStore,
    repo: &str,
    path: &str,
) -> Option<cortex_storage::BootstrapSeenRow> {
    let guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            tracing::warn!(repo, path, "bootstrap_seen lookup: lock poisoned");
            return None;
        }
    };
    match guard.bootstrap_seen_lookup(repo, path) {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(repo, path, error = %e, "bootstrap_seen lookup failed");
            None
        }
    }
}

/// phase10c — upsert `(repo, path, content_hash)` into the dedup
/// ledger with `last_run_id = run_id` and `last_emitted_at = now`.
/// Best-effort: errors are logged but do not propagate.
fn upsert_seen(
    store: &DedupStore,
    repo: &str,
    path: &str,
    content_hash: &str,
    run_id: Option<&str>,
) {
    let guard = match store.lock() {
        Ok(g) => g,
        Err(_) => {
            tracing::warn!(repo, path, "bootstrap_seen upsert: lock poisoned");
            return;
        }
    };
    if let Err(e) = guard.bootstrap_seen_upsert(repo, path, content_hash, run_id, Utc::now()) {
        tracing::warn!(repo, path, error = %e, "bootstrap_seen upsert failed");
    }
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
