//! Rulebook task loader for the dashboard (`/v1/dashboard/tasks*`).
//!
//! Walks the workspace's `.rulebook/tasks/*` (active) and
//! `.rulebook/archive/*` (archived) directories and parses each task
//! into a [`TaskRow`] the dashboard can render. The loader is read-only
//! — it never writes back to the filesystem.
//!
//! Layout assumed (matches the rulebook v5.3.0 conventions):
//!
//! ```text
//! .rulebook/
//!   tasks/
//!     <id>/
//!       proposal.md       # H1 = title; first paragraph = summary
//!       tasks.md          # checklist with `- [x]` / `- [ ]` items
//!       .metadata.json    # { status, createdAt, updatedAt }
//!       specs/<module>/spec.md
//!   archive/
//!     <YYYY-MM-DD>-<id>/  # same shape as the active task
//! ```
//!
//! The loader is wrapped in a small mtime+TTL cache so the dashboard
//! does not re-walk the filesystem on every request. When the
//! workspace root is unset or unreachable the loader yields empty
//! slices — cold-stack dev keeps booting.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Default TTL between full re-scans of the `.rulebook/` tree.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30);

/// One row in the `/v1/dashboard/tasks` list. Mirrors the
/// `mcp__rulebook__rulebook_task_list` shape extended with phase
/// grouping + checklist progress derived from `tasks.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRow {
    /// Canonical task id (the directory name, with the
    /// `YYYY-MM-DD-` archive prefix stripped when applicable).
    pub id: String,
    /// First H1 of `proposal.md`, falling back to the id.
    pub title: String,
    /// Phase identifier parsed from the id prefix (e.g. `phase2g`).
    /// `None` when the id does not start with `phase<N>`.
    pub phase: Option<String>,
    /// Numeric component of the phase used for stable sort order
    /// (`phase2g` → `(2, "g")`).
    pub phase_num: Option<u32>,
    /// Letter suffix of the phase (`phase2g` → `"g"`).
    pub phase_letter: Option<String>,
    /// One of `pending`, `in-progress`, `completed`, `archived`.
    pub status: String,
    /// Optional ISO-8601 created stamp from `.metadata.json`.
    pub created_at: Option<String>,
    /// Optional ISO-8601 updated stamp from `.metadata.json`.
    pub updated_at: Option<String>,
    /// Date prefix from the archive directory (YYYY-MM-DD), when the
    /// task lives under `.rulebook/archive/`.
    pub archived_at: Option<String>,
    /// Checklist progress derived from `tasks.md`.
    pub progress: ProgressCounts,
    /// First non-heading paragraph of `proposal.md`, trimmed to ~280
    /// chars. Empty when the proposal has no body.
    pub summary: String,
    /// Phase5b multi-project: project slug the task came from
    /// (lowercase, derived from the `.rulebook/` parent directory
    /// name). `None` when only a single workspace is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// Checklist progress counters derived from `tasks.md`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProgressCounts {
    /// Items rendered as `- [x]`.
    pub done: u32,
    /// All checklist items (`- [x]` + `- [ ]`).
    pub total: u32,
}

/// Detail body for `/v1/dashboard/tasks/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetail {
    /// Top-level row.
    #[serde(flatten)]
    pub row: TaskRow,
    /// Full `proposal.md` text (best-effort UTF-8).
    pub proposal_md: String,
    /// Sectioned checklist parsed from `tasks.md`.
    pub checklist: Vec<TaskChecklistSection>,
    /// Files under the task's `specs/` directory (recursive). The
    /// body is intentionally not inlined; consumers fetch them
    /// separately when needed.
    pub specs: Vec<SpecFile>,
    /// `true` when an active task and an archived task share the same
    /// id — the active version wins, this flag tells the GUI that
    /// archived history is also available.
    pub also_archived: bool,
}

/// One section of `tasks.md` (`## N. Title`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskChecklistSection {
    /// Section heading, with the leading `##` and number stripped.
    pub section: String,
    /// Checklist items in file order.
    pub items: Vec<TaskChecklistItem>,
}

/// One checklist item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskChecklistItem {
    /// Item text with the `- [x] ` / `- [ ] ` prefix removed.
    pub text: String,
    /// `true` when the line was rendered as `- [x]`.
    pub done: bool,
}

/// One file under a task's `specs/` directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecFile {
    /// Path relative to the task root.
    pub path: String,
    /// Filename (last component of the path).
    pub name: String,
}

/// Aggregate metrics for the sidebar pill + the Tasks-view stats grid.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskSummary {
    /// All tasks (active + archived).
    pub total: u32,
    /// Tasks currently marked `completed`.
    pub completed: u32,
    /// Tasks currently marked `in-progress`.
    pub in_progress: u32,
    /// Tasks currently marked `pending`.
    pub pending: u32,
    /// Tasks living under `.rulebook/archive/`.
    pub archived: u32,
    /// `(completed + archived) / total` × 100, rounded to one decimal.
    pub completion_pct: f32,
}

