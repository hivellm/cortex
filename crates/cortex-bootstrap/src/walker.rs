//! File walker — uses the `ignore` crate so `.gitignore` is honoured
//! out of the box, then layers per-repo `cortex.toml` excludes plus the
//! 10 MB oversize gate from spec 09 §File walker.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::config::CortexSection;

/// Default oversize cap above which a file is dropped rather than
/// indexed. Mirrors [`crate::config::default_max_file_bytes`] —
/// kept here as a re-export so callers that historically read this
/// constant compile unchanged. Per-repo overrides ride
/// `cortex.toml`'s `[cortex.exclude].max_file_bytes`.
pub const MAX_FILE_BYTES: u64 = crate::config::DEFAULT_MAX_FILE_BYTES;

/// Classification of a walked file. Drives which `kind` the synthetic
/// event will carry downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    /// Source code — `artifact.code`.
    Code,
    /// Documentation (Markdown / reST / plain text) — `artifact.doc`.
    Doc,
    /// ADR / OpenSpec / decision — `decision.imported`.
    Decision,
    /// Law / rule file — `law.imported`.
    Law,
    /// Memory file — `memory.imported`.
    Memory,
    /// Anything else worth indexing as opaque text.
    Other,
}

/// One file produced by the walker. Either `Accepted` (size + class
/// resolved) or `Dropped` (filtered out with a reason for telemetry).
#[derive(Debug, Clone)]
pub enum WalkEntry {
    /// File survived all filters and is ready for emission.
    Accepted {
        /// Absolute filesystem path.
        path: PathBuf,
        /// Repo-rooted forward-slash path (the value carried in
        /// `source.path` of the synthetic envelope).
        rel_path: String,
        /// File size in bytes.
        size_bytes: u64,
        /// Classification driving the event kind.
        class: FileClass,
    },
    /// File was rejected; carries the reason used as a metric label.
    Dropped {
        /// Repo-rooted forward-slash path.
        rel_path: String,
        /// Drop reason: `oversize`, `extension`, `path_excluded`,
        /// `binary`, `not_a_file`.
        reason: &'static str,
    },
}

impl WalkEntry {
    /// `true` when the entry is an `Accepted` variant.
    pub fn is_accepted(&self) -> bool {
        matches!(self, WalkEntry::Accepted { .. })
    }

    /// Repo-rooted path (works for both variants).
    pub fn rel_path(&self) -> &str {
        match self {
            WalkEntry::Accepted { rel_path, .. } | WalkEntry::Dropped { rel_path, .. } => rel_path,
        }
    }
}

