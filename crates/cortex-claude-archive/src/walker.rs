//! Directory traversal — phase11i §1.4.
//!
//! Walks `~/.claude/projects/<project>/<session>.jsonl` (the
//! primary corpus), plus the optional sidecar paths the user
//! cares about: `history.jsonl`, `todos/*.json`, `plans/*.md`,
//! and the `~/.codex/` parallel corpus.
//!
//! Output is a flat list of [`WalkEntry`] records — the caller
//! (CLI / watcher) decides which sink to feed each entry into.
//! The walker stays IO-free for tests via the `walk_filtered`
//! variant that takes a pre-built directory listing.
//!
//! Excludes are hard-coded for now (cache, debug, telemetry,
//! shell-snapshots, paste-cache, downloads). The §1.8 redactor +
//! §1.5 emitter handle the per-file content filtering;
//! `walk_filtered` only decides whether the path is worth
//! opening.

use std::path::{Path, PathBuf};

/// One file the walker thinks the caller should consume. The
/// `kind` discriminates which mapper / sidecar projection to
/// apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// What the caller should do with this file.
    pub kind: WalkKind,
    /// Project directory name as the user sees it (e.g.
    /// `e--HiveLLM-Cortex`). Empty for global sidecars (history,
    /// settings).
    pub project_dir: String,
    /// File size in bytes (best-effort; `0` when stat fails or
    /// the caller used [`walk_filtered`]).
    pub size_bytes: u64,
}

/// Discriminator for [`WalkEntry::kind`]. The CLI maps each
/// variant to the matching projector (mapper for sessions,
/// sidecar projector for history/todos/plans).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkKind {
    /// `<project>/<session>.jsonl` — full conversation transcript.
    /// Goes through the [`crate::reader`] + [`crate::mapper`]
    /// pipeline.
    Session,
    /// `~/.claude/history.jsonl` — global command history. Each
    /// line synthesises a `Kind::Turn` envelope with
    /// `assistant_message: null` (mirrors the partial-turn shape
    /// the mapper already produces).
    GlobalHistory,
    /// `~/.claude/todos/<uuid>-agent-<uuid>.json` — per-agent
    /// task list. Becomes `Kind::Memory` envelope with
    /// `memory_type = "todo"`.
    Todo,
    /// `~/.claude/plans/<slug>.md` — session plan. Becomes
    /// `Kind::Artifact` (artifact_type=snippet, language=markdown).
    Plan,
    /// `~/.claude/settings.json` — global harness settings.
    /// Becomes a single `Kind::Memory` envelope on first ingest.
    Settings,
    /// `~/.codex/history.jsonl` or `~/.codex/sessions/*.jsonl`.
    /// Same projection as `Session` / `GlobalHistory` but the
    /// emitter stamps `Envelope.tool = "openai-codex"`.
    CodexSession,
    /// `~/.codex/history.jsonl`.
    CodexHistory,
}

/// Configurable scope for the walker. Mirrors the CLI flags
/// (`--root`, `--projects-only`, `--sidecars`, `--codex`).
#[derive(Debug, Clone)]
pub struct WalkConfig {
    /// Root containing the `projects/` subdir + sidecars.
    /// Defaults to `~/.claude/`.
    pub root: PathBuf,
    /// When true, walk only `<root>/projects/`. Defaults true so
    /// the cheap one-shot bootstrap path stays scoped.
    pub projects_only: bool,
    /// When true, also walk `history.jsonl`, `todos/`, `plans/`,
    /// `settings.json`. Defaults false.
    pub sidecars: bool,
    /// When true, also walk `~/.codex/` sibling corpus. Defaults
    /// false.
    pub codex: bool,
}

impl WalkConfig {
    /// Default configuration — projects only, no sidecars, no
    /// codex. Mirrors the CLI's `--projects-only` default.
    pub fn projects_only(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            projects_only: true,
            sidecars: false,
            codex: false,
        }
    }
}

