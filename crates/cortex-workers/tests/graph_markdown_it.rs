//! Phase11k §3.7 — markdown analyzer integration test.
//!
//! Ten cases driving full markdown documents through the analyzer +
//! patch-builder pipeline so the wire shape of every doc-edge class
//! is pinned to the proposal verbatim.

use cortex_workers::graph::analyzer::{build_graph_patch, PatchBuildContext};
use cortex_workers::graph::markdown::MarkdownAnalyzer;
use cortex_workers::graph::patch::EdgeOp;
use cortex_workers::graph::resolver::{LocalSymbols, ModuleMap, PackageMap, SymbolResolver};

fn run(source: &str, path: &str) -> Vec<EdgeOp> {
    let mm = ModuleMap::new();
    let pm = PackageMap::new();
    let ls = LocalSymbols::new("cortex", "markdown", path);
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let edges = MarkdownAnalyzer::new().extract(source, "cortex", path);
    let ctx = PatchBuildContext {
        source_repo: "cortex",
        source_path: path,
        source_content_hash: "sha256:md",
        source_event_id: Some("evt-md-it"),
        resolver: &resolver,
        content_hash_for: &|_repo: &str, _path: &str| None,
        analyzer_version: "phase11k.it.md",
    };
    build_graph_patch(&edges, &ctx).edges
}

fn count(edges: &[EdgeOp], edge_type: &str) -> usize {
    edges.iter().filter(|e| e.edge_type == edge_type).count()
}

/// Case 1 — top-level heading produces one DOCUMENTS edge to a
/// `:DocSection` node.
#[test]
fn top_level_heading_yields_documents_edge() {
    let edges = run("# Title\n\nbody\n", "docs/spec.md");
    assert!(edges.iter().any(|e| e.edge_type == "DOCUMENTS"
        && e.to_label == "DocSection"
        && e.to_key == "cortex|docs/spec.md#title"));
}

/// Case 2 — nested headings produce a CONTAINS chain.
#[test]
fn nested_headings_produce_contains_chain() {
    let edges = run("# Title\n## Sub\n### Deep\n", "docs/spec.md");
    assert_eq!(count(&edges, "CONTAINS"), 2);
}

/// Case 3 — `[other](./sibling.md)` becomes a LINKS_TO edge against a
/// logical artifact.
#[test]
fn doc_to_doc_link_emits_links_to() {
    let edges = run("see [other](./sibling.md)\n", "docs/spec.md");
    assert!(edges.iter().any(|e| e.edge_type == "LINKS_TO"));
}

/// Case 4 — `[src](path/to/file.rs)` becomes a DOCUMENTS edge.
#[test]
fn doc_to_code_link_emits_documents_against_artifact() {
    let edges = run("see [src](../crates/foo/src/lib.rs)\n", "docs/spec.md");
    assert!(edges
        .iter()
        .any(|e| e.edge_type == "DOCUMENTS" && e.to_label == "Artifact"));
}

/// Case 5 — `[output](file.md#anchor)` emits LINKS_TO_SECTION.
#[test]
fn fragment_link_emits_links_to_section() {
    let edges = run("see [output](./other.md#output)\n", "docs/spec.md");
    assert!(edges
        .iter()
        .any(|e| e.edge_type == "LINKS_TO_SECTION" && e.to_label == "DocSection"));
}

/// Case 6 — qualified backtick mention emits a high-confidence
/// MENTIONS edge.
#[test]
fn qualified_mention_emits_mentions_edge() {
    let edges = run("the `crate::Foo::bar` helper does X.\n", "docs/spec.md");
    assert!(edges.iter().any(|e| e.edge_type == "MENTIONS"
        && e.props.get("kind").and_then(|v| v.as_str()) == Some("mention_qualified")));
}

/// Case 7 — capitalised bare token classified as type mention.
#[test]
fn capitalised_mention_classified_as_type() {
    let edges = run("the `MyType` struct does X.\n", "docs/spec.md");
    assert!(edges.iter().any(|e| e.edge_type == "MENTIONS"
        && e.props.get("kind").and_then(|v| v.as_str()) == Some("mention_type")));
}

/// Case 8 — fenced code block with leading `// path/to/file.rs`
/// emits DESCRIBES_PATH.
#[test]
fn fenced_block_with_leading_path_emits_describes_path() {
    let src = "\
```rust
// crates/foo/src/lib.rs
pub fn helper() {}
```
";
    let edges = run(src, "docs/spec.md");
    assert!(edges.iter().any(|e| e.edge_type == "DESCRIBES_PATH"));
}

/// Case 9 — external URL is skipped (no edge).
#[test]
fn external_url_does_not_produce_link_edge() {
    let edges = run("see [docs](https://docs.rs/x)\n", "README.md");
    assert!(!edges.iter().any(|e| matches!(
        e.edge_type.as_str(),
        "LINKS_TO" | "LINKS_TO_SECTION" | "DOCUMENTS"
    )));
}

/// Case 10 — composite document carries every edge class together.
#[test]
fn composite_document_emits_every_edge_class() {
    let src = "\
# Title

See `crate::Foo::bar` and [the spec](./spec.md#section).

Path doc:

```rust
// crates/foo/src/lib.rs
pub fn helper() {}
```
";
    let edges = run(src, "docs/index.md");
    assert!(count(&edges, "DOCUMENTS") >= 1);
    assert!(count(&edges, "LINKS_TO_SECTION") >= 1);
    assert!(count(&edges, "MENTIONS") >= 1);
    assert!(count(&edges, "DESCRIBES_PATH") >= 1);
}
