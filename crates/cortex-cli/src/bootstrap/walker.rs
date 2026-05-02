//! File walker — uses the `ignore` crate so `.gitignore` is honoured
//! out of the box, then layers per-repo `cortex.toml` excludes plus the
//! 10 MB oversize gate from spec 09 §File walker.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use super::config::CortexSection;

/// Default oversize cap above which a file is dropped rather than
/// indexed. Mirrors [`super::config::default_max_file_bytes`] —
/// kept here as a re-export so callers that historically read this
/// constant compile unchanged. Per-repo overrides ride
/// `cortex.toml`'s `[cortex.exclude].max_file_bytes`.
pub const MAX_FILE_BYTES: u64 = super::config::DEFAULT_MAX_FILE_BYTES;

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
    /// Audit / deep-analysis report — `analysis.imported`. Lives
    /// under `docs/analysis/**/*.md` by convention; the explicit
    /// `[cortex.analyses].promote_patterns` opts a path in.
    Analysis,
    /// phase10e — pattern / anti-pattern entry imported from
    /// `.rulebook/knowledge/**/*.md` (Rulebook MCP
    /// `rulebook_knowledge_add`).
    Knowledge,
    /// phase10e — implementation insight imported from
    /// `.rulebook/learnings/**/*.md` (Rulebook MCP
    /// `rulebook_learn_capture`).
    Learning,
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
        // user did not opt in to. Per-repo cortex.toml patterns are
        // checked first; canonical `.rulebook/*` defaults catch the
        // common case where a sibling repo carries the standard
        // rulebook layout but has no cortex.toml of its own (Nexus /
        // Vectorizer / Rulebook / Synap as of 2026-04-28 — the user
        // reported their decisions / knowledge / learnings never
        // surfaced in discovery).
        let promoted = matches_any(&cfg.decisions.promote_patterns, &rel_path)
            || matches_any(&cfg.laws.promote_patterns, &rel_path)
            || matches_any(&cfg.analyses.promote_patterns, &rel_path)
            || matches_any(&cfg.memories.import_files, &rel_path)
            || matches_str_globs(RULEBOOK_DECISION_GLOBS, &rel_path)
            || matches_str_globs(RULEBOOK_LAW_GLOBS, &rel_path)
            || matches_str_globs(RULEBOOK_KNOWLEDGE_GLOBS, &rel_path)
            || matches_str_globs(RULEBOOK_LEARNING_GLOBS, &rel_path)
            || matches_str_globs(RULEBOOK_MEMORY_GLOBS, &rel_path);
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

/// Canonical `.rulebook/*` patterns that ALWAYS promote, regardless
/// of whether the repo ships a `cortex.toml`. Sibling Hive repos
/// (Nexus, Vectorizer, Rulebook, Synap, …) don't carry per-repo
/// cortex.toml today, so the rescue walk's `promoted` check returned
/// false for every `.rulebook/decisions/*.md` they own — silently
/// dropping their ADRs / knowledge / learnings on the floor. Hardcoding
/// the canonical layout makes discovery work out-of-the-box for every
/// rulebook-managed repo.
///
/// The order of this list is significant: `classify_path` and the
/// rescue walk read it top-to-bottom, so subdirectories that should
/// be `Decision` must precede the catch-all `.rulebook/**` patterns.
pub const RULEBOOK_DECISION_GLOBS: &[&str] =
    &[".rulebook/decisions/*.md", ".rulebook/decisions/**/*.md"];

/// Canonical glob list for spec docs that should fan out into
/// per-`## ` law envelopes via the emitter's
/// `emit_spec_laws_imported` path. `.rulebook/specs/**/*.md` is
/// the canonical Hive convention; the emitter splits each one by
/// top-level heading so the dashboard's `laws_active` overlay
/// finally has rules to surface (closes the 2026-04-27 audit's
/// `laws_active = 0` finding for spec-driven projects).
pub const RULEBOOK_LAW_GLOBS: &[&str] = &[".rulebook/specs/**/*.md"];

/// phase10e — knowledge globs route to `FileClass::Knowledge`
/// (separate kind, separate Vectorizer collection
/// `cortex.knowledge.fp32`, separate Meili index
/// `cortex_knowledge`). Pre-phase10e these were lumped under
/// `RULEBOOK_MEMORY_GLOBS` so they piled into the catch-all
/// memory bucket; the orchestrator could not surface them
/// distinctly on `pre_change_context` / `decision_lookup`.
pub const RULEBOOK_KNOWLEDGE_GLOBS: &[&str] = &[".rulebook/knowledge/**/*.md"];