/// Walk the configured roots and return every [`WalkEntry`] the
/// caller should consume. Filesystem I/O happens here; tests use
/// [`walk_filtered`] to drive the same logic against a synthetic
/// listing.
pub fn walk(config: &WalkConfig) -> Vec<WalkEntry> {
    let mut entries = Vec::new();
    let projects_dir = config.root.join("projects");
    if projects_dir.is_dir() {
        for project in read_dir_sorted(&projects_dir) {
            if !project.is_dir() {
                continue;
            }
            let project_name = project
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            for entry in read_dir_sorted(&project) {
                if !is_jsonl_file(&entry) {
                    continue;
                }
                let size = stat_size(&entry);
                entries.push(WalkEntry {
                    path: entry,
                    kind: WalkKind::Session,
                    project_dir: project_name.clone(),
                    size_bytes: size,
                });
            }
        }
    }
    if !config.projects_only && config.sidecars {
        let history = config.root.join("history.jsonl");
        if history.is_file() {
            entries.push(WalkEntry {
                size_bytes: stat_size(&history),
                path: history,
                kind: WalkKind::GlobalHistory,
                project_dir: String::new(),
            });
        }
        let todos = config.root.join("todos");
        if todos.is_dir() {
            for entry in read_dir_sorted(&todos) {
                if entry.extension().and_then(|s| s.to_str()) == Some("json") {
                    entries.push(WalkEntry {
                        size_bytes: stat_size(&entry),
                        path: entry,
                        kind: WalkKind::Todo,
                        project_dir: String::new(),
                    });
                }
            }
        }
        let plans = config.root.join("plans");
        if plans.is_dir() {
            for entry in read_dir_sorted(&plans) {
                if entry.extension().and_then(|s| s.to_str()) == Some("md") {
                    entries.push(WalkEntry {
                        size_bytes: stat_size(&entry),
                        path: entry,
                        kind: WalkKind::Plan,
                        project_dir: String::new(),
                    });
                }
            }
        }
        let settings = config.root.join("settings.json");
        if settings.is_file() {
            entries.push(WalkEntry {
                size_bytes: stat_size(&settings),
                path: settings,
                kind: WalkKind::Settings,
                project_dir: String::new(),
            });
        }
    }
    if config.codex {
        let codex_root = codex_root_for(&config.root);
        let codex_history = codex_root.join("history.jsonl");
        if codex_history.is_file() {
            entries.push(WalkEntry {
                size_bytes: stat_size(&codex_history),
                path: codex_history,
                kind: WalkKind::CodexHistory,
                project_dir: String::new(),
            });
        }
        let codex_sessions = codex_root.join("sessions");
        if codex_sessions.is_dir() {
            for entry in read_dir_sorted(&codex_sessions) {
                if is_jsonl_file(&entry) {
                    entries.push(WalkEntry {
                        size_bytes: stat_size(&entry),
                        path: entry,
                        kind: WalkKind::CodexSession,
                        project_dir: String::new(),
                    });
                }
            }
        }
    }
    entries
}

/// Pure variant of [`walk`] for tests — takes a synthetic listing
/// of `(path, kind, project_dir, size_bytes)` tuples and returns
/// the same shape. Lets the unit tests pin walker behaviour
/// without touching the filesystem.
pub fn walk_filtered<I>(config: &WalkConfig, listing: I) -> Vec<WalkEntry>
where
    I: IntoIterator<Item = WalkEntry>,
{
    listing
        .into_iter()
        .filter(|e| match e.kind {
            WalkKind::Session => starts_with(&e.path, &config.root.join("projects")),
            WalkKind::GlobalHistory | WalkKind::Todo | WalkKind::Plan | WalkKind::Settings => {
                !config.projects_only && config.sidecars
            }
            WalkKind::CodexHistory | WalkKind::CodexSession => config.codex,
        })
        .collect()
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd.filter_map(|r| r.ok().map(|e| e.path())).collect();
    paths.sort();
    paths
}

fn stat_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn is_jsonl_file(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
}

fn starts_with(path: &Path, prefix: &Path) -> bool {
    path.starts_with(prefix)
}

