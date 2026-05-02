//! Phase11k §1.3 — workspace module map (Rust focus).
//!
//! [`ModuleMap`] is the in-memory index the [`super::SymbolResolver`]
//! consults on tier-2. It maps fully qualified names
//! (`crate::module_a::helper`) — and the path components that compose
//! them — to the artifact + symbol that defined them. Built once at
//! bootstrap by walking each crate's `src/lib.rs` (or `src/main.rs`)
//! and following every `mod foo;` declaration through the file
//! system.
//!
//! The walker is **best-effort by design**. Inline `mod foo { ... }`
//! blocks recurse in the same file; `#[path = "..."]` attribute
//! overrides are honoured when present; missing siblings (e.g. a
//! `mod foo;` whose target file is absent) are silently skipped so
//! the bootstrap pass never panics on an in-flight refactor.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

/// One symbol defined somewhere in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEntry {
    /// Repo identifier (e.g. `cortex`, `vectorizer`).
    pub repo: String,
    /// Source language (`rust`, `typescript`, …).
    pub language: &'static str,
    /// Fully qualified name (`crate::module_a::helper`).
    pub qualified_name: String,
    /// Repo-relative path of the artifact that owns the symbol.
    pub artifact_path: String,
}

/// In-memory module map. Two indexes:
///
/// - `by_path` — exact tuple lookup keyed on the qualified-name
///   components. Hit on tier-2 scoped paths.
/// - `by_basename` — last-component lookup, returning a unique entry
///   when the basename appears exactly once across the workspace.
///   Hit on the tier-2 basename fallback.
#[derive(Debug, Default, Clone)]
pub struct ModuleMap {
    by_path: HashMap<Vec<String>, ModuleEntry>,
    by_basename: HashMap<String, Vec<ModuleEntry>>,
}

impl ModuleMap {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when neither index has any entry.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty() && self.by_basename.is_empty()
    }

    /// Total number of unique paths inserted.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Insert one entry. The qualified name is split on `::` to build
    /// both indexes.
    pub fn insert(&mut self, entry: ModuleEntry) {
        let parts: Vec<String> = entry
            .qualified_name
            .split("::")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            return;
        }
        if let Some(last) = parts.last().cloned() {
            self.by_basename.entry(last).or_default().push(entry.clone());
        }
        self.by_path.insert(parts, entry);
    }

    /// Exact path lookup. Returns the entry whose
    /// `qualified_name.split("::")` equals `path`.
    pub fn resolve_path(&self, path: &[String]) -> Option<&ModuleEntry> {
        self.by_path.get(path)
    }

    /// Basename fallback. Returns `Some(entry)` only when the bare
    /// name appears exactly once workspace-wide — anything ambiguous
    /// degrades to `None` so the resolver routes to tier-3 / fallback.
    pub fn find_by_basename(&self, name: &str) -> Option<&ModuleEntry> {
        let entries = self.by_basename.get(name)?;
        if entries.len() == 1 {
            entries.first()
        } else {
            None
        }
    }

    /// Iterate every entry the map carries (insertion-order is not
    /// preserved — `HashMap` defines the iteration order).
    pub fn iter(&self) -> impl Iterator<Item = &ModuleEntry> {
        self.by_path.values()
    }
}

/// Recursive walker — starts at a Rust crate's root file (`lib.rs` /
/// `main.rs`) and produces a [`ModuleMap`] populated with every
/// top-level item reachable through `mod foo;` chains. `repo` is the
/// repo identifier carried on the produced entries.
///
/// Returns an empty map when `crate_root` does not exist; an
/// `io::Error` when reading a discovered child file fails.
pub fn build_rust_module_map_from_root(
    repo: &str,
    crate_root: &Path,
) -> io::Result<ModuleMap> {
    let mut map = ModuleMap::new();
    if !crate_root.exists() {
        return Ok(map);
    }
    let initial_prefix: Vec<String> = vec!["crate".into()];
    walk_rust_file(repo, crate_root, &initial_prefix, &mut map)?;
    Ok(map)
}

/// Convenience wrapper used in tests + by §1.7's E2E IT — runs
/// [`extract_rust_module_entries`] against an in-memory source string
/// and inserts every produced entry into `map`.
pub fn ingest_rust_source(
    map: &mut ModuleMap,
    repo: &str,
    artifact_path: &str,
    qualified_prefix: &[String],
    source: &str,
) {
    for entry in extract_rust_module_entries(repo, artifact_path, qualified_prefix, source) {
        map.insert(entry);
    }
}

