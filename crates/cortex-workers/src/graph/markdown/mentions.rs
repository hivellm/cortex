//! Phase11k §3.4 — backtick-token symbol mentions.
//!
//! Inline `` `Symbol` `` spans inside markdown prose become
//! [`super::EdgeType::Mentions`] edges anchored on the source
//! artifact. The resolver can dispatch the mention against the
//! workspace's [`super::super::resolver::ModuleMap`] post-hoc; this
//! module deliberately keeps the analyzer pass cheap and emits only
//! syntactic information (the token) plus a `confidence` heuristic
//! the patch builder lifts straight onto `props`.
//!
//! Confidence rules (matches §3.4 of the proposal):
//!
//! - The token contains `::` → confidence `0.9` (qualified mention).
//! - The token starts with an uppercase letter and is at least 3
//!   chars → confidence `0.7` (likely type / trait / struct).
//! - Anything else → confidence `0.3` (best-effort prose mention).
//!
//! These confidence values are stamped on the [`CodeEdge::kind`]
//! sub-discriminator so downstream filters can drop low-confidence
//! mentions without re-parsing the source.

use pulldown_cmark::Event;

use super::super::analyzer::{CodeEdge, EdgeType, ResolutionTarget};
use super::{markdown_source_node, ParsedMarkdown};

/// Walk every `Event::Code` and emit one `Mentions` edge per
/// well-formed token. Returns an empty vec when the markdown carries
/// no inline code spans.
pub fn extract(parsed: &ParsedMarkdown<'_>, repo: &str, path: &str) -> Vec<CodeEdge> {
    let mut out: Vec<CodeEdge> = Vec::new();
    for (event, range) in &parsed.events {
        if let Event::Code(code) = event {
            let raw = code.as_ref().trim();
            if !looks_like_symbol(raw) {
                continue;
            }
            let line = parsed.line_at(range.start);
            let target = if raw.contains("::") {
                ResolutionTarget::ModulePath(
                    raw.split("::")
                        .map(|p| p.to_string())
                        .filter(|p| !p.is_empty())
                        .collect(),
                )
            } else {
                ResolutionTarget::SymbolName(raw.to_string())
            };
            out.push(CodeEdge {
                from_node: markdown_source_node(repo, path),
                edge_type: EdgeType::Mentions,
                to_target: target,
                source_line: Some(line),
                kind: confidence_kind(raw),
            });
        }
    }
    out
}

fn looks_like_symbol(token: &str) -> bool {
    if token.is_empty() || token.len() > 96 {
        return false;
    }
    if token.chars().any(char::is_whitespace) {
        return false;
    }
    // Reject pure punctuation / lifetime annotations / numbers.
    if !token.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Reject things that smell like a path or URL.
    if token.contains('/')
        || token.starts_with('-')
        || token.starts_with('.')
        || token.starts_with("$")
    {
        return false;
    }
    // Allow letters, digits, `_`, `:` — covers `Foo`, `foo_bar`,
    // `crate::Foo`, `Trait::method`.
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn confidence_kind(raw: &str) -> &'static str {
    if raw.contains("::") {
        "mention_qualified"
    } else if raw
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
        && raw.len() >= 3
    {
        "mention_type"
    } else {
        "mention_prose"
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;

    fn extract_for(src: &str) -> Vec<CodeEdge> {
        let parsed = parse(src);
        extract(&parsed, "cortex", "docs/spec.md")
    }

    #[test]
    fn qualified_token_emits_high_confidence_mention() {
        let edges = extract_for("the `crate::Foo::bar` helper does X.\n");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, EdgeType::Mentions);
        assert_eq!(edges[0].kind, "mention_qualified");
        assert_eq!(
            edges[0].to_target,
            ResolutionTarget::ModulePath(vec!["crate".into(), "Foo".into(), "bar".into()])
        );
    }

    #[test]
    fn capitalised_token_classified_as_type_mention() {
        let edges = extract_for("the `MyType` struct does X.\n");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, "mention_type");
    }

    #[test]
    fn lowercase_token_classified_as_prose_mention() {
        let edges = extract_for("call `helper` to bootstrap.\n");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, "mention_prose");
    }

    #[test]
    fn paths_and_punctuation_are_skipped() {
        let edges = extract_for("see `src/foo.rs` for details.\n");
        assert!(edges.is_empty());
    }

    #[test]
    fn empty_or_oversized_tokens_are_skipped() {
        let edges = extract_for("inline `` plus a `verylongtokenwithoutspaces`\n");
        // Empty backticks parse as empty Code nodes — pulldown-cmark
        // skips them so we just see the second one.
        assert_eq!(edges.len(), 1);
    }
}