/// phase10e — learnings globs route to `FileClass::Learning`
/// (`cortex.learning.fp32` + `cortex_learnings`). Same rationale
/// as [`RULEBOOK_KNOWLEDGE_GLOBS`].
pub const RULEBOOK_LEARNING_GLOBS: &[&str] = &[".rulebook/learnings/**/*.md"];

/// Canonical glob list for `.rulebook` memory-shaped artifacts:
/// handoff snapshots and the loose top-level memory files
/// (PLANS.md / STATE.md / CLAUDE.md imports). Knowledge +
/// learnings used to ride here pre-phase10e — they now have
/// their own globs above and dedicated kinds.
pub const RULEBOOK_MEMORY_GLOBS: &[&str] = &[
    // Handoff snapshots — `_pending.md` rotates as the active
    // hand-off, archived ones live alongside. The dashboard surfaces
    // these per-project so a user resuming a session can pull the
    // last hand-off without grepping every repo by hand.
    ".rulebook/handoff/**/*.md",
    ".rulebook/PLANS.md",
    ".rulebook/STATE.md",
    ".rulebook/COMPACT_CONTEXT.md",
];

/// Classify a repo-relative path against the spec-09 promotion
/// patterns + extension heuristics.
///
/// Order: explicit promotions (decision / law / memory) win first,
/// then the canonical `.rulebook/*` defaults, then doc extensions,
/// then code extensions, then `Other`.
pub fn classify_path(rel_path: &str, cfg: &CortexSection) -> FileClass {
    if matches_any(&cfg.decisions.promote_patterns, rel_path) {
        return FileClass::Decision;
    }
    if matches_any(&cfg.laws.promote_patterns, rel_path) {
        return FileClass::Law;
    }
    if matches_any(&cfg.analyses.promote_patterns, rel_path) {
        return FileClass::Analysis;
    }
    if matches_any(&cfg.memories.import_files, rel_path) {
        return FileClass::Memory;
    }
    // Built-in `.rulebook/*` defaults — apply to every repo whether
    // or not it ships a cortex.toml. Decisions first so a path like
    // `.rulebook/decisions/001-foo.md` lands as Decision rather than
    // being caught by the broader memory globs.
    if matches_str_globs(RULEBOOK_DECISION_GLOBS, rel_path) {
        return FileClass::Decision;
    }
    // Spec docs route to Law so the emitter's spec-splitter fans
    // them out into per-`## ` law envelopes. Order matters: this
    // MUST sit before the memory globs because earlier revisions
    // included `.rulebook/specs/**` in the memory list.
    if matches_str_globs(RULEBOOK_LAW_GLOBS, rel_path) {
        return FileClass::Law;
    }
    // phase10e — knowledge / learnings get their own classes
    // before the memory fallback so the canonical
    // `.rulebook/knowledge/**` and `.rulebook/learnings/**`
    // hierarchies route to the dedicated downstream collections.
    if matches_str_globs(RULEBOOK_KNOWLEDGE_GLOBS, rel_path) {
        return FileClass::Knowledge;
    }
    if matches_str_globs(RULEBOOK_LEARNING_GLOBS, rel_path) {
        return FileClass::Learning;
    }
    if matches_str_globs(RULEBOOK_MEMORY_GLOBS, rel_path) {
        return FileClass::Memory;
    }
    let ext = std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" | "mdx" | "rst" | "txt" | "adoc" => FileClass::Doc,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "vue" | "py" | "go" | "java" | "c" | "cc" | "cpp"
        | "h" | "hpp" | "rb" | "ex" | "exs" | "kt" | "swift" | "scala" | "cs" | "php" | "json"
        | "yaml" | "yml" | "toml" | "sh" | "bash" | "zsh" | "fish" | "ps1" => FileClass::Code,
        _ => FileClass::Other,
    }
}

/// Lightweight glob matcher — handles `*`, `**`, `?`, plus literal
/// path segments. The full `globset` machinery is overkill for the
/// few patterns spec 09 calls out and would pull in another crate.
pub fn matches_any(patterns: &[String], rel_path: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, rel_path))
}