fn walk_rust_file(
    repo: &str,
    file: &Path,
    qualified_prefix: &[String],
    map: &mut ModuleMap,
) -> io::Result<()> {
    let source = std::fs::read_to_string(file)?;
    let workspace_path = file.to_string_lossy().replace('\\', "/").to_string();

    for entry in extract_rust_module_entries(repo, &workspace_path, qualified_prefix, &source) {
        map.insert(entry);
    }

    for child_mod in extract_mod_decls(&source) {
        let child_qualified: Vec<String> = qualified_prefix
            .iter()
            .cloned()
            .chain(std::iter::once(child_mod.name.clone()))
            .collect();
        if let Some(child_path) = locate_child_module(file, &child_mod) {
            walk_rust_file(repo, &child_path, &child_qualified, map)?;
        }
    }
    Ok(())
}

/// Pull the top-level item names out of one Rust source string and
/// return one [`ModuleEntry`] per item. The `qualified_prefix` is
/// joined with the item name via `::` to build the qualified name.
pub fn extract_rust_module_entries(
    repo: &str,
    artifact_path: &str,
    qualified_prefix: &[String],
    source: &str,
) -> Vec<ModuleEntry> {
    let lang = rust_language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    if root.has_error() && root.named_child_count() == 0 {
        return Vec::new();
    }
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    collect_top_level_items(root, bytes, qualified_prefix, repo, artifact_path, &mut out);
    out
}

fn collect_top_level_items(
    node: Node<'_>,
    bytes: &[u8],
    prefix: &[String],
    repo: &str,
    artifact_path: &str,
    out: &mut Vec<ModuleEntry>,
) {
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        match child.kind() {
            "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "const_item"
            | "static_item"
            | "type_item" => {
                if let Some(name) = node_name(child, bytes) {
                    let qualified = join_qualified(prefix, &name);
                    out.push(ModuleEntry {
                        repo: repo.to_string(),
                        language: "rust",
                        qualified_name: qualified,
                        artifact_path: artifact_path.to_string(),
                    });
                }
            }
            "mod_item" => {
                if let Some(name) = node_name(child, bytes) {
                    let qualified = join_qualified(prefix, &name);
                    out.push(ModuleEntry {
                        repo: repo.to_string(),
                        language: "rust",
                        qualified_name: qualified.clone(),
                        artifact_path: artifact_path.to_string(),
                    });
                    // Recurse into `mod foo { ... }` inline blocks so
                    // their items show up in the map under
                    // `prefix::foo::<inner>`.
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut inner_prefix = prefix.to_vec();
                        inner_prefix.push(name);
                        collect_top_level_items(
                            body,
                            bytes,
                            &inner_prefix,
                            repo,
                            artifact_path,
                            out,
                        );
                    }
                }
            }
            "impl_item" => {
                // Not a definition site for new symbols by itself —
                // method definitions inside `impl Type { ... }` are
                // produced by the analyzer's `Type::method` scope, but
                // for the *map* we only register the inherent type
                // declarations (handled by the `struct_item` branch).
                // We still descend so methods become entries under
                // their `Type::method` qualified name.
                if let Some(type_node) = child.child_by_field_name("type") {
                    if let Some(type_name) = type_text(type_node, bytes) {
                        let mut inner_prefix = prefix.to_vec();
                        inner_prefix.push(type_name);
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_top_level_items(
                                body,
                                bytes,
                                &inner_prefix,
                                repo,
                                artifact_path,
                                out,
                            );
                        }
                    }
                }
            }
            "declaration_list" => {
                // Body of a `mod` block or `extern` block — descend.
                collect_top_level_items(child, bytes, prefix, repo, artifact_path, out);
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
struct ModDecl {
    name: String,
    path_attr: Option<String>,
}

/// Find every `mod foo;` declaration without a body and return its
/// child-name + (optional) `#[path = "..."]` override. Inline
/// `mod foo { ... }` items are handled by [`collect_top_level_items`]
/// directly.
fn extract_mod_decls(source: &str) -> Vec<ModDecl> {
    let lang = rust_language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() != "mod_item" {
            continue;
        }
        if child.child_by_field_name("body").is_some() {
            // `mod foo { ... }` — body is inline; not a sibling-file
            // declaration.
            continue;
        }
        let Some(name) = node_name(child, bytes) else {
            continue;
        };
        let path_attr = path_attribute(child, bytes);
        out.push(ModDecl { name, path_attr });
    }
    out
}

