//! Phase11k §3 — markdown analyzer.
//!
//! Produces graph edges from markdown prose:
//!
//! - **§3.2 links** — `[text](path)` resolved against the workspace
//!   root → [`EdgeType::LinksTo`] for `.md` targets,
//!   [`EdgeType::Documents`] for source-code targets, and
//!   [`EdgeType::LinksToSection`] when the URL has a fragment.
//! - **§3.3 sections** — every `#`-prefixed heading becomes a
//!   `:DocSection` node with the GitHub-flavoured slug as natural
//!   key; nested headings carry a [`EdgeType::Contains`] parent edge.
//! - **§3.4 mentions** — backtick-token spans (`` `Symbol` ``) emit
//!   one [`EdgeType::Mentions`] edge each, with the resolver's
//!   confidence stamped on the edge.
//! - **§3.5 fenced code** — first-line `// path/to/file.rs` (or `#`
//!   for Python, etc.) emits [`EdgeType::DescribesPath`] anchored on
//!   the surrounding section.
//!
//! The walker is a single pulldown-cmark pass that fans the events
//! into the per-feature submodules below; each submodule is
//! responsible for one edge class so the file stays small as the
//! graph evolves.

pub mod code_blocks;
pub mod links;
pub mod mentions;
pub mod sections;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::analyzer::{artifact_logical_key, CodeEdge, NodeRef, ResolutionTarget};

/// Stateless markdown analyzer.
#[derive(Debug, Default, Clone, Copy)]
pub struct MarkdownAnalyzer;

impl MarkdownAnalyzer {
    /// Construct.
    pub const fn new() -> Self {
        Self
    }

    /// Run all four sub-analyzers and return one flat
    /// [`Vec<CodeEdge>`] the patch builder consumes (the same shape
    /// `analyzer/rust.rs` produces). `repo` and `path` are the
    /// markdown file's repo + repo-relative path.
    pub fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge> {
        let parsed = parse(source);
        let mut out: Vec<CodeEdge> = Vec::new();
        out.extend(sections::extract(&parsed, repo, path));
        out.extend(links::extract(&parsed, repo, path));
        out.extend(mentions::extract(&parsed, repo, path));
        out.extend(code_blocks::extract(&parsed, repo, path));
        out
    }
}

/// Walked markdown event stream — a Vec the per-feature submodules
/// can iterate without re-parsing the source.
pub struct ParsedMarkdown<'a> {
    /// Original text (back-reference for offset → line lookups).
    pub source: &'a str,
    /// Each entry is `(event, byte_range)`. The `byte_range` carries
    /// the source span the event covers; `start.0` is the start byte
    /// of the event and `start.1` the end byte (exclusive).
    pub events: Vec<(Event<'a>, std::ops::Range<usize>)>,
}

impl ParsedMarkdown<'_> {
    /// Convert a byte offset back to a 1-indexed line number.
    pub fn line_at(&self, offset: usize) -> u32 {
        let bytes = self.source.as_bytes();
        let cap = offset.min(bytes.len());
        let mut line = 1u32;
        for &b in &bytes[..cap] {
            if b == b'\n' {
                line = line.saturating_add(1);
            }
        }
        line
    }
}

/// Parse markdown source into the walker-friendly event vector
/// shared by every submodule.
pub fn parse(source: &str) -> ParsedMarkdown<'_> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(source, opts);
    let events: Vec<(Event<'_>, std::ops::Range<usize>)> =
        parser.into_offset_iter().map(|(ev, range)| (ev, range)).collect();
    ParsedMarkdown { source, events }
}

/// Build a `:DocSection` node ref for a heading inside `path` whose
/// slug is `slug`. Natural key is `repo|path#slug`.
pub fn doc_section_node(repo: &str, path: &str, slug: &str) -> NodeRef {
    NodeRef {
        label: "DocSection".into(),
        natural_key: format!("{repo}|{path}#{slug}"),
    }
}

/// Build the `:Artifact` node ref for the markdown source artifact
/// itself — same logical key the code analyzers use.
pub fn markdown_source_node(repo: &str, path: &str) -> NodeRef {
    NodeRef {
        label: "Artifact".into(),
        natural_key: artifact_logical_key(repo, path),
    }
}

