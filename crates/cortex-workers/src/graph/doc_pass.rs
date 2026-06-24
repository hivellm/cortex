//! Phase23e §1 — Deterministic doc pass: Article / Entity nodes +
//! RELATED (wikilink) and MENTIONS edges from the documentation corpus.
//!
//! The pass is the Phase 1 (deterministic) step for the docs-as-graph
//! pipeline. Its output feeds the phase23c reconciliation gate and the
//! §2 LLM annotation pass.

use std::collections::BTreeMap;
use std::path::Path;

use crate::graph::patch::{ConflictPolicy, EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

// ── Node / edge key helpers ───────────────────────────────────────────────────

/// Canonical natural key for an `Article` node: `{repo}|{path}`.
pub fn article_key(repo: &str, path: &str) -> String {
    format!("{repo}|{path}")
}

/// Canonical natural key for an `Entity` node: `{repo}|{name}`.
pub fn entity_key(repo: &str, name: &str) -> String {
    format!("{repo}|{name}")
}

/// Canonical natural key for a `Claim` node: `{repo}|{path}|{index}`.
pub fn claim_key(repo: &str, path: &str, index: usize) -> String {
    format!("{repo}|{path}|{index}")
}

// ── DocPass ───────────────────────────────────────────────────────────────────

/// Deterministic markdown doc pass.
///
/// Call [`DocPass::scan_file`] per document to produce a [`GraphPatch`]
/// containing `Article` and `Entity` nodes plus `RELATED` (wikilink)
/// and `MENTIONS` edges. All edges use [`EdgeConfidence::Extracted`].
pub struct DocPass {
    /// The repo slug this pass is running against.
    pub repo: String,
}

impl DocPass {
    /// Parse `content` (a markdown file) and produce a [`GraphPatch`].
    ///
    /// Emits:
    /// - One `Article` node for the file.
    /// - `RELATED` edges for every `[[wikilink]]` found in the content.
    /// - `Entity` nodes + `MENTIONS` edges for bold inline terms.
    /// - YAML frontmatter props set on the `Article` node.
    pub fn scan_file(&self, content: &str, path: &str) -> GraphPatch {
        let article_k = article_key(&self.repo, path);
        let mut patch = GraphPatch::default();

        let mut article_props: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        article_props.insert(
            "repo".to_string(),
            serde_json::Value::String(self.repo.clone()),
        );
        article_props.insert(
            "path".to_string(),
            serde_json::Value::String(path.to_string()),
        );

        let (frontmatter_map, body) = split_frontmatter(content);
        for (k, v) in &frontmatter_map {
            article_props.insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        patch.nodes.push(NodeOp {
            label: "Article".to_string(),
            natural_key: article_k.clone(),
            external_id: Some(article_k.clone()),
            conflict_policy: ConflictPolicy::Match,
            props: article_props,
        });

        // Wikilinks → RELATED edges.
        // A forward-reference Article node is emitted so the gate has
        // an endpoint anchor; it will be merged when that doc is processed.
        for target_path in extract_wikilinks(body) {
            let target_k = article_key(&self.repo, &target_path);
            if !patch.nodes.iter().any(|n| n.natural_key == target_k) {
                patch.nodes.push(NodeOp {
                    label: "Article".to_string(),
                    natural_key: target_k.clone(),
                    external_id: Some(target_k.clone()),
                    conflict_policy: ConflictPolicy::Match,
                    props: {
                        let mut p = BTreeMap::new();
                        p.insert(
                            "repo".to_string(),
                            serde_json::Value::String(self.repo.clone()),
                        );
                        p.insert(
                            "path".to_string(),
                            serde_json::Value::String(target_path.clone()),
                        );
                        p
                    },
                });
            }
            patch.edges.push(make_edge(
                "RELATED",
                "Article",
                &article_k,
                "Article",
                &target_k,
            ));
        }

        // Bold inline terms → Entity nodes + MENTIONS edges.
        let mut seen_entities: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for term in extract_bold_terms(body) {
            if seen_entities.contains(&term) {
                continue;
            }
            seen_entities.insert(term.clone());
            let ek = entity_key(&self.repo, &term);
            if !patch.nodes.iter().any(|n| n.natural_key == ek) {
                patch.nodes.push(NodeOp {
                    label: "Entity".to_string(),
                    natural_key: ek.clone(),
                    external_id: Some(ek.clone()),
                    conflict_policy: ConflictPolicy::Match,
                    props: {
                        let mut p = BTreeMap::new();
                        p.insert("name".to_string(), serde_json::Value::String(term.clone()));
                        p.insert(
                            "repo".to_string(),
                            serde_json::Value::String(self.repo.clone()),
                        );
                        p
                    },
                });
            }
            patch.edges.push(make_edge("MENTIONS", "Article", &article_k, "Entity", &ek));
        }

        patch
    }

    /// Walk `root` recursively, calling [`Self::scan_file`] for every
    /// `.md` file found. Returns the merged [`GraphPatch`].
    ///
    /// Errors reading individual files are silently skipped so a single
    /// unreadable file never aborts the whole corpus scan.
    pub fn scan_dir(&self, root: &Path) -> GraphPatch {
        let mut merged = GraphPatch::default();
        scan_dir_impl(self, root, root, &mut merged);
        merged
    }
}

fn scan_dir_impl(pass: &DocPass, root: &Path, dir: &Path, out: &mut GraphPatch) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_impl(pass, root, &path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let rel_path = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
            let patch = pass.scan_file(&content, &rel_path);
            out.nodes.extend(patch.nodes);
            out.edges.extend(patch.edges);
        }
    }
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

/// Split a markdown document into YAML frontmatter (key→value pairs) and body.
/// Frontmatter is the block between two `---` fence lines at the very start.
/// Returns empty map when no frontmatter is present.
pub fn split_frontmatter(content: &str) -> (BTreeMap<String, String>, &str) {
    let mut map = BTreeMap::new();
    let trimmed = content.trim_start_matches('\n');
    if !trimmed.starts_with("---") {
        return (map, content);
    }
    let rest = &trimmed["---".len()..];
    let end = match rest.find("\n---") {
        Some(p) => p,
        None => return (map, content),
    };
    let fm_text = &rest[..end];
    let body_start = end + "\n---".len();
    let body = if body_start < rest.len() {
        &rest[body_start..]
    } else {
        ""
    };
    for line in fm_text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    (map, body)
}

/// Extract `[[target]]` wikilink targets from markdown `content`.
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("[[") {
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find("]]") {
            let target = after_open[..end].trim();
            if !target.is_empty() {
                let path = target.split('|').next().unwrap_or(target).trim();
                if !path.is_empty() {
                    links.push(path.to_string());
                }
            }
            rest = &after_open[end + 2..];
        } else {
            break;
        }
    }
    links
}