/// Honour `#[path = "alt.rs"]` on `mod foo;` declarations. Returns
/// the override path string when found.
fn path_attribute(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        if c.kind() != "attribute_item" {
            continue;
        }
        if let Ok(text) = std::str::from_utf8(&bytes[c.start_byte()..c.end_byte()]) {
            if let Some(eq) = text.find("path") {
                let tail = &text[eq..];
                if let Some(open) = tail.find('"') {
                    let rest = &tail[open + 1..];
                    if let Some(close) = rest.find('"') {
                        return Some(rest[..close].to_string());
                    }
                }
            }
        }
    }
    None
}

/// Resolve the on-disk file for a sibling `mod foo;` declaration,
/// honouring `#[path = "..."]` overrides and the standard
/// `foo.rs` / `foo/mod.rs` fallbacks.
fn locate_child_module(parent: &Path, decl: &ModDecl) -> Option<PathBuf> {
    let parent_dir = parent.parent()?;
    if let Some(path) = &decl.path_attr {
        let candidate = parent_dir.join(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let parent_stem = parent.file_stem()?.to_string_lossy().to_string();
    let is_module_root = matches!(parent_stem.as_str(), "lib" | "main" | "mod");
    let search_dir = if is_module_root {
        parent_dir.to_path_buf()
    } else {
        parent_dir.join(&parent_stem)
    };
    let cand_a = search_dir.join(format!("{}.rs", decl.name));
    if cand_a.exists() {
        return Some(cand_a);
    }
    let cand_b = search_dir.join(&decl.name).join("mod.rs");
    if cand_b.exists() {
        return Some(cand_b);
    }
    None
}

fn rust_language() -> Language {
    static CELL: OnceLock<Language> = OnceLock::new();
    CELL.get_or_init(|| tree_sitter_rust::LANGUAGE.into())
        .clone()
}

fn node_name(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let name = node.child_by_field_name("name")?;
    std::str::from_utf8(&bytes[name.start_byte()..name.end_byte()])
        .ok()
        .map(|s| s.trim().to_string())
}

fn type_text(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])
        .ok()
        .map(|s| s.trim().to_string())
}

fn join_qualified(prefix: &[String], name: &str) -> String {
    let mut parts = prefix.to_vec();
    parts.push(name.to_string());
    parts.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_top_level_decls_under_crate_prefix() {
        let src = "\
fn helper() {}
struct Foo;
mod inner {
    fn nested() {}
}
";
        let entries =
            extract_rust_module_entries("cortex", "src/lib.rs", &["crate".into()], src);
        let names: Vec<&str> =
            entries.iter().map(|e| e.qualified_name.as_str()).collect();
        assert!(names.contains(&"crate::helper"));
        assert!(names.contains(&"crate::Foo"));
        assert!(names.contains(&"crate::inner"));
        assert!(names.contains(&"crate::inner::nested"));
    }

    #[test]
    fn module_map_resolves_exact_path() {
        let mut m = ModuleMap::new();
        m.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::a::b".into(),
            artifact_path: "src/a.rs".into(),
        });
        let hit = m.resolve_path(&["crate".into(), "a".into(), "b".into()]);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().qualified_name, "crate::a::b");
    }

    #[test]
    fn basename_fallback_returns_none_on_collision() {
        let mut m = ModuleMap::new();
        m.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::a::run".into(),
            artifact_path: "src/a.rs".into(),
        });
        m.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::b::run".into(),
            artifact_path: "src/b.rs".into(),
        });
        assert!(m.find_by_basename("run").is_none());
    }

    #[test]
    fn impl_block_methods_are_qualified_under_type() {
        let src = "\
struct Foo;
impl Foo {
    fn run(&self) {}
}
";
        let entries =
            extract_rust_module_entries("cortex", "src/lib.rs", &["crate".into()], src);
        let names: Vec<&str> =
            entries.iter().map(|e| e.qualified_name.as_str()).collect();
        assert!(names.contains(&"crate::Foo"));
        assert!(names.contains(&"crate::Foo::run"));
    }

    #[test]
    fn build_from_root_walks_sibling_files() -> io::Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("lib.rs");
        std::fs::write(&root, "mod child;\nfn at_root() {}\n")?;
        std::fs::write(dir.path().join("child.rs"), "fn helper() {}\n")?;

        let map = build_rust_module_map_from_root("cortex", &root)?;
        let qualified: Vec<&str> = map.iter().map(|e| e.qualified_name.as_str()).collect();
        assert!(qualified.contains(&"crate::at_root"));
        assert!(qualified.contains(&"crate::child"));
        assert!(qualified.contains(&"crate::child::helper"));
        Ok(())
    }
}
