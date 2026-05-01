//! Scope derivation — maps `(user_prompt, cwd, recent_files)` →
//! `cortex_api::Scope`. Spec 12 §scope_derive.
//!
//! The derivation is deliberately cheap: walk ancestors for `.git`,
//! optionally read a `cortex.toml` `cortex.id`, then merge the
//! `recent_files` whose `age_seconds < 5 min` with the file paths
//! mentioned verbatim in the user prompt (regex-bounded to 16
//! candidates so a hostile prompt can't blow the bundle).

use std::path::{Path, PathBuf};

use cortex_api::Scope;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Spec-12 cap on prompt-extracted file mentions.
pub const MAX_PROMPT_FILES: usize = 16;
/// `recent_files` age cut-off in seconds — spec 12 calls for 5 min.
pub const RECENT_FILE_MAX_AGE_SECS: u64 = 300;

/// File status echoed from the adapter side. Mirrors the spec-12
/// `RecentFile.status` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// Modified working-tree file.
    Modified,
    /// Staged file.
    Staged,
    /// Untracked file inside the repo.
    Untracked,
}

/// One recent file the adapter passes through from `git status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFile {
    /// Absolute path.
    pub path: PathBuf,
    /// `Modified` / `Staged` / `Untracked`.
    pub status: FileStatus,
    /// Age in seconds since the last `git status` snapshot.
    pub age_seconds: u64,
}

/// Outcome of the scope-derivation pass.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedScope {
    /// `cortex_api::Scope` ready to attach to a `QueryRequest`.
    pub scope: Scope,
    /// `repo` was resolved by walking ancestors. Surfaced for
    /// observability so we can spot un-resolved repos in audit.
    pub repo_resolved: bool,
}

/// Derive the scope from the inputs. Pure function — no I/O beyond
/// reading `cortex.toml` if it exists.
pub fn derive(user_prompt: &str, cwd: &Path, recent_files: &[RecentFile]) -> DerivedScope {
    let repo = repo_from_cwd(cwd);
    let repo_resolved = repo.is_some();

    let mut files: Vec<String> = Vec::new();
    for rf in recent_files {
        if rf.age_seconds >= RECENT_FILE_MAX_AGE_SECS {
            continue;
        }
        if let Some(rel) = relative_to_repo(repo.as_deref(), cwd, &rf.path) {
            push_unique(&mut files, rel);
        }
    }
    for mention in extract_prompt_files(user_prompt) {
        push_unique(&mut files, mention);
        if files.len() >= MAX_PROMPT_FILES {
            break;
        }
    }
    files.truncate(MAX_PROMPT_FILES);

    // phase10h §4.1 — derive topics from the file extensions the
    // walker already produced. The classifier stamps the same
    // canonical topic vocabulary on every event, so a topic
    // inference here scopes the orchestrator's lane filter to
    // the corpus the user is most likely asking about. Recent
    // files first (the user is editing them right now), then
    // prompt-mentioned paths.
    let mut topics: Vec<String> = Vec::new();
    for rf in recent_files {
        if rf.age_seconds >= RECENT_FILE_MAX_AGE_SECS {
            continue;
        }
        if let Some(topic) = topic_for_path(&rf.path.to_string_lossy()) {
            push_unique(&mut topics, topic.to_string());
        }
    }
    for path in &files {
        if let Some(topic) = topic_for_path(path) {
            push_unique(&mut topics, topic.to_string());
        }
    }

    DerivedScope {
        scope: Scope {
            repo,
            files,
            topics,
            since: None,
            ..Scope::default()
        },
        repo_resolved,
    }
}