/// `by_phase` aggregation entry returned by the list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseBreakdown {
    /// Canonical phase key (`phase2g`).
    pub phase: String,
    /// Numeric component for client-side sort.
    pub phase_num: u32,
    /// Letter component for client-side sort.
    pub phase_letter: String,
    /// All tasks in the phase.
    pub total: u32,
    /// Done tasks (completed + archived).
    pub done: u32,
    /// In-progress tasks.
    pub in_progress: u32,
    /// Pending tasks.
    pub pending: u32,
}

/// Full list-endpoint response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListResponse {
    /// Filtered, sorted, paginated rows.
    pub tasks: Vec<TaskRow>,
    /// Post-filter row count (before pagination).
    pub total: u32,
    /// Phase aggregations across the unfiltered population.
    pub by_phase: Vec<PhaseBreakdown>,
    /// Status totals across the unfiltered population.
    pub by_status: BTreeMap<String, u32>,
}

/// `(field, direction)` tuple driving server-side sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortField {
    /// Default — phase numeric/letter asc, then `updated_at` desc.
    #[default]
    Phase,
    /// `updated_at` from `.metadata.json` (or archive date fallback).
    UpdatedAt,
    /// `created_at` from `.metadata.json` (or archive date fallback).
    CreatedAt,
}

/// Sort direction modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

/// Filter knobs accepted by [`TaskLoader::list`].
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// Restrict to these statuses (any-of). Empty = no restriction.
    pub status: Vec<String>,
    /// Restrict to these phases (any-of, exact match). Empty = all.
    pub phase: Vec<String>,
    /// Restrict to these project slugs (any-of). Empty = all.
    /// Phase5b multi-project filter — matches `TaskRow::repo`.
    pub repo: Vec<String>,
    /// When `false`, archived rows are dropped before paging.
    pub include_archived: bool,
    /// Max rows returned. Capped at 500 by the loader.
    pub limit: usize,
    /// Skip this many rows after sort/filter. Default 0.
    pub offset: usize,
    /// Sort field. Defaults to [`SortField::Phase`].
    pub sort: SortField,
    /// Sort order. Defaults to [`SortOrder::Asc`] for `Phase` and
    /// [`SortOrder::Desc`] for the timestamp fields.
    pub order: Option<SortOrder>,
}

impl ListQuery {
    /// Construct the default query the dashboard's list handler uses
    /// when no params are supplied — show everything (active +
    /// archived), 200 rows, default sort.
    pub fn default_view() -> Self {
        Self {
            status: Vec::new(),
            phase: Vec::new(),
            repo: Vec::new(),
            include_archived: true,
            limit: 200,
            offset: 0,
            sort: SortField::Phase,
            order: None,
        }
    }
}

/// Rulebook task loader. Reads on-demand, caches with TTL, never
/// writes.
pub struct TaskLoader {
    /// Path to the `.rulebook/` directory the loader walks.
    root: PathBuf,
    /// TTL between forced re-scans of the directory tree. Per-task
    /// invalidation still happens whenever a file mtime advances.
    ttl: Duration,
    /// Phase5b — repo slug stamped on every TaskRow this loader
    /// produces. Used by the multi-project aggregator to keep rows
    /// distinguishable when the dashboard merges multiple
    /// `.rulebook/` trees into one list.
    repo: Option<String>,
    /// Cached rows + the `Instant` of the last full scan.
    cache: RwLock<Cached>,
}

/// Internal cache state.
#[derive(Default)]
struct Cached {
    /// Last full scan instant (used together with `ttl`).
    scanned_at: Option<Instant>,
    /// Rows keyed by directory path (so active vs. archive collisions
    /// can both be stored, with the later list filter de-duping by id).
    rows: Vec<CachedRow>,
}

#[derive(Clone)]
struct CachedRow {
    /// Original directory the row was read from.
    dir: PathBuf,
    /// `true` when the directory lives under `archive/`.
    archived: bool,
    /// Mtime stamps captured for `proposal.md` / `tasks.md` /
    /// `.metadata.json` to detect per-task drift between scans.
    stamps: FileStamps,
    /// The parsed row.
    row: TaskRow,
}

#[derive(Clone, Default)]
struct FileStamps {
    proposal: Option<SystemTime>,
    tasks: Option<SystemTime>,
    metadata: Option<SystemTime>,
}

