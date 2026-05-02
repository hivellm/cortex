//! Phase11k §3.2 — markdown link extraction.
//!
//! `[text](target)` resolves to one of:
//!
//! - `[text](path#anchor)` → [`super::EdgeType::LinksToSection`]
//!   pointing at the `:DocSection` keyed on `{repo}|{path}#{slug}`.
//! - `[text](src/lib.rs)` → [`super::EdgeType::Documents`] pointing
//!   at the source artifact.
//! - `[text](other-doc.md)` → [`super::EdgeType::LinksTo`] pointing
//!   at the other markdown artifact.
//! - External `https://…` URLs are skipped — we don't materialise
//!   web links in the graph.

use pulldown_cmark::{Event, Tag};

use super::super::analyzer::{artifact_logical_key, CodeEdge, EdgeType, NodeRef, ResolutionTarget};
use super::{
    is_markdown_path, is_source_path, markdown_source_node, resolve_relative_link, ParsedMarkdown,
};

fn doc_section_resolved(repo: &str, path: &str, slug: &str) -> ResolutionTarget {
    ResolutionTarget::Resolved(NodeRef {
        label: "DocSection".into(),
        natural_key: format!("{repo}|{path}#{slug}"),
    })
}

fn artifact_resolved(repo: &str, path: &str) -> ResolutionTarget {
    ResolutionTarget::Resolved(NodeRef {
        label: "Artifact".into(),
        natural_key: artifact_logical_key(repo, path),
    })
}

/// Walk every `Tag::Link` event and produce a `LinksTo` /
/// `Documents` / `LinksToSection` edge per link.
pub fn extract(parsed: &ParsedMarkdown<'_>, repo: &str, path: &str) -> Vec<CodeEdge> {
    let mut out: Vec<CodeEdge> = Vec::new();
    for (event, range) in &parsed.events {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            let line = parsed.line_at(range.start);
            if let Some(edge) = build_link_edge(repo, path, dest_url.as_ref(), line) {
                out.push(edge);
            }
        }
    }
    out
}

fn build_link_edge(repo: &str, path: &str, url: &str, line: u32) -> Option<CodeEdge> {
    let url = url.trim();
    if url.is_empty()
        || url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("mailto:")
    {
        return None;
    }
    let (link_path, fragment) = split_fragment(url);
    if link_path.is_empty() {
        if let Some(frag) = fragment {
            return Some(CodeEdge {
                from_node: markdown_source_node(repo, path),
                edge_type: EdgeType::LinksToSection,
                to_target: doc_section_resolved(repo, path, frag),
                source_line: Some(line),
                kind: "section_link_inline",
            });
        }
        return None;
    }
    let resolved = resolve_relative_link(path, link_path);
    if let Some(frag) = fragment {
        return Some(CodeEdge {
            from_node: markdown_source_node(repo, path),
            edge_type: EdgeType::LinksToSection,
            to_target: doc_section_resolved(repo, &resolved, frag),
            source_line: Some(line),
            kind: "section_link",
        });
    }
    let edge_type = if is_markdown_path(&resolved) {
        EdgeType::LinksTo
    } else if is_source_path(&resolved) {
        EdgeType::Documents
    } else {
        EdgeType::LinksTo
    };
    Some(CodeEdge {
        from_node: markdown_source_node(repo, path),
        edge_type,
        to_target: artifact_resolved(repo, &resolved),
        source_line: Some(line),
        kind: "link",
    })
}

fn split_fragment(url: &str) -> (&str, Option<&str>) {
    if let Some(idx) = url.find('#') {
        let (path, rest) = url.split_at(idx);
        let frag = &rest[1..];
        (path, if frag.is_empty() { None } else { Some(frag) })
    } else {
        (url, None)
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;

    fn extract_for(src: &str, path: &str) -> Vec<CodeEdge> {
        let parsed = parse(src);
        extract(&parsed, "cortex", path)
    }

    #[test]
    fn doc_to_doc_link_emits_links_to() {
        let edges = extract_for("see [other](./sibling.md)\n", "docs/spec.md");
        let l: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::LinksTo)
            .collect();
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn doc_to_code_link_emits_documents() {
        let edges = extract_for("see [src](../crates/foo/src/lib.rs)\n", "docs/spec.md");
        let d: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Documents)
            .collect();
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn fragment_link_emits_links_to_section() {
        let edges = extract_for(
            "see [output](./12-pre-thinking-injection.md#output)\n",
            "docs/spec.md",
        );
        let s: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::LinksToSection)
            .collect();
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn external_url_is_skipped() {
        let edges = extract_for("see [docs](https://docs.rs/foo)\n", "README.md");
        assert!(edges.is_empty());
    }

    #[test]
    fn inline_fragment_only_link_emits_section_edge() {
        let edges = extract_for("see [top](#title)\n", "docs/spec.md");
        let s: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::LinksToSection)
            .collect();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, "section_link_inline");
    }
}