/// phase10h — file-extension → canonical topic mapping. The
/// classifier vocabulary in
/// [`crates/cortex-classifier/src/statics.rs`](../../cortex-classifier/src/statics.rs)
/// is the source of truth; this map mirrors the most common
/// subset so the pre-thinking bundle's topic filter matches
/// real indexed events. Returns `None` for unknown / extensionless
/// paths so the caller treats topic inference as best-effort.
pub fn topic_for_path(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cc" | "cpp" | "h"
        | "hpp" | "rb" | "kt" | "swift" | "scala" | "cs" | "php" | "sh" | "bash" | "zsh"
        | "fish" | "ps1" | "sql" | "proto" => "code",
        "md" | "mdx" | "rst" | "adoc" | "asciidoc" | "txt" | "tex" | "org" => "docs",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" => "config",
        _ => return None,
    })
}

/// Resolve the nearest `.git/` ancestor to a repo identifier. Reads
/// `cortex.toml` (`cortex.id` override) when present.
pub fn repo_from_cwd(cwd: &Path) -> Option<String> {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            // `cortex.toml` override has the final word.
            if let Some(id) = read_cortex_id(dir) {
                return Some(id);
            }
            return dir
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
        }
        current = dir.parent();
    }
    None
}

fn read_cortex_id(repo_root: &Path) -> Option<String> {
    let path = repo_root.join("cortex.toml");
    if !path.exists() {
        return None;
    }
    let body = std::fs::read_to_string(&path).ok()?;
    // Lightweight scan for `id = "..."` under `[cortex]` so we don't
    // re-pull the toml dep just for one field.
    let mut in_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == "[cortex]";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("id") {
            let rest = rest.trim();
            let value = rest.trim_start_matches('=').trim();
            let value = value.trim_matches(|c: char| c == '"' || c.is_whitespace());
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn relative_to_repo(repo: Option<&str>, cwd: &Path, file: &Path) -> Option<String> {
    // The spec-11 `Scope.files` carries forward-slash repo-relative
    // paths. We try the repo-root path first (walked from `cwd`), and
    // fall back to a literal path string when no repo is available.
    let repo_root = if repo.is_some() {
        ancestor_with_git(cwd)
    } else {
        None
    };
    let rel = if let Some(root) = repo_root {
        file.strip_prefix(&root).ok().map(|p| p.to_path_buf())
    } else {
        None
    };
    let path = rel.unwrap_or_else(|| file.to_path_buf());
    Some(path.to_string_lossy().replace('\\', "/"))
}

fn ancestor_with_git(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Pull file-shaped tokens out of the user prompt. Conservative —
/// matches `ext`-suffixed paths so plain English words like "code"
/// don't trip the heuristic.
pub fn extract_prompt_files(prompt: &str) -> Vec<String> {
    // `[A-Za-z0-9_./-]+\.[A-Za-z0-9]+` — at least one path component
    // plus a literal extension. Anchored on word boundaries so
    // backticks and parentheses don't pollute the match.
    let pattern = Regex::new(r"\b[A-Za-z0-9_./-]+\.[A-Za-z0-9]{1,8}\b").expect("static regex");
    let mut out: Vec<String> = Vec::new();
    for cap in pattern.find_iter(prompt) {
        let s = cap.as_str();
        // Drop pure version-number-shaped matches like `1.2.3` and
        // `v1.0` so prompts that mention semver don't pollute the
        // file list.
        if s.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == 'v' || c == 'V')
        {
            continue;
        }
        push_unique(&mut out, s.replace('\\', "/"));
        if out.len() >= MAX_PROMPT_FILES {
            break;
        }
    }
    out
}

fn push_unique(buf: &mut Vec<String>, s: String) {
    if !buf.iter().any(|x| x == &s) {
        buf.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn extract_prompt_files_finds_path_like_tokens() {
        let prompt = "refactor src/index/hnsw/mod.rs to take ef per call. See docs/perf/hnsw.md.";
        let files = extract_prompt_files(prompt);
        assert!(files.contains(&"src/index/hnsw/mod.rs".to_string()));
        assert!(files.contains(&"docs/perf/hnsw.md".to_string()));
    }

    #[test]
    fn extract_prompt_files_ignores_pure_version_numbers() {
        let prompt = "bump tokio 1.40.1 then patch v1.0";
        let files = extract_prompt_files(prompt);
        assert!(files.is_empty(), "got {files:?}");
    }

    #[test]
    fn extract_prompt_files_caps_at_sixteen() {
        let mut prompt = String::new();
        for i in 0..30 {
            prompt.push_str(&format!("path{i}.rs "));
        }
        let files = extract_prompt_files(&prompt);
        assert_eq!(files.len(), MAX_PROMPT_FILES);
    }

    #[test]
    fn repo_from_cwd_uses_directory_name_when_no_cortex_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("Vectorizer");
        fs::create_dir_all(repo_root.join(".git")).unwrap();
        let nested = repo_root.join("src/index/hnsw");
        fs::create_dir_all(&nested).unwrap();
        let resolved = repo_from_cwd(&nested);
        assert_eq!(resolved.as_deref(), Some("Vectorizer"));
    }

    #[test]
    fn repo_from_cwd_honours_cortex_toml_override() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("AnyDir");
        fs::create_dir_all(repo_root.join(".git")).unwrap();
        fs::write(
            repo_root.join("cortex.toml"),
            "[cortex]\nid = \"OverrideId\"\n",
        )
        .unwrap();
        assert_eq!(repo_from_cwd(&repo_root).as_deref(), Some("OverrideId"));
    }

    #[test]
    fn derive_drops_aged_recent_files_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("R");
        fs::create_dir_all(repo_root.join(".git")).unwrap();
        let recent_a = repo_root.join("src/lib.rs");
        let recent_b = repo_root.join("src/main.rs");
        let recent = vec![
            RecentFile {
                path: recent_a.clone(),
                status: FileStatus::Modified,
                age_seconds: 60,
            },
            RecentFile {
                path: recent_b.clone(),
                status: FileStatus::Modified,
                age_seconds: 60,
            },
            RecentFile {
                path: recent_a.clone(),
                status: FileStatus::Modified,
                age_seconds: 60,
            },
            RecentFile {
                path: repo_root.join("old.rs"),
                status: FileStatus::Modified,
                age_seconds: 9999,
            },
        ];
        let scope = derive("touch src/lib.rs", &repo_root, &recent);
        assert!(scope.repo_resolved);
        assert_eq!(scope.scope.repo.as_deref(), Some("R"));
        // src/lib.rs from recent_files + the verbatim mention
        // dedupe to a single entry; old.rs is pruned by age.
        assert_eq!(scope.scope.files.len(), 2);
        assert!(scope.scope.files.iter().any(|f| f == "src/lib.rs"));
        assert!(scope.scope.files.iter().any(|f| f == "src/main.rs"));
    }

    // ---- phase10h — file-extension topic inference ----

    #[test]
    fn topic_for_path_maps_canonical_extensions() {
        assert_eq!(topic_for_path("src/lib.rs"), Some("code"));
        assert_eq!(topic_for_path("docs/specs/11-query-api.md"), Some("docs"));
        assert_eq!(topic_for_path("Cargo.toml"), Some("config"));
        assert_eq!(topic_for_path("data.json"), Some("config"));
        // Unknown / extensionless paths return None — caller
        // treats topic inference as best-effort.
        assert_eq!(topic_for_path("README"), None);
        assert_eq!(topic_for_path(""), None);
        assert_eq!(topic_for_path("artifact.unknown_ext"), None);
    }

    #[test]
    fn derive_stamps_topics_from_recent_files_and_prompt_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("R");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        let recent = vec![RecentFile {
            path: repo_root.join("src/lib.rs"),
            status: FileStatus::Modified,
            age_seconds: 60,
        }];
        let scope = derive("look at docs/specs/11-query-api.md", &repo_root, &recent);
        assert!(
            scope.scope.topics.contains(&"code".to_string()),
            "recent .rs file must add `code`; got {:?}",
            scope.scope.topics
        );
        assert!(
            scope.scope.topics.contains(&"docs".to_string()),
            "prompt-mentioned .md must add `docs`; got {:?}",
            scope.scope.topics
        );
    }
}
