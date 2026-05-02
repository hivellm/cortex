//! Phase11k §3.5 — fenced-code first-line path extraction.
//!
//! When a markdown fenced code block opens with a comment line that
//! looks like a file path (`// path/to/file.rs`, `# path/to/conf.yaml`,
//! `<!-- path/to/file.html -->`), the surrounding section *describes*
//! the file. The walker emits one [`super::EdgeType::DescribesPath`]
//! edge from the markdown artifact to the referenced source path.
//!
//! Path detection is conservative — the comment line MUST contain a
//! recognised source extension (matches
//! [`super::is_source_path`]) and have no whitespace inside the
//! filename so a free-form `// like a fence`-style comment never
//! materialises as an edge.

use pulldown_cmark::{Event, Tag, TagEnd};

use super::super::analyzer::{CodeEdge, EdgeType};
use super::{is_source_path, logical_artifact_target, markdown_source_node, ParsedMarkdown};

/// Scan every fenced code block for a leading comment-line file path
/// and emit `DescribesPath` edges from the markdown artifact to the
/// referenced source file.
pub fn extract(parsed: &ParsedMarkdown<'_>, repo: &str, path: &str) -> Vec<CodeEdge> {
    let mut out: Vec<CodeEdge> = Vec::new();
    let events = &parsed.events;
    let mut i = 0;
    while i < events.len() {
        if let (Event::Start(Tag::CodeBlock(_)), open_range) = (&events[i].0, events[i].1.clone()) {
            // First non-empty Text event is the body's leading line.
            let mut body = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j].0 {
                    Event::Text(t) => body.push_str(t.as_ref()),
                    Event::End(TagEnd::CodeBlock) => break,
                    _ => {}
                }
                j += 1;
            }
            let line = parsed.line_at(open_range.start);
            if let Some(detected) = detect_first_line_path(&body) {
                if is_source_path(detected) {
                    out.push(CodeEdge {
                        from_node: markdown_source_node(repo, path),
                        edge_type: EdgeType::DescribesPath,
                        to_target: logical_artifact_target(repo, detected),
                        source_line: Some(line),
                        kind: "fenced_path",
                    });
                }
            }
            i = j;
        }
        i += 1;
    }
    out
}

fn detect_first_line_path(body: &str) -> Option<&str> {
    let first = body.lines().find(|l| !l.trim().is_empty())?;
    let trimmed = first.trim();
    let candidate = trimmed
        .trim_start_matches("//")
        .trim_start_matches('#')
        .trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim();
    if candidate.is_empty() || candidate.contains(' ') {
        return None;
    }
    if !candidate.contains('.') {
        return None;
    }
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;

    fn run(src: &str) -> Vec<CodeEdge> {
        let parsed = parse(src);
        extract(&parsed, "cortex", "docs/spec.md")
    }

    #[test]
    fn rust_fenced_block_with_path_emits_describes_path_edge() {
        let src = "\
```rust
// crates/foo/src/lib.rs
pub fn helper() {}
```
";
        let edges = run(src);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, EdgeType::DescribesPath);
    }

    #[test]
    fn python_block_with_hash_comment_path_emits_edge() {
        let src = "\
```python
# scripts/run.py
print('hi')
```
";
        let edges = run(src);
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn fenced_block_without_leading_path_emits_nothing() {
        let src = "\
```rust
fn helper() {}
```
";
        let edges = run(src);
        assert!(edges.is_empty());
    }

    #[test]
    fn comment_with_natural_language_is_skipped() {
        let src = "\
```rust
// helper function definition
fn helper() {}
```
";
        let edges = run(src);
        assert!(edges.is_empty());
    }
}
