//! Phase11k §4 — governance auto-republish watcher.
//!
//! The phase11i §5 tail watcher polls Claude Code session JSONL files
//! and re-emits envelopes when sessions advance. Phase11k extends the
//! watcher's footprint to ALSO cover the governance source files
//! (`.rulebook/decisions/`, `.rulebook/laws/`, `.claude/rules/`,
//! `AGENTS.override.md`, `AGENTS.md`). On change (rename / write /
//! delete), the watcher hands the change to a [`GovernanceEmitter`]
//! that is responsible for shipping a `cortex.events.bootstrap`
//! envelope. The worker side dedupes via `content_hash`, so a noisy
//! filesystem (file-saved-then-touched-by-IDE / unchanged re-write)
//! still produces at most one downstream document per content hash.
//!
//! Cross-platform notes:
//! - Same polling rationale as `tail.rs`: avoid `notify-rs` /
//!   inotify / `ReadDirectoryChangesW`. Governance changes are rare;
//!   the cost of an O(N) stat sweep is negligible compared to the
//!   correctness benefit of a single uniform mechanism.
//! - Cursor key is the absolute path: a delete then-recreate at the
//!   same path produces a fresh `(0, 0)` cursor, so the next change
//!   re-emits even if `(mtime, len)` collides.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// Default governance paths the watcher inspects, evaluated relative
/// to the watch root. The list captures every file family the
/// phase11k spec calls out:
///
/// - `.rulebook/decisions/` — ADRs, fan out per-file.
/// - `.rulebook/laws/` — single-law YAML/MD files.
/// - `.claude/rules/` — `.claude/rules/*.md` rule snippets.
/// - `AGENTS.override.md` / `AGENTS.md` — LAW-CORTEX-* declarations.
pub const DEFAULT_GOVERNANCE_PATHS: &[&str] = &[
    ".rulebook/decisions",
    ".rulebook/laws",
    ".claude/rules",
    "AGENTS.override.md",
    "AGENTS.md",
];

/// One observed change the watcher hands to the emitter. The change
/// kind tells the emitter whether the file lives at this path now
/// (`Upserted`) or has been removed since the last tick (`Deleted`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovernanceChange {
    /// File exists and has new content. Carries the body so the
    /// emitter does not have to re-read it (and risk a TOCTOU split
    /// from the cursor stat).
    Upserted {
        /// Absolute path of the changed file.
        path: PathBuf,
        /// Repo-rooted forward-slash path the bootstrap envelope
        /// stamps as `source.path`.
        rel_path: String,
        /// Full file body at the moment of detection.
        body: String,
        /// `sha256(body)` hex string. Lets the emitter dedupe
        /// re-emits without re-hashing.
        content_hash: String,
    },
    /// File previously existed but has now disappeared. The emitter
    /// handles tombstone publication (or no-op).
    Deleted {
        /// Absolute path of the removed file.
        path: PathBuf,
        /// Repo-rooted forward-slash path.
        rel_path: String,
    },
}

/// Sink the watcher hands changes to. Production wiring publishes
/// onto `cortex.events.bootstrap`; tests use [`MemoryGovernanceEmitter`]
/// to capture changes synchronously.
pub trait GovernanceEmitter: Send {
    /// Handle a single observed change. Returning `Err` flags the
    /// tick as degraded but does not abort subsequent files.
    fn emit(&mut self, change: GovernanceChange) -> Result<(), String>;
}

/// In-memory emitter used by tests and `--dry-run` callers. The
/// shared `Arc<Mutex<Vec<_>>>` lets callers retain a handle for
/// inspection after the watcher has taken ownership of the emitter
/// itself.
#[derive(Debug, Default, Clone)]
pub struct MemoryGovernanceEmitter {
    /// Every change handed to `emit`, in observation order.
    captured: Arc<Mutex<Vec<GovernanceChange>>>,
}

impl MemoryGovernanceEmitter {
    /// Construct an empty capture buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot every change captured so far.
    pub fn captured(&self) -> Vec<GovernanceChange> {
        self.captured
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }
}

impl GovernanceEmitter for MemoryGovernanceEmitter {
    fn emit(&mut self, change: GovernanceChange) -> Result<(), String> {
        if let Ok(mut guard) = self.captured.lock() {
            guard.push(change);
        }
        Ok(())
    }
}

/// Per-file cursor — `(content_hash, exists)` pair. We dedupe on the
/// content hash directly rather than `(mtime, len)` because IDEs
/// touch save-without-content-change frequently; the hash check
/// stays correct when mtime jitters but bytes are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GovernanceCursor {
    content_hash: String,
}

/// Long-lived watcher state — owns cursors, the emitter, and the
/// configured governance globs. Driven externally via repeated
/// [`Self::tick_once`] calls so the host (the existing tail watcher
/// loop, an integration test, or a CLI replay) controls cadence.
pub struct GovernanceWatcher {
    root: PathBuf,
    paths: Vec<String>,
    cursors: BTreeMap<PathBuf, GovernanceCursor>,
    emitter: Box<dyn GovernanceEmitter>,
}