/// Walk `repo_root` honouring the `cortex.toml` body in `cfg`.
///
/// Returns one [`WalkEntry`] per file encountered. The walker
/// transparently honours `.gitignore` (via the `ignore` crate) and
/// adds:
///
/// - Per-repo path / extension exclusions (`cortex.exclude.*`).
/// - The 10 MB oversize gate (spec 09 §File walker).
/// - Classification via [`classify_path`].
pub fn walk_repo(repo_root: &Path, cfg: &CortexSection) -> Vec<WalkEntry> {
    let extension_drop: HashSet<String> = cfg
        .exclude
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();

    let mut overrides = OverrideBuilder::new(repo_root);
    for path in &cfg.exclude.paths {
        // `OverrideBuilder` uses gitignore-style negation: a leading
        // `!` excludes. We always want exclusion, so prefix.
        let pattern = format!("!{}", path.trim_end_matches('/'));
        // Trailing-glob to catch the directory and its descendants.
        let _ = overrides.add(&pattern);
        let descendants = format!("!{}/**", path.trim_end_matches('/'));
        let _ = overrides.add(&descendants);
    }
    let mut walker = WalkBuilder::new(repo_root);
    walker
        .standard_filters(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        // `hidden(false)` keeps dot-files in the walk so things like
        // `.cursor/rules/*.md` (spec 09 §Per-repo configuration) and
        // committed `.env` files reach the redactor. `.git/` itself
        // is still filtered via the default `cortex.exclude.paths`.
        .hidden(false);
    if let Ok(o) = overrides.build() {
        walker.overrides(o);
    }
    let walker = walker.build();

    let mut out = Vec::new();
    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == repo_root {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        let rel = match path.strip_prefix(repo_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_path = rel
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();

        // Extension drop.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if let Some(ref e) = ext {
            if extension_drop.contains(e) {
                out.push(WalkEntry::Dropped {
                    rel_path,
                    reason: "extension",
                });
                continue;
            }
        }

        // Oversize gate. The cap defaults to 8 MB (1 MB headroom
        // below Synap's 10 MB body limit for envelope wrapping +
        // JSON expansion); per-repo overrides land via
        // `cortex.toml`'s `[cortex.exclude].max_file_bytes`.
        let size_bytes = metadata.len();
        if size_bytes > cfg.exclude.max_file_bytes {
            out.push(WalkEntry::Dropped {
                rel_path,
                reason: "oversize",
            });
            continue;
        }

        let class = classify_path(&rel_path, cfg);
        out.push(WalkEntry::Accepted {
            path: path.to_path_buf(),
            rel_path,
            size_bytes,
            class,
        });
    }

    // Second pass: rescue files that the gitignore-aware walk would
    // have excluded but that the user explicitly opted in to via
    // `cortex.toml`'s `[cortex.decisions]`, `[cortex.laws]`, or
    // `[cortex.memories]` blocks. The default walk above respects
    // `.gitignore`, which in many Hive repos blanket-excludes
    // `.rulebook/*` while only whitelisting `specs/` and `tasks/` —
    // dropping ADRs, laws, and memory imports on the floor. This
    // rescue walk runs without gitignore filtering and only retains
    // paths that match a promote / import pattern.
    let already_seen: HashSet<String> = out
        .iter()
        .filter_map(|e| match e {
            WalkEntry::Accepted { rel_path, .. } => Some(rel_path.clone()),
            WalkEntry::Dropped { .. } => None,
        })
        .collect();
    let mut rescue = WalkBuilder::new(repo_root);
    rescue
        .standard_filters(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .hidden(false);
    for entry in rescue.build().flatten() {
        let path = entry.path();
        if path == repo_root {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(rel) = path.strip_prefix(repo_root) else {
            continue;
        };
        let rel_path = rel
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
        if already_seen.contains(&rel_path) {
            continue;
        }
        // Skip anything that doesn't match a promotion / import pattern
        // — the rescue walk should never re-include random files the
        // user did not opt in to.
        let promoted = matches_any(&cfg.decisions.promote_patterns, &rel_path)
            || matches_any(&cfg.laws.promote_patterns, &rel_path)
            || matches_any(&cfg.memories.import_files, &rel_path);
        if !promoted {
            continue;
        }
        // Honour the same extension drop + oversize gates as the main
        // walk so a 12 MB binary the user happened to whitelist
        // doesn't blow past spec 09's safety nets.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if let Some(ref e) = ext {
            if extension_drop.contains(e) {
                out.push(WalkEntry::Dropped {
                    rel_path,
                    reason: "extension",
                });
                continue;
            }
        }
        let size_bytes = metadata.len();
        if size_bytes > MAX_FILE_BYTES {
            out.push(WalkEntry::Dropped {
                rel_path,
                reason: "oversize",
            });
            continue;
        }
        let class = classify_path(&rel_path, cfg);
        out.push(WalkEntry::Accepted {
            path: path.to_path_buf(),
            rel_path,
            size_bytes,
            class,
        });
    }

    out
}

/// Classify a repo-relative path against the spec-09 promotion
/// patterns + extension heuristics.
///
/// Order: explicit promotions (decision / law / memory) win first,
/// then doc extensions, then code extensions, then `Other`.
pub fn classify_path(rel_path: &str, cfg: &CortexSection) -> FileClass {
    if matches_any(&cfg.decisions.promote_patterns, rel_path) {
        return FileClass::Decision;
    }
    if matches_any(&cfg.laws.promote_patterns, rel_path) {
        return FileClass::Law;
    }
    if matches_any(&cfg.memories.import_files, rel_path) {
        return FileClass::Memory;
    }
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" | "mdx" | "rst" | "txt" | "adoc" => FileClass::Doc,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cc" | "cpp"
        | "h" | "hpp" | "rb" | "ex" | "exs" | "kt" | "swift" | "scala" | "cs" | "php"
        | "json" | "yaml" | "yml" | "toml" | "sh" | "bash" | "zsh" | "fish" | "ps1" => {
            FileClass::Code
        }
        _ => FileClass::Other,
    }
}

/// Lightweight glob matcher — handles `*`, `**`, `?`, plus literal
/// path segments. The full `globset` machinery is overkill for the
/// few patterns spec 09 calls out and would pull in another crate.
pub fn matches_any(patterns: &[String], rel_path: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, rel_path))
}