impl TaskLoader {
    /// Construct a loader rooted at the given `.rulebook/` directory.
    /// The path is not validated up-front — a missing directory just
    /// makes [`Self::list`] / [`Self::summary`] return empty results.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ttl: DEFAULT_TTL,
            repo: None,
            cache: RwLock::new(Cached::default()),
        }
    }

    /// Override the default TTL. Mainly used by tests.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Stamp every TaskRow this loader produces with the given repo
    /// slug (lowercase). Phase5b: enables one cortex-api instance to
    /// surface tasks from multiple sibling projects without losing
    /// the project boundary.
    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    /// Returns the repo slug stamped on every row from this loader,
    /// when configured.
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// `.rulebook/` directory the loader walks.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Refresh the cache from disk if either the TTL has elapsed or
    /// the cache has never been populated.
    fn refresh_if_stale(&self) {
        let needs_scan = {
            let cache = self.cache.read().expect("tasks loader cache poisoned");
            match cache.scanned_at {
                None => true,
                Some(ts) => ts.elapsed() >= self.ttl,
            }
        };
        if needs_scan {
            self.full_scan();
        } else {
            self.invalidate_drifted();
        }
    }

    /// Walk the entire `.rulebook/` tree and rebuild the cache.
    fn full_scan(&self) {
        let mut rows: Vec<CachedRow> = Vec::new();
        for entry in scan_dir(&self.root.join("tasks")) {
            if let Some(mut row) = parse_task_dir(&entry, false) {
                row.row.repo = self.repo.clone();
                rows.push(row);
            }
        }
        for entry in scan_dir(&self.root.join("archive")) {
            if let Some(mut row) = parse_task_dir(&entry, true) {
                row.row.repo = self.repo.clone();
                rows.push(row);
            }
        }
        let mut cache = self.cache.write().expect("tasks loader cache poisoned");
        cache.rows = rows;
        cache.scanned_at = Some(Instant::now());
    }

    /// Re-parse only the rows whose constituent files advanced their
    /// mtime since the cached snapshot. Cheaper than a full re-scan
    /// for the steady-state case where most rows are stable.
    fn invalidate_drifted(&self) {
        let snapshot = {
            let cache = self.cache.read().expect("tasks loader cache poisoned");
            cache.rows.clone()
        };
        let mut updates: Vec<(usize, CachedRow)> = Vec::new();
        for (i, cached) in snapshot.iter().enumerate() {
            let current = read_stamps(&cached.dir);
            if !stamps_match(&cached.stamps, &current) {
                if let Some(refreshed) = parse_task_dir(&cached.dir, cached.archived) {
                    updates.push((i, refreshed));
                }
            }
        }
        if updates.is_empty() {
            return;
        }
        let mut cache = self.cache.write().expect("tasks loader cache poisoned");
        for (i, fresh) in updates {
            if let Some(slot) = cache.rows.get_mut(i) {
                *slot = fresh;
            }
        }
    }

    /// Expose the cached rows. The dashboard handlers wrap this with
    /// filtering + pagination on top.
    pub fn rows(&self) -> Vec<CachedRowSnapshot> {
        self.refresh_if_stale();
        let cache = self.cache.read().expect("tasks loader cache poisoned");
        cache
            .rows
            .iter()
            .map(|r| CachedRowSnapshot {
                archived: r.archived,
                dir: r.dir.clone(),
                row: r.row.clone(),
            })
            .collect()
    }

    /// Aggregate counts across every row, regardless of filters.
    pub fn summary(&self) -> TaskSummary {
        let rows = self.rows();
        let total = rows.len() as u32;
        let mut completed = 0u32;
        let mut in_progress = 0u32;
        let mut pending = 0u32;
        let mut archived = 0u32;
        for r in &rows {
            match r.row.status.as_str() {
                "completed" => completed += 1,
                "in-progress" => in_progress += 1,
                "pending" => pending += 1,
                "archived" => archived += 1,
                _ => {}
            }
        }
        let completion_pct = if total == 0 {
            0.0
        } else {
            let num = (completed + archived) as f32 * 100.0;
            (num / total as f32 * 10.0).round() / 10.0
        };
        TaskSummary {
            total,
            completed,
            in_progress,
            pending,
            archived,
            completion_pct,
        }
    }

    /// Filtered, sorted, paginated list. Also surfaces the
    /// pre-filter `by_phase` / `by_status` breakdowns the UI uses
    /// to populate its filter chips.
    pub fn list(&self, query: &ListQuery) -> TaskListResponse {
        let snapshot = self.rows();
        let by_phase = compute_phase_breakdown(&snapshot);
        let by_status = compute_status_breakdown(&snapshot);

        let mut filtered: Vec<TaskRow> = snapshot
            .into_iter()
            .filter(|r| query.include_archived || r.row.status != "archived")
            .filter(|r| query.status.is_empty() || query.status.iter().any(|s| s == &r.row.status))
            .filter(|r| {
                query.phase.is_empty()
                    || r.row
                        .phase
                        .as_ref()
                        .map(|p| query.phase.iter().any(|q| q == p))
                        .unwrap_or(false)
            })
            .filter(|r| {
                query.repo.is_empty()
                    || r.row
                        .repo
                        .as_ref()
                        .map(|p| query.repo.iter().any(|q| q == p))
                        .unwrap_or(false)
            })
            .map(|r| r.row)
            .collect();

        sort_rows(&mut filtered, query.sort, query.order);

        let total = filtered.len() as u32;
        let limit = query.limit.min(500);
        let limit = if limit == 0 { 200 } else { limit };
        let offset = query.offset.min(filtered.len());
        let end = offset.saturating_add(limit).min(filtered.len());
        let page: Vec<TaskRow> = filtered[offset..end].to_vec();
        TaskListResponse {
            tasks: page,
            total,
            by_phase,
            by_status,
        }
    }

    /// Detail body for one task. When the same id is visible both
    /// active and archived, the active row wins and `also_archived`
    /// is set to `true` on the response.
    pub fn detail(&self, id: &str) -> Option<TaskDetail> {
        let snapshot = self.rows();
        let active = snapshot
            .iter()
            .find(|r| !r.archived && r.row.id == id)
            .cloned();
        let archived = snapshot
            .iter()
            .find(|r| r.archived && r.row.id == id)
            .cloned();
        let chosen = active.clone().or_else(|| archived.clone())?;
        let also_archived = active.is_some() && archived.is_some();
        let proposal_md = read_to_string(&chosen.dir.join("proposal.md")).unwrap_or_default();
        let tasks_md = read_to_string(&chosen.dir.join("tasks.md")).unwrap_or_default();
        let checklist = parse_checklist(&tasks_md);
        let specs = list_specs(&chosen.dir);
        Some(TaskDetail {
            row: chosen.row,
            proposal_md,
            checklist,
            specs,
            also_archived,
        })
    }
}

