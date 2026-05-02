//! Phase11k §3.6 — Rust intra-doc reference parser.
//!
//! `///` doc comments occasionally carry typed cross-references
//! (`[`crate::module::Symbol`]`, `[Sym`prim@u32`]`,
//! `[link]: crate::Sym`). When a Rust artifact ships, the markdown
//! analyzer feeds the doc-comment text through [`extract_intra_doc_refs`]
//! and the resolver promotes every recovered reference to a
//! `:DOCSTRING_REFERENCES` edge anchored at the Symbol's `:DocSection`.
//!
//! Section §3 of the phase11k task tree owns the wiring side; this
//! module ships the parser itself so §3 can land additively without
//! reshaping the resolver crate.
//!
//! Grammar handled (matches rustdoc's intra-doc resolution rules):
//!
//! - `[crate::module::Symbol]` — full path inside square brackets.
//! - `[Symbol]` — bare basename; resolved against the per-file
//!   symbol table at the call site.
//! - `[link]: crate::module::Symbol` — reference-style links whose
//!   target is a Rust path.

/// One reference recovered from a Rust doc comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntraDocRef {
    /// Path components (`["crate", "foo", "Bar"]` for
    /// `[crate::foo::Bar]`).
    pub path: Vec<String>,
    /// 1-indexed line number of the bracket inside the source doc
    /// comment (the parser receives a single contiguous string —
    /// callers add their own outer offset).
    pub line: u32,
}

/// Recover every intra-doc reference from `source`. The input is the
/// raw text of one Rust doc comment block (with the leading `///`
/// markers already stripped — same shape rustdoc itself sees).
///
/// The parser is strict on what it accepts so a sentence like
/// `"see also `helper`"` is *not* turned into a reference: only
/// `[link]` and `[link]: target` shapes match. Empty brackets and
/// brackets that contain whitespace are skipped.
pub fn extract_intra_doc_refs(source: &str) -> Vec<IntraDocRef> {
    let mut out = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        scan_line(line, line_no as u32 + 1, &mut out);
    }
    out
}

fn scan_line(line: &str, line_no: u32, out: &mut Vec<IntraDocRef>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let Some(end) = find_matching_bracket(bytes, i) else {
            i += 1;
            continue;
        };
        let inner = &line[i + 1..end];
        // Skip if the inner has no character / has whitespace.
        if inner.is_empty() || inner.chars().any(char::is_whitespace) {
            i = end + 1;
            continue;
        }
        // Two shapes:
        //   1. `[link]` — inner is the path itself.
        //   2. `[link]: target` — inner is the label, target follows
        //      the colon.
        let after = &line[end + 1..];
        if let Some(rest) = after.strip_prefix(':') {
            let target = rest.trim();
            if !target.is_empty() && looks_like_path(target) {
                if let Some(refr) = parse_path(target, line_no) {
                    out.push(refr);
                }
            }
        } else if looks_like_path(inner) {
            if let Some(refr) = parse_path(inner, line_no) {
                out.push(refr);
            }
        }
        i = end + 1;
    }
}

fn find_matching_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn looks_like_path(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        && s.chars().any(|c| c.is_ascii_alphanumeric())
}

fn parse_path(s: &str, line: u32) -> Option<IntraDocRef> {
    let parts: Vec<String> = s
        .split("::")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(IntraDocRef { path: parts, line })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_with_spaces_are_ignored() {
        let refs = extract_intra_doc_refs("see [the docs] for more");
        assert!(refs.is_empty());
    }

    #[test]
    fn full_path_in_brackets_round_trips() {
        let refs = extract_intra_doc_refs("uses [crate::foo::Bar] internally");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, vec!["crate", "foo", "Bar"]);
        assert_eq!(refs[0].line, 1);
    }

    #[test]
    fn reference_style_link_target_is_recovered() {
        let src = "consult [helper]: crate::module::helper";
        let refs = extract_intra_doc_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, vec!["crate", "module", "helper"]);
    }

    #[test]
    fn bare_symbol_in_brackets_returns_one_component() {
        let refs = extract_intra_doc_refs("[Symbol] is described here.");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, vec!["Symbol"]);
    }

    #[test]
    fn line_numbers_track_across_multiline_input() {
        let src = "first\nsee [crate::Foo]\nthird";
        let refs = extract_intra_doc_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].line, 2);
    }
}