/// Match a single glob pattern against a forward-slash path.
///
/// Supports `*` (matches any non-`/` chars), `**` (matches across
/// `/`), `?` (any single non-`/` char), and literal segments. Pure
/// recursive backtracker so the match logic is easy to audit; the
/// inputs we run it against (per-repo promote_patterns + import_files)
/// are tiny so the recursion depth stays bounded.
fn glob_match(pattern: &str, path: &str) -> bool {
    glob_recurse(pattern.as_bytes(), 0, path.as_bytes(), 0)
}

fn glob_recurse(p: &[u8], mut pi: usize, s: &[u8], mut si: usize) -> bool {
    while pi < p.len() {
        match p[pi] {
            b'*' => {
                if pi + 1 < p.len() && p[pi + 1] == b'*' {
                    // `**` matches any char (including `/`). Skip an
                    // optional trailing `/` so `**/x` and `**x` both
                    // line up.
                    let mut next_pi = pi + 2;
                    if next_pi < p.len() && p[next_pi] == b'/' {
                        next_pi += 1;
                    }
                    for try_si in si..=s.len() {
                        if glob_recurse(p, next_pi, s, try_si) {
                            return true;
                        }
                    }
                    return false;
                }
                // Single `*`: matches any run of non-`/` characters.
                let next_pi = pi + 1;
                if glob_recurse(p, next_pi, s, si) {
                    return true;
                }
                let mut try_si = si;
                while try_si < s.len() && s[try_si] != b'/' {
                    try_si += 1;
                    if glob_recurse(p, next_pi, s, try_si) {
                        return true;
                    }
                }
                return false;
            }
            b'?' => {
                if si >= s.len() || s[si] == b'/' {
                    return false;
                }
                pi += 1;
                si += 1;
            }
            c => {
                if si >= s.len() || s[si] != c {
                    return false;
                }
                pi += 1;
                si += 1;
            }
        }
    }
    si == s.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CortexSection;

    #[test]
    fn glob_matches_simple() {
        assert!(glob_match("*.md", "README.md"));
        assert!(glob_match("docs/*.md", "docs/intro.md"));
        assert!(!glob_match("docs/*.md", "docs/sub/intro.md"));
    }

    #[test]
    fn glob_matches_double_star_across_separators() {
        assert!(glob_match("docs/**/*.md", "docs/sub/intro.md"));
        assert!(glob_match("docs/**/*.md", "docs/intro.md"));
        assert!(glob_match("**/*.yaml", "rulebook/laws/LAW-007.yaml"));
    }

    #[test]
    fn classify_recognises_promotions() {
        let mut cfg = CortexSection::default();
        cfg.decisions.promote_patterns = vec!["docs/decisions/*.md".into(), "ADR-*.md".into()];
        cfg.laws.promote_patterns = vec!["rulebook/laws/*.yaml".into()];
        cfg.memories.import_files = vec!["CLAUDE.md".into()];
        assert_eq!(
            classify_path("docs/decisions/0042-adopt-meili.md", &cfg),
            FileClass::Decision
        );
        assert_eq!(
            classify_path("rulebook/laws/LAW-007.yaml", &cfg),
            FileClass::Law
        );
        assert_eq!(classify_path("CLAUDE.md", &cfg), FileClass::Memory);
        assert_eq!(classify_path("README.md", &cfg), FileClass::Doc);
        assert_eq!(classify_path("src/lib.rs", &cfg), FileClass::Code);
        // `.lock` lands in `Other` — defaults exclude it via the
        // walker's extension filter rather than indexing it as code.
        assert_eq!(classify_path("Cargo.lock", &cfg), FileClass::Other);
        assert_eq!(classify_path("Cargo.toml", &cfg), FileClass::Code);
    }
}