/// Phase5b — fan-out wrapper that aggregates multiple per-project
/// [`TaskLoader`]s. The dashboard handlers depend on this when the
/// operator points cortex-api at a workspace tree containing
/// several `.rulebook/` directories (one per project) so the
/// resulting list / summary span every captured project.
///
/// Single-project deployments stay backward-compatible: a
/// [`MultiTaskLoader`] with one inner loader behaves exactly like
/// the underlying [`TaskLoader`].
pub struct MultiTaskLoader {
    loaders: Vec<TaskLoader>,
}

impl MultiTaskLoader {
    /// Build the aggregator from a non-empty list of per-project
    /// loaders.
    pub fn new(loaders: Vec<TaskLoader>) -> Self {
        Self { loaders }
    }

    /// Number of underlying loaders.
    pub fn len(&self) -> usize {
        self.loaders.len()
    }

    /// `true` when no loaders are configured.
    pub fn is_empty(&self) -> bool {
        self.loaders.is_empty()
    }

    /// Sorted list of repo slugs the aggregator carries. Empty when
    /// all configured loaders are repo-less.
    pub fn repos(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for l in &self.loaders {
            if let Some(r) = l.repo() {
                set.insert(r.to_string());
            }
        }
        set.into_iter().collect()
    }

    /// Aggregate `summary` across every configured loader.
    pub fn summary(&self) -> TaskSummary {
        let mut acc = TaskSummary::default();
        for l in &self.loaders {
            let s = l.summary();
            acc.total += s.total;
            acc.completed += s.completed;
            acc.in_progress += s.in_progress;
            acc.pending += s.pending;
            acc.archived += s.archived;
        }
        if acc.total == 0 {
            acc.completion_pct = 0.0;
        } else {
            let num = (acc.completed + acc.archived) as f32 * 100.0;
            acc.completion_pct = (num / acc.total as f32 * 10.0).round() / 10.0;
        }
        acc
    }

    /// Aggregate `list` across every configured loader. Sorting +
    /// pagination happen on the merged set so cross-project order
    /// matches single-project behaviour.
    pub fn list(&self, query: &ListQuery) -> TaskListResponse {
        // Re-issue each child query with no limit / offset so the
        // outer pagination operates on the merged-and-sorted set.
        // `summary` and `by_phase` aggregations stay correct because
        // we collect them independently from the unbounded merge.
        let inner_query = ListQuery {
            limit: 500,
            offset: 0,
            ..query.clone()
        };
        let mut tasks: Vec<TaskRow> = Vec::new();
        let mut by_phase_map: BTreeMap<String, PhaseBreakdown> = BTreeMap::new();
        let mut by_status: BTreeMap<String, u32> = BTreeMap::new();
        for l in &self.loaders {
            let resp = l.list(&inner_query);
            tasks.extend(resp.tasks);
            for p in resp.by_phase {
                let entry =
                    by_phase_map
                        .entry(p.phase.clone())
                        .or_insert_with(|| PhaseBreakdown {
                            phase: p.phase.clone(),
                            phase_num: p.phase_num,
                            phase_letter: p.phase_letter.clone(),
                            total: 0,
                            done: 0,
                            in_progress: 0,
                            pending: 0,
                        });
                entry.total += p.total;
                entry.done += p.done;
                entry.in_progress += p.in_progress;
                entry.pending += p.pending;
            }
            for (k, v) in resp.by_status {
                *by_status.entry(k).or_insert(0) += v;
            }
        }
        sort_rows(&mut tasks, query.sort, query.order);
        let total = tasks.len() as u32;
        let limit = query.limit.min(500);
        let limit = if limit == 0 { 200 } else { limit };
        let offset = query.offset.min(tasks.len());
        let end = offset.saturating_add(limit).min(tasks.len());
        let page: Vec<TaskRow> = tasks[offset..end].to_vec();
        TaskListResponse {
            tasks: page,
            total,
            by_phase: by_phase_map.into_values().collect(),
            by_status,
        }
    }

