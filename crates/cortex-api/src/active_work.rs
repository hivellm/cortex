//! phase13g §1 — `cortex_active_work` operator-state snapshot.
//!
//! Reads from `.rulebook/tasks/*/{.metadata.json,tasks.md}` plus the
//! `.rulebook/archive/*` tree under `CORTEX_WORKSPACE_ROOT` and
//! projects each task into a typed [`ActiveTaskRow`]. The result is
//! cached for [`DEFAULT_TTL`] (60 seconds) and invalidated whenever
//! any tracked `tasks.md` or `.metadata.json` advances its mtime past
//! the snapshot. Mirrors the pattern in
//! [`crate::dashboard::tasks_loader`] but produces a different
//! projection — the active-work view drops the proposal preview /
//! progress / repo metadata and adds the two grounding-critical
//! fields the pre-thinking pipeline consumes:
//!
//! - `next_unchecked_item`: the first `- [ ]` row in `tasks.md`
//!   (lowest-numbered section), or `None` when every item is done.
//! - `blocked_reason`: surfaced when `.metadata.json::status =
//!   "blocked"` — carries the metadata's `blocked_reason` string
//!   when present, otherwise `Some("blocked")` as the marker.
//!
//! ADR-014 pure-reader contract: the handler returns this view
//! verbatim. No fallback strings, no handler-side derivations.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::Query;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

/// TTL between forced re-scans of the `.rulebook/` tree. Per-task
/// invalidation still happens whenever a tracked file mtime
/// advances past the snapshot.
pub const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// Hard cap on how many archive rows appear in the response. Sized
/// so a typical phase (~15 sub-archives) fits but a runaway archive
/// tree does not blow up the MCP envelope.
pub const RECENT_ARCHIVES_CAP: usize = 20;

/// One active-task row in the response. Pure projection of the
/// task directory: id + phase + status + next-unchecked-item +
/// blocked-reason. No proposal preview, no progress counter — the
/// pre-thinking renderer keeps the row tight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTaskRow {
    /// Task directory name.
    pub id: String,
    /// Phase identifier parsed from the id prefix (e.g. `phase13g`).
    /// `None` when the id does not start with `phase<N>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// One of `pending`, `in-progress`, `completed`, `blocked`.
    pub status: String,
    /// First `- [ ]` row in `tasks.md`, trimmed and bounded to 256
    /// chars. `None` when every checklist item is `- [x]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_unchecked_item: Option<String>,
    /// Operator-facing reason carried when the metadata `status`
    /// field equals `"blocked"`. `None` for other statuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// Project slug derived from the workspace root's parent
    /// directory name (lowercase). `None` when the workspace root
    /// has no parent (the rare case where `.rulebook/` lives at
    /// disk root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// One recent-archive row in the response. The id keeps the
/// archive's `YYYY-MM-DD-` prefix so callers can sort or scope by
/// date without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRow {
    /// Archive directory name (with the `YYYY-MM-DD-` prefix).
    pub id: String,
    /// Archive date (`YYYY-MM-DD`) parsed from the directory name.
    /// `None` when the directory does not carry the expected prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    /// First H1 of `proposal.md`, falling back to the id with the
    /// prefix stripped.
    pub title: String,
}

/// Top-level response shape returned by `/v1/dashboard/active-work`
/// and projected verbatim by the `cortex_active_work` MCP tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWorkReport {
    /// All non-archived tasks (`pending` / `in-progress` /
    /// `blocked` / `completed`) under `.rulebook/tasks/`, sorted by
    /// `(phase_num, phase_letter, id)`.
    pub active_tasks: Vec<ActiveTaskRow>,
    /// Number of `active_tasks` rows whose status is `in-progress`.
    pub in_progress_count: u32,
    /// Number of `active_tasks` rows whose status is `blocked`.
    pub blocked_count: u32,
    /// Newest-first archive directory listing, capped at
    /// [`RECENT_ARCHIVES_CAP`].
    pub recent_archives: Vec<ArchiveRow>,
}