/// Per-tick report — what the watcher saw on the latest pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GovernanceTickReport {
    /// Number of files surfaced by the walk this tick.
    pub files_seen: usize,
    /// Files that produced an `Upserted` change.
    pub upserted: usize,
    /// Files that produced a `Deleted` change.
    pub deleted: usize,
    /// Files whose hash matched the prior cursor (no-op).
    pub unchanged: usize,
    /// Per-file errors (read / hash failures). Empty on a clean tick.
    pub errors: Vec<String>,
}

impl GovernanceWatcher {
    /// Build a watcher rooted at `root`, watching the default
    /// governance path list. The emitter is owned by the watcher;
    /// callers wanting to inspect captured changes should pass a
    /// [`MemoryGovernanceEmitter`] and read `captured` out via
    /// [`Self::take_emitter`].
    pub fn with_defaults(root: impl Into<PathBuf>, emitter: Box<dyn GovernanceEmitter>) -> Self {
        Self::new(root, DEFAULT_GOVERNANCE_PATHS, emitter)
    }

    /// Build a watcher with an explicit path list — exposes
    /// [`DEFAULT_GOVERNANCE_PATHS`] for callers that need to extend
    /// or override it.
    pub fn new<P: AsRef<str>>(
        root: impl Into<PathBuf>,
        paths: &[P],
        emitter: Box<dyn GovernanceEmitter>,
    ) -> Self {
        Self {
            root: root.into(),
            paths: paths.iter().map(|p| p.as_ref().to_string()).collect(),
            cursors: BTreeMap::new(),
            emitter,
        }
    }

    /// Drop the watcher and return the owned emitter. Tests that
    /// passed in a [`MemoryGovernanceEmitter`] use this to read the
    /// captured change log.
    pub fn take_emitter(self) -> Box<dyn GovernanceEmitter> {
        self.emitter
    }

    /// Run one polling tick: stat every governance path, hash any
    /// file whose cursor is stale, hand the change to the emitter,
    /// and update the cursor. Returns counts the host watcher's
    /// healthz endpoint can surface.
    pub fn tick_once(&mut self) -> GovernanceTickReport {
        let mut report = GovernanceTickReport::default();
        let mut seen: Vec<PathBuf> = Vec::new();

        for entry in self.iter_files() {
            let abs = entry.absolute.clone();
            let rel = entry.rel_path.clone();
            seen.push(abs.clone());
            report.files_seen += 1;
            let body = match fs::read_to_string(&abs) {
                Ok(b) => b,
                Err(err) => {
                    report.errors.push(format!("{}: {err}", abs.display()));
                    continue;
                }
            };
            let content_hash = sha256_hex(body.as_bytes());
            let prior = self.cursors.get(&abs).cloned();
            if prior
                .as_ref()
                .map(|c| c.content_hash == content_hash)
                .unwrap_or(false)
            {
                report.unchanged += 1;
                continue;
            }
            let change = GovernanceChange::Upserted {
                path: abs.clone(),
                rel_path: rel,
                body,
                content_hash: content_hash.clone(),
            };
            if let Err(err) = self.emitter.emit(change) {
                report
                    .errors
                    .push(format!("emit {}: {err}", abs.display()));
                continue;
            }
            self.cursors.insert(abs, GovernanceCursor { content_hash });
            report.upserted += 1;
        }

        // Detect deletions: any cursor whose path isn't in this tick's
        // walk has been removed. Drain those and hand the emitter a
        // tombstone change.
        let deleted_paths: Vec<PathBuf> = self
            .cursors
            .keys()
            .filter(|p| !seen.contains(p))
            .cloned()
            .collect();
        for path in deleted_paths {
            self.cursors.remove(&path);
            let rel_path = self
                .relativise(&path)
                .unwrap_or_else(|| path.display().to_string());
            let change = GovernanceChange::Deleted {
                path: path.clone(),
                rel_path,
            };
            if let Err(err) = self.emitter.emit(change) {
                report
                    .errors
                    .push(format!("emit-delete {}: {err}", path.display()));
                continue;
            }
            report.deleted += 1;
        }

        report
    }

