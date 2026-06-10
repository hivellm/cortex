//! Symbol existence verifier for cited `(path, symbol)` pairs (phase17 §3.2–§3.4).
//!
//! Parses source files via Tree-sitter (Rust) or plain string scanning
//! (Markdown) and confirms that the named symbol is still present at the
//! given path. Results for up to 1 000 recently-accessed files are cached
//! in an LRU to avoid repeated disk reads on hot retrieval paths (§3.7).
//!
//! ## Supported languages
//!
//! | Extension | Resolver          | Matches                                   |
//! |-----------|-------------------|-------------------------------------------|
//! | `.rs`     | Tree-sitter (Rust)| `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `type`, `const`, `static` |
//! | `.md`     | String scan       | ATX headings (anchor-slug form), code-fence `fn`/`struct` identifier lines |
//!
//! All other extensions return [`SymbolVerdict::Unsupported`].

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tree_sitter::{Node, Parser};

// ── LRU file-content cache (§3.7) ────────────────────────────────────────────

const CACHE_CAPACITY: usize = 1_000;

type FileCache = Mutex<LruCache<std::path::PathBuf, Arc<String>>>;

fn file_cache() -> &'static FileCache {
    static CACHE: OnceLock<FileCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(CACHE_CAPACITY).expect("non-zero cache size"),
        ))
    })
}

/// Read `path` from disk, returning the content as an `Arc<String>`.
/// On the hot path the value comes from the LRU cache — no disk I/O.
fn read_cached(path: &Path) -> Option<Arc<String>> {
    // Fast path: already cached.
    {
        let mut cache = file_cache().lock().expect("file_cache lock poisoned");
        if let Some(content) = cache.get(path) {
            return Some(content.clone());
        }
    }

    // Slow path: read from disk, then insert into cache.
    let content = std::fs::read_to_string(path).ok()?;
    let arc = Arc::new(content);
    {
        let mut cache = file_cache().lock().expect("file_cache lock poisoned");
        cache.put(path.to_path_buf(), arc.clone());
    }
    Some(arc)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Outcome of a single symbol-existence check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolVerdict {
    /// The symbol was found at the given path.
    Verified,
    /// The file exists but the symbol was not found in it.
    NotFound,
    /// The file does not exist at the given path.
    FileMissing,
    /// The file's language is not supported by any resolver.
    Unsupported,
}

/// Check whether `symbol` exists in the file at `path`.
///
/// Dispatches to the Rust resolver for `.rs` files and the Markdown
/// resolver for `.md` files. All other extensions return
/// [`SymbolVerdict::Unsupported`].
pub fn verify_symbol(path: &Path, symbol: &str) -> SymbolVerdict {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => verify_rust(path, symbol),
        Some("md") => verify_markdown(path, symbol),
        _ => SymbolVerdict::Unsupported,
    }
}

// ── Rust resolver (§3.3) ─────────────────────────────────────────────────────

/// Top-level node kinds that carry a name in Rust.
const RUST_TOP_LEVEL_KINDS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "type_alias",
    "const_item",
    "static_item",
    "macro_definition",
    "macro_invocation",
];

fn rust_language() -> tree_sitter::Language {
    static LANG: OnceLock<tree_sitter::Language> = OnceLock::new();
    LANG.get_or_init(|| tree_sitter_rust::LANGUAGE.into())
        .clone()
}

fn verify_rust(path: &Path, symbol: &str) -> SymbolVerdict {
    let content = match read_cached(path) {
        Some(c) => c,
        None => return SymbolVerdict::FileMissing,
    };

    let mut parser = Parser::new();
    if parser.set_language(&rust_language()).is_err() {
        return SymbolVerdict::Unsupported;
    }
    let Some(tree) = parser.parse(content.as_str(), None) else {
        return SymbolVerdict::NotFound;
    };

    let root = tree.root_node();
    let bytes = content.as_bytes();

    if walk_rust_for_symbol(root, bytes, symbol) {
        SymbolVerdict::Verified
    } else {
        SymbolVerdict::NotFound
    }
}

/// Recursively walk a tree-sitter subtree looking for a node whose
/// identifier child matches `symbol`. Returns `true` on the first match.
fn walk_rust_for_symbol<'a>(node: Node<'a>, bytes: &'a [u8], symbol: &str) -> bool {
    if RUST_TOP_LEVEL_KINDS.contains(&node.kind()) {
        if let Some(name) = rust_item_name(node, bytes) {
            if name == symbol {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if walk_rust_for_symbol(child, bytes, symbol) {
            return true;
        }
    }
    false
}

/// Extract the identifier name from a top-level Rust node.
///
/// Most items carry a `name` child with kind `identifier` or
/// `type_identifier`. For `impl_item` the "name" is the type being
/// implemented (the `type` child), so we match on `type_identifier`.
fn rust_item_name<'a>(node: Node<'a>, bytes: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "type_identifier" | "field_identifier" => {
                let s = child.utf8_text(bytes).ok()?;
                return Some(s);
            }
            _ => {}
        }
    }
    None
}

// ── Markdown resolver (§3.4) ─────────────────────────────────────────────────

/// Convert an ATX heading line (without the leading `#`) to its GitHub
/// anchor slug: lowercase, spaces/underscores → `-`, non-word chars removed,
/// leading/trailing `-` stripped.
fn heading_to_slug(heading: &str) -> String {
    let heading = heading.trim();
    let mut slug = String::with_capacity(heading.len());
    for ch in heading.chars() {
        if ch.is_alphanumeric() || ch == '-' {
            slug.push(ch.to_ascii_lowercase());
        } else if ch == ' ' || ch == '_' {
            slug.push('-');
        }
    }
    // Deduplicate consecutive dashes.
    let mut deduped = String::with_capacity(slug.len());
    let mut prev_dash = false;
    for ch in slug.chars() {
        if ch == '-' {
            if !prev_dash {
                deduped.push(ch);
            }
            prev_dash = true;
        } else {
            deduped.push(ch);
            prev_dash = false;
        }
    }
    deduped.trim_matches('-').to_string()
}