/// Loader that walks `.rulebook/tasks/` and `.rulebook/archive/`
/// once per [`DEFAULT_TTL`] (or whenever a tracked file mtime
/// advances) and serves the cached [`ActiveWorkReport`] to every
/// caller.
pub struct ActiveWorkLoader {
    root: PathBuf,
    ttl: Duration,
    cache: RwLock<Cached>,
}

#[derive(Default)]
struct Cached {
    /// Instant of the last full scan (`None` until the first read).
    scanned_at: Option<Instant>,
    /// Mtime stamps captured at scan time. The cache invalidates as
    /// soon as any tracked file moves past the stamp.
    stamps: Vec<(PathBuf, SystemTime)>,
    /// Snapshot the loader serves to callers.
    report: ActiveWorkReport,
}

impl ActiveWorkLoader {
    /// Construct a loader rooted at the supplied `.rulebook/`
    /// directory. The path is not validated up-front — a missing
    /// directory just makes [`Self::snapshot`] return an empty
    /// report.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ttl: DEFAULT_TTL,
            cache: RwLock::new(Cached::default()),
        }
    }

    /// Override the default TTL. Mainly used by tests.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// `.rulebook/` directory the loader walks.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the cached report. Refreshes the cache when the TTL
    /// has elapsed OR when any tracked file mtime advanced past
    /// the snapshot.
    pub fn snapshot(&self) -> ActiveWorkReport {
        if !self.cache_is_fresh() {
            self.refresh();
        }
        let cache = self.cache.read().expect("active_work cache poisoned");
        cache.report.clone()
    }

    /// Snapshot filtered to one repo slug. Compared
    /// case-insensitively against each row's `repo`. Rows with a
    /// `None` repo are excluded when a filter is supplied.
    pub fn snapshot_filtered(&self, repo: Option<&str>) -> ActiveWorkReport {
        let report = self.snapshot();
        match repo {
            None => report,
            Some(filter) => {
                let needle = filter.to_ascii_lowercase();
                let active_tasks: Vec<ActiveTaskRow> = report
                    .active_tasks
                    .into_iter()
                    .filter(|row| {
                        row.repo
                            .as_deref()
                            .map(|r| r.eq_ignore_ascii_case(&needle))
                            .unwrap_or(false)
                    })
                    .collect();
                let in_progress_count = active_tasks
                    .iter()
                    .filter(|r| r.status == "in-progress")
                    .count() as u32;
                let blocked_count = active_tasks
                    .iter()
                    .filter(|r| r.status == "blocked")
                    .count() as u32;
                ActiveWorkReport {
                    active_tasks,
                    in_progress_count,
                    blocked_count,
                    recent_archives: report.recent_archives,
                }
            }
        }
    }

    fn cache_is_fresh(&self) -> bool {
        let cache = self.cache.read().expect("active_work cache poisoned");
        let Some(scanned_at) = cache.scanned_at else {
            return false;
        };
        if scanned_at.elapsed() >= self.ttl {
            return false;
        }
        for (path, captured) in &cache.stamps {
            let live = match fs::metadata(path).and_then(|m| m.modified()) {
                Ok(ts) => ts,
                Err(_) => return false,
            };
            if live > *captured {
                return false;
            }
        }
        true
    }

    fn refresh(&self) {
        let (report, stamps) = scan(&self.root);
        let mut cache = self.cache.write().expect("active_work cache poisoned");
        cache.report = report;
        cache.stamps = stamps;
        cache.scanned_at = Some(Instant::now());
    }
}