    /// Detail lookup — first loader to find the id wins. With well-
    /// scoped per-project loaders this is unambiguous because task
    /// ids are namespaced under their `.rulebook/` directory.
    pub fn detail(&self, id: &str) -> Option<TaskDetail> {
        for l in &self.loaders {
            if let Some(d) = l.detail(id) {
                return Some(d);
            }
        }
        None
    }
}

/// Public projection of a cached row used by the dashboard handlers.
/// The internal `dir` path is exposed for the detail handler to load
/// the full `proposal.md` body without re-scanning.
#[derive(Debug, Clone)]
pub struct CachedRowSnapshot {
    /// `true` when the row came from `.rulebook/archive/`.
    pub archived: bool,
    /// Source directory (used by the detail handler).
    pub dir: PathBuf,
    /// Parsed row.
    pub row: TaskRow,
}

// ---------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------

fn scan_dir(parent: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(parent) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        out.push(p);
    }
    out
}

fn read_stamps(dir: &Path) -> FileStamps {
    FileStamps {
        proposal: file_mtime(&dir.join("proposal.md")),
        tasks: file_mtime(&dir.join("tasks.md")),
        metadata: file_mtime(&dir.join(".metadata.json")),
    }
}

fn stamps_match(a: &FileStamps, b: &FileStamps) -> bool {
    a.proposal == b.proposal && a.tasks == b.tasks && a.metadata == b.metadata
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read_to_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn parse_task_dir(dir: &Path, archived: bool) -> Option<CachedRow> {
    let dir_name = dir.file_name()?.to_str()?.to_string();
    if dir_name.starts_with('.') {
        return None;
    }
    let (id, archived_at) = if archived {
        strip_archive_prefix(&dir_name)
    } else {
        (dir_name.clone(), None)
    };
    let (phase, phase_num, phase_letter) = parse_phase(&id);

    let proposal_path = dir.join("proposal.md");
    let tasks_path = dir.join("tasks.md");
    let metadata_path = dir.join(".metadata.json");

    let proposal = read_to_string(&proposal_path).unwrap_or_default();
    let tasks = read_to_string(&tasks_path).unwrap_or_default();
    let metadata: serde_json::Value = read_to_string(&metadata_path)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);

    let title = extract_title(&proposal).unwrap_or_else(|| id.clone());
    let summary = extract_summary(&proposal);
    let progress = count_progress(&tasks);

    let mut status = metadata
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending")
        .to_string();
    if archived {
        status = "archived".into();
    }
    let created_at = metadata
        .get("createdAt")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| archived_at.as_ref().map(|d| format!("{d}T00:00:00Z")));
    let updated_at = metadata
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| created_at.clone());

    let row = TaskRow {
        id,
        title,
        phase,
        phase_num,
        phase_letter,
        status,
        created_at,
        updated_at,
        archived_at,
        progress,
        summary,
        repo: None,
    };
    Some(CachedRow {
        dir: dir.to_path_buf(),
        archived,
        stamps: FileStamps {
            proposal: file_mtime(&proposal_path),
            tasks: file_mtime(&tasks_path),
            metadata: file_mtime(&metadata_path),
        },
        row,
    })
}

/// Strip a leading `YYYY-MM-DD-` prefix from an archive directory.
/// Returns `(id, Some(date))` on a match and `(name, None)` otherwise.
pub fn strip_archive_prefix(name: &str) -> (String, Option<String>) {
    let bytes = name.as_bytes();
    if bytes.len() < 11 {
        return (name.to_string(), None);
    }
    let is_digit = |i: usize| bytes[i].is_ascii_digit();
    let dash = |i: usize| bytes[i] == b'-';
    if is_digit(0)
        && is_digit(1)
        && is_digit(2)
        && is_digit(3)
        && dash(4)
        && is_digit(5)
        && is_digit(6)
        && dash(7)
        && is_digit(8)
        && is_digit(9)
        && dash(10)
    {
        let date = name[..10].to_string();
        let id = name[11..].to_string();
        (id, Some(date))
    } else {
        (name.to_string(), None)
    }
}

/// Parse the phase prefix of a task id. Returns
/// `(canonical_key, num, letter)`.
pub fn parse_phase(id: &str) -> (Option<String>, Option<u32>, Option<String>) {
    static_regex(|re| {
        let caps = re.captures(id)?;
        let num: u32 = caps.get(1)?.as_str().parse().ok()?;
        let letter = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
        let canonical = if letter.is_empty() {
            format!("phase{num}")
        } else {
            format!("phase{num}{letter}")
        };
        Some((Some(canonical), Some(num), Some(letter)))
    })
    .unwrap_or((None, None, None))
}

fn static_regex<F, T>(f: F) -> Option<T>
where
    F: FnOnce(&Regex) -> Option<T>,
{
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^phase(\d+)([a-z]?)").expect("valid regex"));
    f(re)
}

