//! Phase11k §3.3 — `:DocSection` extraction + `:CONTAINS` parent
//! edges.
//!
//! Every `#`-prefixed heading becomes a `:DocSection` node whose
//! natural key is `{repo}|{path}#{slug}` (slug computed by
//! [`super::slugify`]). Headings nested under a deeper heading carry
//! a [`super::EdgeType::Contains`] edge from the *parent* section to
//! the child so a downstream traversal can render the section tree.
//!
//! The graph layer materialises sections via two surfaces:
//!
//! 1. The owning artifact emits a [`super::EdgeType::Documents`]
//!    edge against each top-level section (lands in §3.2's `links`
//!    pass via the cross-call orchestration in [`super::extract`]).
//! 2. Sub-headings emit `:CONTAINS` from their nearest higher-depth
//!    ancestor.

use super::super::analyzer::{artifact_logical_key, CodeEdge, EdgeType, NodeRef, ResolutionTarget};
use super::{doc_section_node, each_heading, slugify, ParsedMarkdown};

fn doc_section_target(repo: &str, path: &str, slug: &str) -> ResolutionTarget {
    ResolutionTarget::Resolved(doc_section_node(repo, path, slug))
}

/// Walk every heading event and emit `:DocSection` ownership edges
/// (`Documents` for top-level sections, `Contains` for nested ones).
pub fn extract(parsed: &ParsedMarkdown<'_>, repo: &str, path: &str) -> Vec<CodeEdge> {
    let mut out: Vec<CodeEdge> = Vec::new();
    // Stack: `(depth, slug)` for the chain of ancestors above the
    // current heading. Each new heading pops every entry whose depth
    // is `>= new.depth` so the top of the stack is always the
    // immediate parent.
    let mut ancestors: Vec<(u32, String)> = Vec::new();

    each_heading(parsed, |text, depth, line, _range| {
        let slug = slugify(text);
        if slug.is_empty() {
            return;
        }
        while ancestors.last().map(|(d, _)| *d >= depth).unwrap_or(false) {
            ancestors.pop();
        }
        if let Some((_parent_depth, parent_slug)) = ancestors.last() {
            out.push(CodeEdge {
                from_node: doc_section_node(repo, path, parent_slug),
                edge_type: EdgeType::Contains,
                to_target: doc_section_target(repo, path, &slug),
                source_line: Some(line),
                kind: "section_contains",
            });
        } else {
            out.push(CodeEdge {
                from_node: NodeRef {
                    label: "Artifact".into(),
                    natural_key: artifact_logical_key(repo, path),
                },
                edge_type: EdgeType::Documents,
                to_target: doc_section_target(repo, path, &slug),
                source_line: Some(line),
                kind: "section_root",
            });
        }
        ancestors.push((depth, slug));
    });
    out
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;

    #[test]
    fn top_level_heading_emits_documents_edge() {
        let parsed = parse("# Title\nbody\n");
        let edges = extract(&parsed, "cortex", "docs/spec.md");
        let docs: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Documents)
            .collect();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].from_node.natural_key, "cortex|docs/spec.md");
    }

    #[test]
    fn nested_headings_emit_contains_chain() {
        let parsed = parse("# Title\n## Sub\n### Deep\n");
        let edges = extract(&parsed, "cortex", "docs/spec.md");
        let contains: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Contains)
            .collect();
        assert_eq!(contains.len(), 2);
    }

    #[test]
    fn sibling_headings_share_parent_not_each_other() {
        let parsed = parse("# Title\n## A\n## B\n");
        let edges = extract(&parsed, "cortex", "docs/spec.md");
        let contains: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Contains)
            .collect();
        // Title contains A, Title contains B.
        assert_eq!(contains.len(), 2);
        for e in &contains {
            assert_eq!(e.from_node.natural_key, "cortex|docs/spec.md#title");
        }
    }
}