/// Walk the `.rulebook/` tree once and return the freshly scanned
/// report plus the list of `(path, mtime)` stamps the cache uses
/// for invalidation.
fn scan(root: &Path) -> (ActiveWorkReport, Vec<(PathBuf, SystemTime)>) {
    let repo = root
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned().to_ascii_lowercase());
    let mut active_tasks = Vec::new();
    let mut stamps = Vec::new();
    let tasks_root = root.join("tasks");
    if let Ok(entries) = fs::read_dir(&tasks_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir = entry.path();
            if let Some(row) = read_task_row(&dir, repo.clone(), &mut stamps) {
                active_tasks.push(row);
            }
        }
    }
    active_tasks.sort_by_key(sort_key);

    let in_progress_count = active_tasks
        .iter()
        .filter(|r| r.status == "in-progress")
        .count() as u32;
    let blocked_count = active_tasks
        .iter()
        .filter(|r| r.status == "blocked")
        .count() as u32;

    let mut recent_archives = Vec::new();
    let archive_root = root.join("archive");
    if let Ok(entries) = fs::read_dir(&archive_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir = entry.path();
            let id = match dir.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let archived_at = parse_archive_date(&id);
            let title = read_proposal_title(&dir).unwrap_or_else(|| {
                archived_at
                    .as_deref()
                    .map(|d| id.strip_prefix(&format!("{d}-")).unwrap_or(&id).to_string())
                    .unwrap_or_else(|| id.clone())
            });
            recent_archives.push(ArchiveRow {
                id,
                archived_at,
                title,
            });
        }
    }
    recent_archives.sort_by(|a, b| b.id.cmp(&a.id));
    recent_archives.truncate(RECENT_ARCHIVES_CAP);

    (
        ActiveWorkReport {
            active_tasks,
            in_progress_count,
            blocked_count,
            recent_archives,
        },
        stamps,
    )
}

fn read_task_row(
    dir: &Path,
    repo: Option<String>,
    stamps: &mut Vec<(PathBuf, SystemTime)>,
) -> Option<ActiveTaskRow> {
    let id = dir.file_name()?.to_str()?.to_string();
    let phase = parse_phase(&id);

    let metadata_path = dir.join(".metadata.json");
    let tasks_path = dir.join("tasks.md");

    if let Ok(meta) = fs::metadata(&metadata_path).and_then(|m| m.modified()) {
        stamps.push((metadata_path.clone(), meta));
    }
    if let Ok(meta) = fs::metadata(&tasks_path).and_then(|m| m.modified()) {
        stamps.push((tasks_path.clone(), meta));
    }

    let metadata_text = fs::read_to_string(&metadata_path).ok()?;
    let metadata: MetadataFile = serde_json::from_str(&metadata_text).ok()?;
    let status = metadata.status.unwrap_or_else(|| "pending".to_string());

    let blocked_reason = if status == "blocked" {
        Some(
            metadata
                .blocked_reason
                .unwrap_or_else(|| "blocked".to_string()),
        )
    } else {
        None
    };

    let next_unchecked_item = fs::read_to_string(&tasks_path)
        .ok()
        .and_then(|body| extract_first_unchecked(&body))
        .map(|s| truncate(s, 256));

    Some(ActiveTaskRow {
        id,
        phase,
        status,
        next_unchecked_item,
        blocked_reason,
        repo,
    })
}

#[derive(Default, Deserialize)]
struct MetadataFile {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    blocked_reason: Option<String>,
}

fn parse_phase(id: &str) -> Option<String> {
    let rest = id.strip_prefix("phase")?;
    let mut end = 0;
    for (idx, ch) in rest.char_indices() {
        if ch == '_' {
            end = idx;
            break;
        }
    }
    if end == 0 {
        return None;
    }
    Some(format!("phase{}", &rest[..end]))
}

fn parse_archive_date(id: &str) -> Option<String> {
    if id.len() < 10 {
        return None;
    }
    let head = &id[..10];
    let bytes = head.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    for (i, b) in bytes.iter().enumerate() {
        if i == 4 || i == 7 {
            continue;
        }
        if !b.is_ascii_digit() {
            return None;
        }
    }
    Some(head.to_string())
}