/// Extract the first H1 (`# Title`) from a markdown body, stripping
/// the `# ` prefix. Returns `None` when no H1 is present.
pub fn extract_title(md: &str) -> Option<String> {
    for line in md.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let t = rest.trim().trim_start_matches("Proposal:").trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Extract the first non-heading paragraph of a markdown body and
/// trim it to ~280 chars. Headings, blank lines, and the literal
/// scaffold placeholder `[Explain why...]` are skipped.
pub fn extract_summary(md: &str) -> String {
    let mut buf = String::new();
    let mut in_section = false;
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_section && !buf.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            in_section = trimmed.starts_with("## Why") || trimmed.starts_with("# ");
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Skip rulebook scaffold placeholders.
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
        if buf.len() >= 280 {
            break;
        }
    }
    if buf.len() > 280 {
        // Walk back to the nearest UTF-8 char boundary so multi-byte
        // chars (em-dashes, em-spaces, accented letters) don't trip
        // `truncate`'s assert. This is the phase5b regression that
        // crashed the daemon when scanning Synap / Vectorizer
        // proposals carrying non-ASCII glyphs.
        let mut idx = 280;
        while idx > 0 && !buf.is_char_boundary(idx) {
            idx -= 1;
        }
        buf.truncate(idx);
        buf.push('…');
    }
    buf
}

/// Count `- [x]` (done) and `- [ ]` (pending) checkbox lines in a
/// `tasks.md` body.
pub fn count_progress(md: &str) -> ProgressCounts {
    let mut done = 0u32;
    let mut total = 0u32;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("- [x]") || t.starts_with("- [X]") {
            done += 1;
            total += 1;
        } else if t.starts_with("- [ ]") {
            total += 1;
        }
    }
    ProgressCounts { done, total }
}

/// Parse a `tasks.md` body into the sectioned checklist shape.
/// Sections are introduced by `## ` lines; checkbox items inside the
/// section are accumulated until the next `## ` heading.
pub fn parse_checklist(md: &str) -> Vec<TaskChecklistSection> {
    let mut out: Vec<TaskChecklistSection> = Vec::new();
    let mut current: Option<TaskChecklistSection> = None;
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(TaskChecklistSection {
                section: rest.trim().to_string(),
                items: Vec::new(),
            });
            continue;
        }
        let (done, body) = if let Some(rest) = trimmed.strip_prefix("- [x] ") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("- [X] ") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            (false, rest)
        } else {
            continue;
        };
        if let Some(section) = current.as_mut() {
            section.items.push(TaskChecklistItem {
                text: body.trim().to_string(),
                done,
            });
        } else {
            // Items appearing before any `## ` heading land in a
            // synthetic "Checklist" bucket so they remain visible.
            current = Some(TaskChecklistSection {
                section: "Checklist".into(),
                items: vec![TaskChecklistItem {
                    text: body.trim().to_string(),
                    done,
                }],
            });
        }
    }
    if let Some(last) = current {
        out.push(last);
    }
    out
}

fn list_specs(dir: &Path) -> Vec<SpecFile> {
    let specs_root = dir.join("specs");
    let mut out = Vec::new();
    walk_files(&specs_root, &specs_root, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk_files(root: &Path, current: &Path, out: &mut Vec<SpecFile>) {
    let it = match fs::read_dir(current) {
        Ok(it) => it,
        Err(_) => return,
    };
    for entry in it.flatten() {
        let p = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk_files(root, &p, out);
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            let path = rel.to_string_lossy().replace('\\', "/");
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(SpecFile { path, name });
        }
    }
}

fn compute_phase_breakdown(rows: &[CachedRowSnapshot]) -> Vec<PhaseBreakdown> {
    let mut acc: BTreeMap<String, PhaseBreakdown> = BTreeMap::new();
    for r in rows {
        let phase = match r.row.phase.as_deref() {
            Some(p) => p.to_string(),
            None => continue,
        };
        let entry = acc.entry(phase.clone()).or_insert(PhaseBreakdown {
            phase: phase.clone(),
            phase_num: r.row.phase_num.unwrap_or(0),
            phase_letter: r.row.phase_letter.clone().unwrap_or_default(),
            total: 0,
            done: 0,
            in_progress: 0,
            pending: 0,
        });
        entry.total += 1;
        match r.row.status.as_str() {
            "completed" | "archived" => entry.done += 1,
            "in-progress" => entry.in_progress += 1,
            "pending" => entry.pending += 1,
            _ => {}
        }
    }
    let mut out: Vec<PhaseBreakdown> = acc.into_values().collect();
    out.sort_by(|a, b| {
        a.phase_num
            .cmp(&b.phase_num)
            .then_with(|| a.phase_letter.cmp(&b.phase_letter))
    });
    out
}

fn compute_status_breakdown(rows: &[CachedRowSnapshot]) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for r in rows {
        *out.entry(r.row.status.clone()).or_insert(0) += 1;
    }
    out
}