/// `matches_any` for `&[&str]` — used by the canonical `.rulebook/*`
/// default patterns which live in `&'static [&'static str]` form so
/// they don't need to allocate `String`s on every call.
pub fn matches_str_globs(patterns: &[&str], rel_path: &str) -> bool {
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
    use crate::bootstrap::config::CortexSection;

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
        cfg.analyses.promote_patterns = vec!["docs/analysis/**/*.md".into()];
        cfg.memories.import_files = vec!["CLAUDE.md".into()];
        assert_eq!(
            classify_path("docs/decisions/0042-adopt-meili.md", &cfg),
            FileClass::Decision
        );
        assert_eq!(
            classify_path("rulebook/laws/LAW-007.yaml", &cfg),
            FileClass::Law
        );
        assert_eq!(
            classify_path("docs/analysis/cortex/01-overview.md", &cfg),
            FileClass::Analysis
        );
        assert_eq!(
            classify_path("docs/analysis/2026-04-28-audit.md", &cfg),
            FileClass::Analysis
        );
        assert_eq!(classify_path("CLAUDE.md", &cfg), FileClass::Memory);
        assert_eq!(classify_path("README.md", &cfg), FileClass::Doc);
        assert_eq!(classify_path("src/lib.rs", &cfg), FileClass::Code);
        // `.lock` lands in `Other` — defaults exclude it via the
        // walker's extension filter rather than indexing it as code.
        assert_eq!(classify_path("Cargo.lock", &cfg), FileClass::Other);
        assert_eq!(classify_path("Cargo.toml", &cfg), FileClass::Code);
    }

    #[test]
    fn classify_picks_up_canonical_rulebook_layout_without_cortex_toml() {
        // Nexus / Vectorizer / Rulebook / Synap don't ship cortex.toml
        // today — `cfg` here mirrors a default-constructed config with
        // no per-repo promote patterns. The canonical `.rulebook/*`
        // layout MUST still classify correctly so discovery stops
        // dropping their ADRs / knowledge / learnings on the floor.
        let cfg = CortexSection::default();

        // Decisions: top-level + nested.
        assert_eq!(
            classify_path(".rulebook/decisions/001-adopt-foo.md", &cfg),
            FileClass::Decision
        );
        assert_eq!(
            classify_path(".rulebook/decisions/sub/002-bar.md", &cfg),
            FileClass::Decision
        );

        // phase10e — knowledge + learnings now route to dedicated
        // FileClass variants (and downstream kinds) so the
        // orchestrator can fan out to them on
        // `pre_change_context` / `decision_lookup` without
        // diluting the catch-all memory bucket.
        assert_eq!(
            classify_path(".rulebook/knowledge/patterns/foo.md", &cfg),
            FileClass::Knowledge
        );
        assert_eq!(
            classify_path(".rulebook/knowledge/anti-patterns/bar.md", &cfg),
            FileClass::Knowledge
        );
        assert_eq!(
            classify_path(".rulebook/learnings/2026-04-27-baz.md", &cfg),
            FileClass::Learning
        );

        // Specs route to Law so the emitter's spec-splitter fans
        // them out into per-`## ` law envelopes — closes the
        // dashboard's `laws_active = 0` gap.
        assert_eq!(
            classify_path(".rulebook/specs/RULEBOOK.md", &cfg),
            FileClass::Law
        );
        assert_eq!(
            classify_path(".rulebook/specs/sub/sub.md", &cfg),
            FileClass::Law
        );

        // Handoff snapshots — both the active `_pending.md` and
        // archived ones must classify as Memory.
        assert_eq!(
            classify_path(".rulebook/handoff/_pending.md", &cfg),
            FileClass::Memory
        );
        assert_eq!(
            classify_path(".rulebook/handoff/archived/2026-04-27.md", &cfg),
            FileClass::Memory
        );

        // Loose top-level memory files.
        assert_eq!(classify_path(".rulebook/PLANS.md", &cfg), FileClass::Memory);
        assert_eq!(classify_path(".rulebook/STATE.md", &cfg), FileClass::Memory);

        // Sanity: a random `.rulebook/random/foo.md` not under a
        // canonical subdir should NOT be promoted — discovery only
        // promotes the documented layout, not arbitrary content.
        assert_eq!(
            classify_path(".rulebook/random/foo.md", &cfg),
            FileClass::Doc,
            "non-canonical .rulebook paths fall through to extension routing"
        );
    }

    #[test]
    fn explicit_promote_patterns_still_take_precedence_over_defaults() {
        // A repo that promotes `.rulebook/learnings/*.md` to Decision
        // (unusual but legal) should win over the default Memory
        // routing — explicit > defaults.
        let mut cfg = CortexSection::default();
        cfg.decisions.promote_patterns = vec![".rulebook/learnings/*.md".into()];
        assert_eq!(
            classify_path(".rulebook/learnings/x.md", &cfg),
            FileClass::Decision,
            "explicit cortex.toml promotion overrides built-in defaults"
        );
    }
}