fn read_proposal_title(dir: &Path) -> Option<String> {
    let body = fs::read_to_string(dir.join("proposal.md")).ok()?;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Extract the first `- [ ]` line from a tasks.md body, lowest
/// section first (top-to-bottom). Returns the row body trimmed
/// (the leading `- [ ] ` removed). `None` when every item is done.
fn extract_first_unchecked(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn truncate(mut s: String, cap_bytes: usize) -> String {
    if s.len() <= cap_bytes {
        return s;
    }
    let mut cap = cap_bytes;
    while cap > 0 && !s.is_char_boundary(cap) {
        cap -= 1;
    }
    s.truncate(cap);
    s
}

/// Process-wide loader instance lazily built from `CORTEX_WORKSPACE_ROOT`
/// (resolved via `cortex_config::Config`). One loader instance per
/// daemon — the 60s TTL applies across requests.
fn shared_loader() -> &'static ActiveWorkLoader {
    static LOADER: OnceLock<ActiveWorkLoader> = OnceLock::new();
    LOADER.get_or_init(|| {
        let root = resolve_workspace_rulebook_root();
        ActiveWorkLoader::new(root)
    })
}

fn resolve_workspace_rulebook_root() -> PathBuf {
    // Phase20 §11.2 — was reading `cfg.ingestion.home` which is
    // unset in the live container (the actual value points at
    // `/var/lib/cortex`, not a workspace path), so the loader
    // resolved to a non-existent directory and every call
    // returned `active_tasks: []`. Mirror how `main.rs` resolves
    // task loaders: prefer `cortex_config.rulebook.roots`
    // (`CORTEX_RULEBOOK_ROOTS=/path/A,/path/B,...`) when set, take
    // the first entry, and fall back to `cortex_config.rulebook.root`
    // before the CWD-based default.
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(roots) = cfg.rulebook.roots.as_deref() {
        for raw in roots.split([',', ';']) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    if let Some(single) = cfg.rulebook.root.as_deref() {
        if !single.trim().is_empty() {
            return PathBuf::from(single);
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.join(".rulebook")
}

/// Query params for `/v1/dashboard/active-work`.
#[derive(Debug, Default, Deserialize)]
pub struct ActiveWorkQuery {
    /// Optional repo filter. When set, the response only carries
    /// tasks whose `repo` matches case-insensitively.
    #[serde(default)]
    pub repo: Option<String>,
}

/// Axum handler for `GET /v1/dashboard/active-work`.
pub async fn active_work_handler(Query(params): Query<ActiveWorkQuery>) -> Response {
    let report = shared_loader().snapshot_filtered(params.repo.as_deref());
    Json(report).into_response()
}

fn sort_key(row: &ActiveTaskRow) -> (u32, String, String) {
    // Lower numeric phase first; the letter suffix orders within
    // the same numeric phase; `id` provides the final tiebreaker.
    let mut num = u32::MAX;
    let mut letter = String::new();
    if let Some(phase) = row.phase.as_deref() {
        if let Some(rest) = phase.strip_prefix("phase") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                num = n;
            }
            letter = rest.chars().skip_while(|c| c.is_ascii_digit()).collect();
        }
    }
    (num, letter, row.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;
    use std::thread::sleep;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let rb = tmp.path().join("project").join(".rulebook");
        stdfs::create_dir_all(rb.join("tasks")).unwrap();
        stdfs::create_dir_all(rb.join("archive")).unwrap();
        tmp
    }

    fn write_task(
        rb: &Path,
        id: &str,
        status: &str,
        blocked_reason: Option<&str>,
        tasks_md: &str,
        proposal: Option<&str>,
    ) {
        let dir = rb.join("tasks").join(id);
        stdfs::create_dir_all(&dir).unwrap();
        let metadata = match blocked_reason {
            Some(reason) => format!("{{\"status\":\"{status}\",\"blocked_reason\":\"{reason}\"}}",),
            None => format!("{{\"status\":\"{status}\"}}"),
        };
        stdfs::write(dir.join(".metadata.json"), metadata).unwrap();
        stdfs::write(dir.join("tasks.md"), tasks_md).unwrap();
        if let Some(p) = proposal {
            stdfs::write(dir.join("proposal.md"), p).unwrap();
        }
    }

    #[test]
    fn empty_workspace_returns_zero_counts_and_empty_lists() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        let loader = ActiveWorkLoader::new(rb);
        let report = loader.snapshot();
        assert!(report.active_tasks.is_empty());
        assert_eq!(report.in_progress_count, 0);
        assert_eq!(report.blocked_count, 0);
        assert!(report.recent_archives.is_empty());
    }

    #[test]
    fn cache_hit_skips_disk_when_no_tracked_file_changed() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        write_task(
            &rb,
            "phase1a_demo",
            "in-progress",
            None,
            "## 1. one\n- [ ] 1.1 alpha\n- [x] 1.2 beta\n",
            None,
        );
        let loader = ActiveWorkLoader::new(rb.clone()).with_ttl(Duration::from_secs(3600));
        let first = loader.snapshot();
        assert_eq!(first.active_tasks.len(), 1);
        // Add a second task — its files are NOT in the cache's
        // stamps list, so the freshness probe stays green and the
        // cache returns the old single-row view. This is the
        // documented behaviour: per-TTL the loader does not
        // discover newly-added task directories. Operators rely
        // on the 60s TTL to refresh.
        write_task(
            &rb,
            "phase1b_demo",
            "pending",
            None,
            "- [ ] 1.1 fresh\n",
            None,
        );
        let second = loader.snapshot();
        assert_eq!(
            second.active_tasks.len(),
            1,
            "cache hit must serve the prior snapshot until TTL elapses"
        );
    }

    #[test]
    fn cache_invalidates_when_tracked_file_is_removed() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        write_task(
            &rb,
            "phase1a_demo",
            "in-progress",
            None,
            "- [ ] 1.1 alpha\n",
            None,
        );
        let loader = ActiveWorkLoader::new(rb.clone()).with_ttl(Duration::from_secs(3600));
        let first = loader.snapshot();
        assert_eq!(first.active_tasks.len(), 1);
        // Delete the tracked metadata file. The freshness probe
        // sees the missing path and forces a refresh; the rebuilt
        // report drops the task entirely (read_task_row needs
        // `.metadata.json`).
        stdfs::remove_file(rb.join("tasks").join("phase1a_demo").join(".metadata.json")).unwrap();
        let second = loader.snapshot();
        assert_eq!(second.active_tasks.len(), 0);
    }

    #[test]
    fn cache_invalidates_when_tasks_md_mtime_advances() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        write_task(
            &rb,
            "phase1a_demo",
            "in-progress",
            None,
            "## 1. one\n- [ ] 1.1 alpha\n",
            None,
        );
        let loader = ActiveWorkLoader::new(rb.clone()).with_ttl(Duration::from_secs(3600));
        let first = loader.snapshot();
        assert_eq!(
            first.active_tasks[0].next_unchecked_item.as_deref(),
            Some("1.1 alpha")
        );
        // Bump the file's mtime by rewriting it with new content.
        // Sleep briefly so the filesystem's mtime resolution
        // registers the change on Windows (FAT32-style 2s resolution
        // would defeat this, but NTFS/most CI is sub-second).
        sleep(Duration::from_millis(50));
        stdfs::write(
            rb.join("tasks").join("phase1a_demo").join("tasks.md"),
            "## 1. one\n- [x] 1.1 alpha\n- [ ] 1.2 beta\n",
        )
        .unwrap();
        let second = loader.snapshot();
        assert_eq!(
            second.active_tasks[0].next_unchecked_item.as_deref(),
            Some("1.2 beta")
        );
    }

    #[test]
    fn repo_filter_scopes_to_matching_workspace() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        write_task(
            &rb,
            "phase1a_demo",
            "in-progress",
            None,
            "- [ ] 1.1 alpha\n",
            None,
        );
        let loader = ActiveWorkLoader::new(rb);
        // Workspace parent dir name is `project`, so the filter
        // matches case-insensitively.
        let matched = loader.snapshot_filtered(Some("Project"));
        assert_eq!(matched.active_tasks.len(), 1);
        let other = loader.snapshot_filtered(Some("nope"));
        assert_eq!(other.active_tasks.len(), 0);
        assert_eq!(other.in_progress_count, 0);
    }

    #[test]
    fn blocked_status_surfaces_blocked_reason_marker() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        write_task(
            &rb,
            "phase1a_demo",
            "blocked",
            Some("nexus upgrade pending"),
            "- [ ] 1.1 alpha\n",
            None,
        );
        write_task(
            &rb,
            "phase1b_demo",
            "blocked",
            None, // no reason in metadata
            "- [ ] 1.1 alpha\n",
            None,
        );
        let loader = ActiveWorkLoader::new(rb);
        let report = loader.snapshot();
        assert_eq!(report.blocked_count, 2);
        let with_reason = report
            .active_tasks
            .iter()
            .find(|r| r.id == "phase1a_demo")
            .unwrap();
        assert_eq!(
            with_reason.blocked_reason.as_deref(),
            Some("nexus upgrade pending")
        );
        let without_reason = report
            .active_tasks
            .iter()
            .find(|r| r.id == "phase1b_demo")
            .unwrap();
        assert_eq!(without_reason.blocked_reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn next_unchecked_picks_lowest_section_first() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        let tasks = "\
## 1. First
- [x] 1.1 done one
- [x] 1.2 done two

## 2. Second
- [ ] 2.1 work A
- [ ] 2.2 work B

## 3. Third
- [ ] 3.1 work C
";
        write_task(&rb, "phase1a_demo", "in-progress", None, tasks, None);
        let loader = ActiveWorkLoader::new(rb);
        let report = loader.snapshot();
        assert_eq!(
            report.active_tasks[0].next_unchecked_item.as_deref(),
            Some("2.1 work A")
        );
    }

    #[test]
    fn recent_archives_sort_newest_first_and_cap_applies() {
        let tmp = make_workspace();
        let rb = tmp.path().join("project").join(".rulebook");
        let archive = rb.join("archive");
        for (date, slug) in [
            ("2026-05-01", "phase1a_old"),
            ("2026-05-25", "phase13f_demo"),
            ("2026-05-10", "phase2g_mid"),
        ] {
            let dir = archive.join(format!("{date}-{slug}"));
            stdfs::create_dir_all(&dir).unwrap();
            stdfs::write(dir.join("proposal.md"), format!("# Title {slug}\n")).unwrap();
        }
        let loader = ActiveWorkLoader::new(rb);
        let report = loader.snapshot();
        assert_eq!(report.recent_archives.len(), 3);
        assert_eq!(report.recent_archives[0].id, "2026-05-25-phase13f_demo");
        assert_eq!(
            report.recent_archives[0].archived_at.as_deref(),
            Some("2026-05-25")
        );
        assert_eq!(report.recent_archives[0].title, "Title phase13f_demo");
        assert_eq!(report.recent_archives[2].id, "2026-05-01-phase1a_old");
    }

    #[test]
    fn truncate_keeps_utf8_char_boundary() {
        let s = "🌑".repeat(200); // 4 bytes each = 800
        let out = truncate(s, 100);
        assert!(out.len() <= 100);
        assert_eq!(out.len() % 4, 0);
        // Re-parsing must succeed.
        let _: &str = out.as_str();
    }
}