/// True when `path` ends with a markdown extension.
pub fn is_markdown_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// True when `path` ends with one of the recognised source-code
/// extensions (mirrors
/// [`crate::graph::analyzer::AnalyzerLanguage::from_path_extension`]
/// plus a couple of plain-text descendants the doc layer should
/// still link to).
pub fn is_source_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    matches!(
        lower.rsplit_once('.').map(|(_, ext)| ext),
        Some("rs")
            | Some("ts")
            | Some("tsx")
            | Some("js")
            | Some("py")
            | Some("go")
            | Some("java")
            | Some("c")
            | Some("h")
            | Some("cpp")
            | Some("hpp")
            | Some("toml")
            | Some("yaml")
            | Some("yml")
            | Some("json")
    )
}

/// Walk `events` and pair each `Tag::Heading` with the inline text
/// up to its matching close. Used by §3.3 (heading → DocSection) and
/// by §3.5 (the section a code block sits inside).
pub fn each_heading<F: FnMut(&str, u32, u32, std::ops::Range<usize>)>(
    parsed: &ParsedMarkdown<'_>,
    mut callback: F,
) {
    let events = &parsed.events;
    let mut i = 0;
    while i < events.len() {
        if let (Event::Start(Tag::Heading { level, .. }), open_range) =
            (&events[i].0, events[i].1.clone())
        {
            let depth = *level as u32;
            let mut text_buf = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j].0 {
                    Event::Text(t) | Event::Code(t) => text_buf.push_str(t.as_ref()),
                    Event::End(TagEnd::Heading(_)) => break,
                    _ => {}
                }
                j += 1;
            }
            let line = parsed.line_at(open_range.start);
            callback(&text_buf, depth, line, open_range);
            i = j;
        }
        i += 1;
    }
}

/// GitHub-flavoured heading slugger — matches the algorithm rustdoc /
/// docs.rs / GitHub all use: lowercase, drop punctuation, replace
/// runs of whitespace with a single dash.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_dash = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for low in ch.to_lowercase() {
                out.push(low);
            }
            prev_dash = false;
        } else if ch.is_whitespace() || ch == '-' || ch == '_' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Resolve a relative link target against the markdown source
/// artifact's `path`. Returns the workspace-relative form of the
/// target — `../foo/bar.md` resolved against `docs/sub.md` becomes
/// `docs/foo/bar.md`. Absolute paths and external URLs (`https://`)
/// pass through unchanged.
pub fn resolve_relative_link(source_path: &str, link: &str) -> String {
    if link.starts_with("http://")
        || link.starts_with("https://")
        || link.starts_with("mailto:")
    {
        return link.to_string();
    }
    let mut parts: Vec<&str> = source_path.split('/').collect();
    if !parts.is_empty() {
        parts.pop();
    }
    for segment in link.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<&str>>()
        .join("/")
}

/// Logical link used by §3 edges that point at a "logical" Artifact
/// (just `repo|path` — the patch builder canonicalises the
/// content_hash slot).
pub fn logical_artifact_target(repo: &str, path: &str) -> ResolutionTarget {
    let parts: Vec<String> = path
        .split('/')
        .map(|p| p.to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        ResolutionTarget::SymbolName(repo.to_string())
    } else {
        ResolutionTarget::ModulePath(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_github_rules() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("§3.5  Output"), "35-output");
        assert_eq!(slugify("foo & bar"), "foo-bar");
        assert_eq!(slugify("LEAD-WITH-DASH"), "lead-with-dash");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn relative_link_resolves_against_source_path() {
        assert_eq!(
            resolve_relative_link("docs/foo/index.md", "../bar.md"),
            "docs/bar.md"
        );
        assert_eq!(
            resolve_relative_link("docs/index.md", "./crates/lib.rs"),
            "docs/crates/lib.rs"
        );
        assert_eq!(
            resolve_relative_link("README.md", "https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn is_markdown_path_matches_extensions() {
        assert!(is_markdown_path("docs/spec.md"));
        assert!(is_markdown_path("README.MARKDOWN"));
        assert!(!is_markdown_path("src/lib.rs"));
    }

    #[test]
    fn is_source_path_matches_recognised_extensions() {
        assert!(is_source_path("crates/foo/src/lib.rs"));
        assert!(is_source_path("gui/src/app.tsx"));
        assert!(is_source_path("Cargo.toml"));
        assert!(!is_source_path("docs/spec.md"));
    }

    #[test]
    fn each_heading_yields_text_and_depth() {
        let parsed = parse("# Title\n## Sub\nbody\n");
        let mut hits: Vec<(String, u32)> = Vec::new();
        each_heading(&parsed, |text, depth, _line, _range| {
            hits.push((text.to_string(), depth));
        });
        assert_eq!(hits, vec![("Title".into(), 1), ("Sub".into(), 2)]);
    }
}