    fn iter_files(&self) -> Vec<WalkedEntry> {
        let mut out: Vec<WalkedEntry> = Vec::new();
        for entry in &self.paths {
            let abs = self.root.join(entry);
            if !abs.exists() {
                continue;
            }
            if abs.is_file() {
                if let Some(rel) = self.relativise(&abs) {
                    out.push(WalkedEntry {
                        absolute: abs,
                        rel_path: rel,
                    });
                }
                continue;
            }
            // Directory — walk recursively. Governance files are
            // tracked artefacts so we deliberately skip the
            // gitignore filter (a developer running `git stash`-and-
            // experiment shouldn't lose visibility on rule changes).
            let walker = ignore::WalkBuilder::new(&abs)
                .standard_filters(false)
                .git_ignore(false)
                .git_exclude(false)
                .git_global(false)
                .hidden(false)
                .build();
            for w in walker.flatten() {
                let metadata = match w.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !metadata.is_file() {
                    continue;
                }
                let p = w.path().to_path_buf();
                if let Some(rel) = self.relativise(&p) {
                    out.push(WalkedEntry {
                        absolute: p,
                        rel_path: rel,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.absolute.cmp(&b.absolute));
        out.dedup_by(|a, b| a.absolute == b.absolute);
        out
    }

    fn relativise(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        Some(
            rel.to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_string(),
        )
    }
}

#[derive(Debug, Clone)]
struct WalkedEntry {
    absolute: PathBuf,
    rel_path: String,
}

/// Compute the lowercase-hex `sha256` of `bytes`. Used as the cursor
/// dedupe key.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[allow(dead_code)]
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_root() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(abs, body).expect("write");
    }

    #[test]
    fn first_tick_emits_upsert_for_each_governance_file() {
        let root = make_root();
        write(root.path(), "AGENTS.override.md", "# laws\n## LAW-CORTEX-001\n");
        write(
            root.path(),
            ".rulebook/decisions/0001-pick-meili.md",
            "# ADR-0001\nStatus: accepted\n",
        );
        write(
            root.path(),
            ".claude/rules/no-shortcuts.md",
            "# Rule\nNever ship stubs.\n",
        );
        let mut watcher = GovernanceWatcher::with_defaults(
            root.path(),
            Box::new(MemoryGovernanceEmitter::new()),
        );
        let report = watcher.tick_once();
        assert_eq!(report.files_seen, 3);
        assert_eq!(report.upserted, 3);
        assert_eq!(report.unchanged, 0);
        assert!(report.errors.is_empty(), "errors={:?}", report.errors);
    }

    #[test]
    fn unchanged_files_do_not_re_emit_on_subsequent_ticks() {
        let root = make_root();
        write(root.path(), "AGENTS.override.md", "# laws\n## LAW-CORTEX-001\n");
        let mut watcher = GovernanceWatcher::with_defaults(
            root.path(),
            Box::new(MemoryGovernanceEmitter::new()),
        );
        watcher.tick_once();
        let report = watcher.tick_once();
        assert_eq!(report.upserted, 0, "second tick must not re-emit");
        assert_eq!(report.unchanged, 1);
    }

    #[test]
    fn modified_file_re_emits_with_fresh_content_hash() {
        let root = make_root();
        let path = "AGENTS.override.md";
        write(root.path(), path, "v1");
        let emitter = MemoryGovernanceEmitter::new();
        let handle = emitter.clone();
        let mut watcher = GovernanceWatcher::with_defaults(root.path(), Box::new(emitter));
        watcher.tick_once();
        write(root.path(), path, "v2 — bigger body");
        let report = watcher.tick_once();
        assert_eq!(
            report.upserted, 1,
            "content change must produce a fresh upsert",
        );
        let captured = handle.captured();
        let upserts: Vec<&GovernanceChange> = captured
            .iter()
            .filter(|c| matches!(c, GovernanceChange::Upserted { .. }))
            .collect();
        assert_eq!(upserts.len(), 2, "v1 and v2 both produce one upsert each");
        let mut hashes: Vec<String> = upserts
            .iter()
            .filter_map(|c| match c {
                GovernanceChange::Upserted { content_hash, .. } => Some(content_hash.clone()),
                _ => None,
            })
            .collect();
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes.len(), 2);
    }

    #[test]
    fn deleted_file_emits_tombstone_on_next_tick() {
        let root = make_root();
        let path = "AGENTS.override.md";
        write(root.path(), path, "x");
        let mut watcher = GovernanceWatcher::with_defaults(
            root.path(),
            Box::new(MemoryGovernanceEmitter::new()),
        );
        watcher.tick_once();
        fs::remove_file(root.path().join(path)).unwrap();
        let report = watcher.tick_once();
        assert_eq!(report.deleted, 1);
        assert_eq!(report.upserted, 0);
        assert_eq!(report.files_seen, 0);
    }

    #[test]
    fn missing_governance_paths_do_not_error() {
        // A repo without `.rulebook/laws/` or `AGENTS.override.md`
        // is the common case — the watcher MUST treat absent
        // paths as zero files, not as errors. Otherwise every
        // sibling Hive repo without governance content would mark
        // the watcher degraded.
        let root = make_root();
        let mut watcher = GovernanceWatcher::with_defaults(
            root.path(),
            Box::new(MemoryGovernanceEmitter::new()),
        );
        let report = watcher.tick_once();
        assert_eq!(report.files_seen, 0);
        assert!(report.errors.is_empty());
    }
}