fn verify_markdown(path: &Path, symbol: &str) -> SymbolVerdict {
    let content = match read_cached(path) {
        Some(c) => c,
        None => return SymbolVerdict::FileMissing,
    };

    let target_slug = symbol.trim_start_matches('#').to_string();

    for line in content.lines() {
        // Match ATX headings: `# Heading Text` → slug comparison.
        if let Some(rest) = line.strip_prefix('#') {
            let text = rest.trim_start_matches('#').trim();
            let slug = heading_to_slug(text);
            if slug == target_slug || text == symbol {
                return SymbolVerdict::Verified;
            }
        }

        // Match code-fence identifier lines: look for `fn <symbol>` or
        // `struct <symbol>` etc. inside fenced blocks (simple pattern).
        let stripped = line.trim();
        for kw in &[
            "fn ", "struct ", "enum ", "trait ", "impl ", "mod ", "type ",
        ] {
            if let Some(rest) = stripped.strip_prefix(kw) {
                let ident = rest
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                if ident == symbol {
                    return SymbolVerdict::Verified;
                }
            }
        }
    }

    SymbolVerdict::NotFound
}

// ── Unit tests (§3.8) ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn rust_file(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".rs")
            .tempfile()
            .expect("temp file");
        f.write_all(content.as_bytes()).expect("write");
        f.flush().expect("flush");
        f
    }

    fn md_file(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".md")
            .tempfile()
            .expect("temp file");
        f.write_all(content.as_bytes()).expect("write");
        f.flush().expect("flush");
        f
    }

    // ── Rust: present symbol ─────────────────────────────────────────────────

    #[test]
    fn rust_fn_present() {
        let f = rust_file("pub fn my_function() {}\n");
        assert_eq!(
            verify_symbol(f.path(), "my_function"),
            SymbolVerdict::Verified
        );
    }

    #[test]
    fn rust_struct_present() {
        let f = rust_file("pub struct MyStruct { field: u32 }\n");
        assert_eq!(verify_symbol(f.path(), "MyStruct"), SymbolVerdict::Verified);
    }

    #[test]
    fn rust_enum_present() {
        let f = rust_file("enum Direction { North, South }\n");
        assert_eq!(
            verify_symbol(f.path(), "Direction"),
            SymbolVerdict::Verified
        );
    }

    #[test]
    fn rust_trait_present() {
        let f = rust_file("pub trait Reranker { fn score(&self); }\n");
        assert_eq!(verify_symbol(f.path(), "Reranker"), SymbolVerdict::Verified);
    }

    // ── Rust: renamed symbol → NotFound ─────────────────────────────────────

    #[test]
    fn rust_renamed_symbol_not_found() {
        let f = rust_file("pub fn new_name() {}\n");
        // Cite the old name.
        assert_eq!(verify_symbol(f.path(), "old_name"), SymbolVerdict::NotFound);
    }

    // ── Rust: deleted file → FileMissing ────────────────────────────────────

    #[test]
    fn rust_file_missing() {
        let path = std::path::PathBuf::from("/nonexistent/path/to/file.rs");
        assert_eq!(verify_symbol(&path, "anything"), SymbolVerdict::FileMissing);
    }

    // ── Unsupported language ─────────────────────────────────────────────────

    #[test]
    fn unsupported_extension_returns_unsupported() {
        let path = std::path::PathBuf::from("script.sh");
        assert_eq!(verify_symbol(&path, "foo"), SymbolVerdict::Unsupported);
    }

    #[test]
    fn no_extension_returns_unsupported() {
        let path = std::path::PathBuf::from("Makefile");
        assert_eq!(verify_symbol(&path, "all"), SymbolVerdict::Unsupported);
    }

    // ── Markdown: heading anchor ─────────────────────────────────────────────

    #[test]
    fn markdown_heading_present() {
        let f = md_file("# My Section\n\nSome text.\n");
        assert_eq!(
            verify_symbol(f.path(), "my-section"),
            SymbolVerdict::Verified
        );
    }

    #[test]
    fn markdown_heading_exact_text_present() {
        let f = md_file("## API Reference\n\nDocs here.\n");
        assert_eq!(
            verify_symbol(f.path(), "API Reference"),
            SymbolVerdict::Verified
        );
    }

    #[test]
    fn markdown_heading_renamed_not_found() {
        let f = md_file("# New Section Name\n\nText.\n");
        assert_eq!(
            verify_symbol(f.path(), "old-section-name"),
            SymbolVerdict::NotFound
        );
    }

    // ── Markdown: code-fence identifier ─────────────────────────────────────

    #[test]
    fn markdown_code_fence_fn_present() {
        let f = md_file("```rust\nfn verify_symbol() {}\n```\n");
        assert_eq!(
            verify_symbol(f.path(), "verify_symbol"),
            SymbolVerdict::Verified
        );
    }

    // ── Heading slug edge cases ──────────────────────────────────────────────

    #[test]
    fn heading_to_slug_basic() {
        assert_eq!(heading_to_slug("My Heading"), "my-heading");
    }

    #[test]
    fn heading_to_slug_special_chars_stripped() {
        assert_eq!(heading_to_slug("Hello, World!"), "hello-world");
    }

    #[test]
    fn heading_to_slug_leading_trailing_dash_stripped() {
        assert_eq!(heading_to_slug("  - leading -  "), "leading");
    }
}