fn codex_root_for(claude_root: &Path) -> PathBuf {
    // Sibling: `~/.claude` → `~/.codex`. When the claude root
    // does not live under a parent (rare; tests pass `/tmp/foo`),
    // fall back to the literal sibling rename.
    let parent = claude_root.parent().unwrap_or(Path::new("/"));
    let basename = claude_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(".claude");
    let codex_basename = if basename == ".claude" {
        ".codex"
    } else {
        ".codex"
    };
    parent.join(codex_basename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(path: &str, kind: WalkKind, project: &str) -> WalkEntry {
        WalkEntry {
            path: PathBuf::from(path),
            kind,
            project_dir: project.to_string(),
            size_bytes: 0,
        }
    }

    #[test]
    fn projects_only_keeps_session_entries() {
        let cfg = WalkConfig::projects_only("/root");
        let listing = vec![
            entry("/root/projects/a/s.jsonl", WalkKind::Session, "a"),
            entry("/root/history.jsonl", WalkKind::GlobalHistory, ""),
        ];
        let out = walk_filtered(&cfg, listing);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].kind, WalkKind::Session));
    }

    #[test]
    fn sidecars_disabled_when_projects_only() {
        let cfg = WalkConfig::projects_only("/root");
        let listing = vec![
            entry("/root/history.jsonl", WalkKind::GlobalHistory, ""),
            entry("/root/todos/a.json", WalkKind::Todo, ""),
            entry("/root/plans/p.md", WalkKind::Plan, ""),
            entry("/root/settings.json", WalkKind::Settings, ""),
        ];
        let out = walk_filtered(&cfg, listing);
        assert!(out.is_empty());
    }

    #[test]
    fn sidecars_enabled_when_explicitly_set() {
        let cfg = WalkConfig {
            root: PathBuf::from("/root"),
            projects_only: false,
            sidecars: true,
            codex: false,
        };
        let listing = vec![
            entry("/root/projects/a/s.jsonl", WalkKind::Session, "a"),
            entry("/root/history.jsonl", WalkKind::GlobalHistory, ""),
            entry("/root/todos/a.json", WalkKind::Todo, ""),
            entry("/root/plans/p.md", WalkKind::Plan, ""),
            entry("/root/settings.json", WalkKind::Settings, ""),
        ];
        let out = walk_filtered(&cfg, listing);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn codex_entries_filtered_when_disabled() {
        let cfg = WalkConfig {
            root: PathBuf::from("/root"),
            projects_only: false,
            sidecars: true,
            codex: false,
        };
        let listing = vec![
            entry("/root/.codex/history.jsonl", WalkKind::CodexHistory, ""),
            entry("/root/.codex/sessions/a.jsonl", WalkKind::CodexSession, ""),
        ];
        let out = walk_filtered(&cfg, listing);
        assert!(out.is_empty());
    }

    #[test]
    fn codex_entries_admitted_when_enabled() {
        let cfg = WalkConfig {
            root: PathBuf::from("/root"),
            projects_only: false,
            sidecars: false,
            codex: true,
        };
        let listing = vec![
            entry("/root/.codex/history.jsonl", WalkKind::CodexHistory, ""),
            entry("/root/.codex/sessions/a.jsonl", WalkKind::CodexSession, ""),
        ];
        let out = walk_filtered(&cfg, listing);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn session_entries_outside_projects_dir_are_filtered_out() {
        let cfg = WalkConfig::projects_only("/root");
        let listing = vec![
            entry("/elsewhere/s.jsonl", WalkKind::Session, "x"),
            entry("/root/projects/a/s.jsonl", WalkKind::Session, "a"),
        ];
        let out = walk_filtered(&cfg, listing);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].project_dir, "a");
    }

    #[test]
    fn walk_returns_empty_when_root_is_missing() {
        let cfg = WalkConfig::projects_only(PathBuf::from(
            "/this/path/should/not/exist/cortex-walker-test",
        ));
        let out = walk(&cfg);
        assert!(out.is_empty());
    }

    #[test]
    fn codex_root_is_sibling_dot_codex() {
        let claude = PathBuf::from("/home/u/.claude");
        let codex = codex_root_for(&claude);
        assert_eq!(codex, PathBuf::from("/home/u/.codex"));
    }
}