fn sort_rows(rows: &mut [TaskRow], field: SortField, order: Option<SortOrder>) {
    let order = order.unwrap_or(match field {
        SortField::Phase => SortOrder::Asc,
        SortField::UpdatedAt | SortField::CreatedAt => SortOrder::Desc,
    });
    rows.sort_by(|a, b| {
        let ord = match field {
            SortField::Phase => a
                .phase_num
                .unwrap_or(u32::MAX)
                .cmp(&b.phase_num.unwrap_or(u32::MAX))
                .then_with(|| {
                    a.phase_letter
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.phase_letter.as_deref().unwrap_or(""))
                })
                .then_with(|| ts_cmp(&a.updated_at, &b.updated_at).reverse()),
            SortField::UpdatedAt => ts_cmp(&a.updated_at, &b.updated_at),
            SortField::CreatedAt => ts_cmp(&a.created_at, &b.created_at),
        };
        match order {
            SortOrder::Asc => ord,
            SortOrder::Desc => ord.reverse(),
        }
    });
}

fn ts_cmp(a: &Option<String>, b: &Option<String>) -> std::cmp::Ordering {
    let pa = a.as_deref().and_then(parse_iso).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let pb = b.as_deref().and_then(parse_iso).unwrap_or(DateTime::<Utc>::MIN_UTC);
    pa.cmp(&pb)
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_archive_prefix_recognises_iso_dates() {
        let (id, date) = strip_archive_prefix("2026-04-27-phase2_keyword_lane_live_meilisearch");
        assert_eq!(id, "phase2_keyword_lane_live_meilisearch");
        assert_eq!(date.as_deref(), Some("2026-04-27"));
    }

    #[test]
    fn strip_archive_prefix_passes_through_non_dated_dirs() {
        let (id, date) = strip_archive_prefix("phase4d_indexing_consistency_doctor");
        assert_eq!(id, "phase4d_indexing_consistency_doctor");
        assert!(date.is_none());
    }

    #[test]
    fn strip_archive_prefix_handles_short_names() {
        let (id, date) = strip_archive_prefix("short");
        assert_eq!(id, "short");
        assert!(date.is_none());
    }

    #[test]
    fn parse_phase_handles_simple_and_lettered_phases() {
        let (key, num, letter) = parse_phase("phase0_cortex-core");
        assert_eq!(key.as_deref(), Some("phase0"));
        assert_eq!(num, Some(0));
        assert_eq!(letter.as_deref(), Some(""));

        let (key, num, letter) = parse_phase("phase2g_dashboard_enriched_metrics");
        assert_eq!(key.as_deref(), Some("phase2g"));
        assert_eq!(num, Some(2));
        assert_eq!(letter.as_deref(), Some("g"));

        let (key, num, letter) = parse_phase("phase4a_fulltext_fanout_parity_and_stale_meili_cleanup");
        assert_eq!(key.as_deref(), Some("phase4a"));
        assert_eq!(num, Some(4));
        assert_eq!(letter.as_deref(), Some("a"));
    }

    #[test]
    fn parse_phase_returns_none_for_unrelated_ids() {
        let (key, num, letter) = parse_phase("ad-hoc-experiment");
        assert!(key.is_none());
        assert!(num.is_none());
        assert!(letter.is_none());
    }

    #[test]
    fn count_progress_tallies_done_and_pending() {
        let md = "## 1. Foo\n- [x] one\n- [X] two\n- [ ] three\nirrelevant\n## 2. Bar\n- [ ] four\n";
        let p = count_progress(md);
        assert_eq!(p.done, 2);
        assert_eq!(p.total, 4);
    }

    #[test]
    fn parse_checklist_groups_items_by_section() {
        let md = "## 1. Foo\n- [x] alpha\n- [ ] beta\n## 2. Bar\n- [ ] gamma\n";
        let sections = parse_checklist(md);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].section, "1. Foo");
        assert_eq!(sections[0].items.len(), 2);
        assert!(sections[0].items[0].done);
        assert!(!sections[0].items[1].done);
        assert_eq!(sections[1].section, "2. Bar");
        assert_eq!(sections[1].items[0].text, "gamma");
    }

    #[test]
    fn parse_checklist_handles_orphan_items_without_a_section() {
        let md = "- [x] orphan\n## 1. Real\n- [ ] real\n";
        let sections = parse_checklist(md);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].section, "Checklist");
        assert!(sections[0].items[0].done);
    }

    #[test]
    fn extract_title_strips_proposal_prefix() {
        let md = "# Proposal: phase5a_dashboard_tasks_backend\n\n## Why\n";
        assert_eq!(
            extract_title(md).as_deref(),
            Some("phase5a_dashboard_tasks_backend")
        );
    }

    #[test]
    fn extract_summary_picks_first_paragraph_after_why() {
        let md = "# Proposal: foo\n\n## Why\n\nFirst sentence here. Second sentence.\n\n## What\n\nIgnored.\n";
        let s = extract_summary(md);
        assert!(s.starts_with("First sentence"));
        assert!(!s.contains("Ignored"));
    }

    #[test]
    fn extract_summary_skips_scaffold_placeholders() {
        let md = "# Proposal: foo\n\n## Why\n[Explain why this change is needed - minimum 20 characters]\n\nReal body here.\n";
        let s = extract_summary(md);
        assert_eq!(s, "Real body here.");
    }

    #[test]
    fn loader_yields_empty_when_root_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loader = TaskLoader::new(dir.path().join("absent"));
        assert!(loader.rows().is_empty());
        assert_eq!(loader.summary().total, 0);
    }

    #[test]
    fn loader_walks_active_and_archived_with_progress_and_phase() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let active = root.join("tasks/phase2g_dashboard_enriched_metrics");
        fs::create_dir_all(&active).unwrap();
        fs::write(
            active.join("proposal.md"),
            "# Proposal: phase2g_dashboard_enriched_metrics\n\n## Why\n\nWiden the dashboard.\n",
        )
        .unwrap();
        fs::write(
            active.join("tasks.md"),
            "## 1. Foo\n- [x] done\n- [ ] pending\n",
        )
        .unwrap();
        fs::write(
            active.join(".metadata.json"),
            r#"{"status":"in-progress","createdAt":"2026-04-27T00:12:58.590Z","updatedAt":"2026-04-28T17:00:57.841Z"}"#,
        )
        .unwrap();

        let archived = root.join("archive/2026-04-26-phase1_query-api");
        fs::create_dir_all(&archived).unwrap();
        fs::write(
            archived.join("proposal.md"),
            "# Proposal: phase1_query-api\n\n## Why\n\nQuery API skeleton.\n",
        )
        .unwrap();
        fs::write(archived.join("tasks.md"), "## 1. Foo\n- [x] only\n").unwrap();
        fs::write(
            archived.join(".metadata.json"),
            r#"{"status":"pending","createdAt":"2026-04-18T01:21:33.009Z","updatedAt":"2026-04-18T01:21:33.009Z"}"#,
        )
        .unwrap();

        let loader = TaskLoader::new(root).with_ttl(Duration::from_millis(0));
        let rows = loader.rows();
        assert_eq!(rows.len(), 2);

        let archived_row = rows
            .iter()
            .find(|r| r.row.id == "phase1_query-api")
            .expect("archived row");
        assert!(archived_row.archived);
        assert_eq!(archived_row.row.status, "archived");
        assert_eq!(archived_row.row.archived_at.as_deref(), Some("2026-04-26"));
        assert_eq!(archived_row.row.phase.as_deref(), Some("phase1"));

        let active_row = rows
            .iter()
            .find(|r| r.row.id == "phase2g_dashboard_enriched_metrics")
            .expect("active row");
        assert_eq!(active_row.row.status, "in-progress");
        assert_eq!(active_row.row.phase.as_deref(), Some("phase2g"));
        assert_eq!(active_row.row.progress.done, 1);
        assert_eq!(active_row.row.progress.total, 2);
        assert!(active_row.row.summary.contains("Widen"));

        let summary = loader.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.archived, 1);
        assert_eq!(summary.in_progress, 1);
        assert!(summary.completion_pct > 0.0);

        let listed = loader.list(&ListQuery::default_view());
        assert_eq!(listed.total, 2);
        assert!(listed.by_status.contains_key("archived"));
        assert!(listed.by_phase.iter().any(|p| p.phase == "phase1"));
        assert!(listed.by_phase.iter().any(|p| p.phase == "phase2g"));

        let mut filtered = ListQuery::default_view();
        filtered.include_archived = false;
        let filtered = loader.list(&filtered);
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.tasks[0].id, "phase2g_dashboard_enriched_metrics");

        let detail = loader
            .detail("phase2g_dashboard_enriched_metrics")
            .expect("detail present");
        assert!(!detail.also_archived);
        assert!(detail.proposal_md.contains("Widen"));
        assert_eq!(detail.checklist.len(), 1);
    }

    #[test]
    fn loader_marks_also_archived_when_id_is_in_both_trees() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let active = root.join("tasks/phase1_demo");
        let archived = root.join("archive/2026-04-01-phase1_demo");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(&archived).unwrap();
        for d in [&active, &archived] {
            fs::write(d.join("proposal.md"), "# Proposal: phase1_demo\n\n## Why\n\nDemo.\n").unwrap();
            fs::write(d.join("tasks.md"), "## 1. Foo\n- [x] only\n").unwrap();
            fs::write(
                d.join(".metadata.json"),
                r#"{"status":"in-progress","createdAt":"2026-04-01T00:00:00Z","updatedAt":"2026-04-01T00:00:00Z"}"#,
            )
            .unwrap();
        }
        let loader = TaskLoader::new(root).with_ttl(Duration::from_millis(0));
        let detail = loader.detail("phase1_demo").expect("detail");
        assert!(detail.also_archived);
        assert_eq!(detail.row.status, "in-progress");
    }
}