/// Extract `**term**` and `__term__` bold inline spans from markdown.
/// Only extracts terms up to 40 chars (avoids treating full sentences
/// as entity names).
pub fn extract_bold_terms(content: &str) -> Vec<String> {
    let mut terms = Vec::new();
    extract_bold_delimited(content, "**", &mut terms);
    extract_bold_delimited(content, "__", &mut terms);
    terms
}

fn extract_bold_delimited(content: &str, delim: &str, out: &mut Vec<String>) {
    let mut rest = content;
    while let Some(start) = rest.find(delim) {
        let after_open = &rest[start + delim.len()..];
        if let Some(end) = after_open.find(delim) {
            let term = after_open[..end].trim();
            if !term.is_empty() && term.len() <= 40 && !term.contains('\n') {
                out.push(term.to_ascii_lowercase());
            }
            rest = &after_open[end + delim.len()..];
        } else {
            break;
        }
    }
}

// ── Edge factory ──────────────────────────────────────────────────────────────

fn make_edge(
    edge_type: &str,
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
) -> EdgeOp {
    EdgeOp {
        edge_type: edge_type.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        ..Default::default()
    }
    .with_confidence(EdgeConfidence::Extracted, None)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"---
title: "Understand-Anything Architecture"
status: accepted
date: 2026-06-01
---

# Understand-Anything Architecture

This doc describes the **UA pipeline**. It builds on the [[extraction-contract]]
and the [[incremental-indexer]].

The **reconciliation gate** is the key artefact.

See also [[noncode-parsers|Non-Code Parsers]] for the registry design.
"#;

    fn run_fixture() -> GraphPatch {
        DocPass { repo: "myrepo".to_string() }
            .scan_file(FIXTURE, "docs/analysis/ua-architecture.md")
    }

    #[test]
    fn doc_pass_emits_article_node() {
        let p = run_fixture();
        let has_article = p.nodes.iter().any(|n| {
            n.label == "Article"
                && n.natural_key == "myrepo|docs/analysis/ua-architecture.md"
        });
        assert!(has_article, "should emit Article node for the file");
    }

    #[test]
    fn doc_pass_parses_frontmatter_into_article_props() {
        let p = run_fixture();
        let article = p
            .nodes
            .iter()
            .find(|n| n.label == "Article" && n.natural_key.ends_with("ua-architecture.md"))
            .expect("Article node must exist");
        let title = article.props.get("title").and_then(|v| v.as_str());
        assert_eq!(
            title,
            Some("Understand-Anything Architecture"),
            "frontmatter title must be set"
        );
        let status = article.props.get("status").and_then(|v| v.as_str());
        assert_eq!(status, Some("accepted"), "frontmatter status must be set");
    }

    #[test]
    fn doc_pass_extracts_wikilinks_as_related_edges() {
        let p = run_fixture();
        let related_edges: Vec<_> =
            p.edges.iter().filter(|e| e.edge_type == "RELATED").collect();
        // [[extraction-contract]], [[incremental-indexer]], [[noncode-parsers|...]]
        assert_eq!(related_edges.len(), 3, "should emit 3 RELATED edges");
    }

    #[test]
    fn doc_pass_forward_ref_article_nodes_emitted() {
        let p = run_fixture();
        let has_ec = p
            .nodes
            .iter()
            .any(|n| n.label == "Article" && n.natural_key.contains("extraction-contract"));
        assert!(has_ec, "forward-ref Article for extraction-contract must be emitted");
    }

    #[test]
    fn doc_pass_alias_wikilink_uses_path_not_alias() {
        let p = run_fixture();
        // [[noncode-parsers|Non-Code Parsers]] → target is "noncode-parsers"
        let has_ncp = p
            .nodes
            .iter()
            .any(|n| n.label == "Article" && n.natural_key.contains("noncode-parsers"));
        assert!(has_ncp, "alias wikilink must use path portion not display alias");
    }

    #[test]
    fn doc_pass_extracts_bold_terms_as_entity_nodes() {
        let p = run_fixture();
        let has_ua_pipeline = p
            .nodes
            .iter()
            .any(|n| n.label == "Entity" && n.natural_key.contains("ua pipeline"));
        assert!(has_ua_pipeline, "should emit Entity for bold term 'ua pipeline'");
    }

    #[test]
    fn doc_pass_emits_mentions_edges() {
        let p = run_fixture();
        let has_mentions = p.edges.iter().any(|e| e.edge_type == "MENTIONS");
        assert!(has_mentions, "should emit MENTIONS edges for bold terms");
    }

    #[test]
    fn doc_pass_edges_are_extracted_confidence() {
        let p = run_fixture();
        for edge in &p.edges {
            let conf = edge.props.get("confidence").and_then(|v| v.as_str());
            assert_eq!(
                conf,
                Some("extracted"),
                "all doc pass edges must be extracted confidence"
            );
        }
    }

    #[test]
    fn doc_pass_no_frontmatter_still_emits_article() {
        let p = DocPass { repo: "r".to_string() }
            .scan_file("# Simple doc\n\nNo frontmatter here.", "notes.md");
        assert!(p.nodes.iter().any(|n| n.label == "Article"));
    }

    #[test]
    fn doc_pass_empty_file_emits_only_article_node() {
        let p = DocPass { repo: "r".to_string() }.scan_file("", "empty.md");
        assert_eq!(
            p.nodes.iter().filter(|n| n.label == "Article").count(),
            1,
            "empty file should produce exactly one Article node"
        );
        assert!(p.edges.is_empty(), "empty file should produce no edges");
    }

    #[test]
    fn split_frontmatter_parses_known_keys() {
        let (map, _) = split_frontmatter("---\ntitle: foo\nstatus: draft\n---\nbody");
        assert_eq!(map.get("title").map(String::as_str), Some("foo"));
        assert_eq!(map.get("status").map(String::as_str), Some("draft"));
    }

    #[test]
    fn extract_wikilinks_handles_alias_syntax() {
        let links = extract_wikilinks("see [[path/to/doc|Display Text]] for details");
        assert_eq!(links, vec!["path/to/doc"]);
    }

    #[test]
    fn extract_bold_terms_deduplicates_via_caller() {
        let terms = extract_bold_terms("**foo** and **foo** again");
        // extract_bold_terms itself does NOT dedup — caller does
        assert_eq!(terms.len(), 2);
    }
}
