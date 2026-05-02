//! Phase11k §1.2 — Rust [`CodeAnalyzer`] backed by tree-sitter-rust.
//!
//! Walks the CST produced by `tree_sitter_rust::LANGUAGE` and emits one
//! [`CodeEdge`] per **import**, **call**, **type reference**, and **impl
//! block**. The resolver runs in a separate pass (see
//! `crates/cortex-workers/src/graph/resolver/`) so this module never
//! touches the workspace-wide module map.
//!
//! Edge anchoring (matches the proposal's wording verbatim — "from_node
//! always known up-front; file-level for imports, enclosing symbol for
//! calls / type uses"):
//!
//! - **`use_declaration`** → source = file artifact, target =
//!   [`ResolutionTarget::ModulePath`]. Re-exports (`pub use ...`) emit
//!   [`EdgeType::ReExports`] instead of [`EdgeType::ImportsFile`]; the
//!   resolver still dispatches on the path.
//! - **`call_expression`** → source = enclosing symbol's
//!   `crate-style::qualified_name`, target = [`ResolutionTarget::SymbolName`]
//!   (bare ident or method receiver) or [`ResolutionTarget::ModulePath`]
//!   (scoped path).
//! - **type uses** (`type_identifier` / `scoped_type_identifier`) →
//!   source = enclosing symbol, target = `SymbolName` or `ModulePath`.
//!   Emits [`EdgeType::UsesType`] for every reference; the coalescer
//!   dedupes per `(source_symbol, target, edge_type)`.
//! - **`impl_item` with a `trait` field** → emits [`EdgeType::Implements`]
//!   anchored at the impl block's *type* symbol (source) and pointing at
//!   the trait symbol (target).
//!
//! Natural-key convention for the source side: until phase11k §1.4 wires
//! the analyzer's output through the `GraphPatch` builder (which is the
//! one place that knows the artifact's `content_hash`), the analyzer
//! emits a *logical* artifact key of the form `{repo}|{path}` via
//! [`artifact_logical_key`]. The patch builder rewrites this to the
//! canonical `repo|path|content_hash` triple (see
//! [`crate::graph::identity::artifact_natural_key`]) before any Nexus
//! upsert. This keeps the analyzer trait pure (no I/O, no hash lookup)
//! while still producing a deterministic source identity.

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use super::{AnalyzerLanguage, CodeAnalyzer, CodeEdge, EdgeType, NodeRef, ResolutionTarget};
use crate::graph::identity::symbol_natural_key;

/// Stateless Rust analyzer. Tree-sitter parsers are cheap to construct
/// per call (`Parser::new()` does not load grammars); the *grammar*
/// load happens once via the [`OnceLock`] in [`rust_language`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RustAnalyzer;

impl RustAnalyzer {
    /// Construct a fresh analyzer.
    pub const fn new() -> Self {
        Self
    }
}

/// Logical artifact natural key used inside [`CodeEdge::from_node`]
/// before the §1.4 patch builder backfills the content hash. Format
/// is `{repo}|{path}` so the splitter can recover both halves with a
/// single `splitn(2, '|')` — pipe is forbidden in either component
/// upstream by the redactor (matches
/// [`crate::graph::identity::artifact_natural_key`]).
pub fn artifact_logical_key(repo: &str, path: &str) -> String {
    format!("{repo}|{path}")
}

fn rust_language() -> Language {
    static CELL: OnceLock<Language> = OnceLock::new();
    CELL.get_or_init(|| tree_sitter_rust::LANGUAGE.into())
        .clone()
}

impl CodeAnalyzer for RustAnalyzer {
    fn language(&self) -> AnalyzerLanguage {
        AnalyzerLanguage::Rust
    }

    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge> {
        let mut parser = Parser::new();
        if parser.set_language(&rust_language()).is_err() {
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
        let mut edges: Vec<CodeEdge> = Vec::new();
        let mut walker = Walker { repo, path, bytes, edges: &mut edges };
        let mut scope: Vec<String> = Vec::new();
        walker.walk(root, &mut scope);
        edges
    }
}

/// Mutable per-call walk state. Lives on the stack — `bytes` borrows
/// the caller-owned source; `edges` is a sink that the parent
/// `extract` call owns.
struct Walker<'a> {
    repo: &'a str,
    path: &'a str,
    bytes: &'a [u8],
    edges: &'a mut Vec<CodeEdge>,
}

impl Walker<'_> {
    fn artifact_node(&self) -> NodeRef {
        NodeRef {
            label: "Artifact".into(),
            natural_key: artifact_logical_key(self.repo, self.path),
        }
    }

    fn symbol_node(&self, qualified_name: &str) -> NodeRef {
        NodeRef {
            label: "Symbol".into(),
            natural_key: symbol_natural_key(self.repo, "rust", qualified_name),
        }
    }

    fn text(&self, node: Node<'_>) -> Option<String> {
        let s = node.start_byte();
        let e = node.end_byte();
        if e <= s || e > self.bytes.len() {
            return None;
        }
        std::str::from_utf8(&self.bytes[s..e]).ok().map(|t| t.trim().to_string())
    }

    fn line_of(&self, node: Node<'_>) -> Option<u32> {
        Some((node.start_position().row + 1) as u32)
    }

    /// Recursive depth-first traversal. The `scope` stack tracks the
    /// path of named declarations enclosing `node` (e.g. `["mod_a",
    /// "MyType", "method"]`) so calls / type uses can anchor at the
    /// right symbol.
    fn walk(&mut self, node: Node<'_>, scope: &mut Vec<String>) {
        match node.kind() {
            "use_declaration" => {
                self.handle_use(node);
                // Don't descend into use trees — the path inside is
                // not a "type use" in the editor sense.
                return;
            }
            "impl_item" => {
                let pushed = self.handle_impl(node, scope);
                self.walk_named_children(node, scope);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "function_item"
            | "trait_item"
            | "struct_item"
            | "enum_item"
            | "mod_item"
            | "const_item"
            | "static_item" => {
                let pushed = self.push_named_decl(node, scope);
                self.walk_named_children(node, scope);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "call_expression" => {
                self.handle_call(node, scope);
                // Continue descending so nested calls / closures still
                // anchor at the same enclosing symbol.
            }
            "type_identifier" | "scoped_type_identifier" => {
                self.handle_type_use(node, scope);
                return;
            }
            _ => {}
        }
        self.walk_named_children(node, scope);
    }

    fn walk_named_children(&mut self, node: Node<'_>, scope: &mut Vec<String>) {
        let name_field_id = node.child_by_field_name("name").map(|n| n.id());
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            // Skip the *declarator name* — emitting a `USES_TYPE`
            // self-edge for `struct Foo { ... }`'s `Foo` is just
            // noise.
            if Some(c.id()) == name_field_id {
                continue;
            }
            self.walk(c, scope);
        }
    }

    /// Push the named declaration's identifier onto `scope`. Returns
    /// `true` if a name was actually pushed (`false` lets the caller
    /// know there is nothing to pop).
    fn push_named_decl(&self, node: Node<'_>, scope: &mut Vec<String>) -> bool {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Some(name) = self.text(name_node) {
                scope.push(name);
                return true;
            }
        }
        false
    }

    /// `use_declaration` → flatten the use tree into one or more
    /// `Vec<String>` paths and emit one edge per leaf.
    fn handle_use(&mut self, node: Node<'_>) {
        let is_re_export = first_child_is_visibility_modifier(node);
        let line = self.line_of(node);

        // Find the `argument` field — the actual path / list / wildcard.
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let mut paths: Vec<Vec<String>> = Vec::new();
        flatten_use_tree(arg, self.bytes, &[], &mut paths);
        for p in paths {
            if p.is_empty() {
                continue;
            }
            let edge_type = if is_re_export {
                EdgeType::ReExports
            } else {
                EdgeType::ImportsFile
            };
            let kind = if is_re_export {
                "re_export"
            } else {
                "use_decl"
            };
            self.edges.push(CodeEdge {
                from_node: self.artifact_node(),
                edge_type,
                to_target: ResolutionTarget::ModulePath(p),
                source_line: line,
                kind,
            });
        }
    }

    /// `impl_item` → emit `IMPLEMENTS` when a `trait` field is
    /// present, then keep descending so nested method bodies pick up
    /// `Type::method` scope. Returns `true` if `Type` was pushed onto
    /// `scope`.
    fn handle_impl(&mut self, node: Node<'_>, scope: &mut Vec<String>) -> bool {
        let line = self.line_of(node);
        let type_field = node.child_by_field_name("type").and_then(|n| self.text(n));
        let trait_field = node.child_by_field_name("trait").and_then(|n| self.text(n));

        let Some(type_name) = type_field else {
            return false;
        };

        if let Some(trait_name) = trait_field {
            // `impl Trait for Type` → IMPLEMENTS(Type → Trait)
            let trait_path = parse_scoped_path(&trait_name);
            let target = if trait_path.len() == 1 {
                ResolutionTarget::SymbolName(trait_path.into_iter().next().unwrap_or_default())
            } else {
                ResolutionTarget::ModulePath(trait_path)
            };
            self.edges.push(CodeEdge {
                from_node: self.symbol_node(&type_name),
                edge_type: EdgeType::Implements,
                to_target: target,
                source_line: line,
                kind: "impl_block",
            });
        }

        scope.push(type_name);
        true
    }

    /// `call_expression` → resolve the call target's syntactic shape
    /// into a [`ResolutionTarget`] and anchor at the enclosing symbol.
    fn handle_call(&mut self, node: Node<'_>, scope: &[String]) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let line = self.line_of(node);
        let kind = func.kind();
        let (target, sub_kind) = match kind {
            "identifier" => match self.text(func) {
                Some(name) => (ResolutionTarget::SymbolName(name), "function_call"),
                None => return,
            },
            "scoped_identifier" => {
                let path = scoped_identifier_components(func, self.bytes);
                if path.is_empty() {
                    return;
                }
                if path.len() == 1 {
                    (
                        ResolutionTarget::SymbolName(path.into_iter().next().unwrap_or_default()),
                        "function_call",
                    )
                } else {
                    (ResolutionTarget::ModulePath(path), "function_call")
                }
            }
            "field_expression" => {
                let Some(field) = func.child_by_field_name("field") else {
                    return;
                };
                let Some(name) = self.text(field) else {
                    return;
                };
                (ResolutionTarget::SymbolName(name), "method_call")
            }
            "generic_function" => {
                if let Some(inner) = func.child_by_field_name("function") {
                    return self.handle_call_with_func(node, inner, scope, line);
                }
                return;
            }
            _ => return,
        };
        let from = self.enclosing_symbol_or_artifact(scope);
        self.edges.push(CodeEdge {
            from_node: from,
            edge_type: EdgeType::Calls,
            to_target: target,
            source_line: line,
            kind: sub_kind,
        });
    }

    /// Indirection used only by `generic_function` so the generic-args
    /// wrapper resolves to the underlying call shape.
    fn handle_call_with_func(
        &mut self,
        call_node: Node<'_>,
        func: Node<'_>,
        scope: &[String],
        line: Option<u32>,
    ) {
        let kind = func.kind();
        let (target, sub_kind) = match kind {
            "identifier" => match self.text(func) {
                Some(name) => (ResolutionTarget::SymbolName(name), "function_call"),
                None => return,
            },
            "scoped_identifier" => {
                let path = scoped_identifier_components(func, self.bytes);
                if path.is_empty() {
                    return;
                }
                if path.len() == 1 {
                    (
                        ResolutionTarget::SymbolName(path.into_iter().next().unwrap_or_default()),
                        "function_call",
                    )
                } else {
                    (ResolutionTarget::ModulePath(path), "function_call")
                }
            }
            "field_expression" => {
                let Some(field) = func.child_by_field_name("field") else {
                    return;
                };
                let Some(name) = self.text(field) else {
                    return;
                };
                (ResolutionTarget::SymbolName(name), "method_call")
            }
            _ => return,
        };
        let _ = call_node; // span anchor already captured via `line`
        let from = self.enclosing_symbol_or_artifact(scope);
        self.edges.push(CodeEdge {
            from_node: from,
            edge_type: EdgeType::Calls,
            to_target: target,
            source_line: line,
            kind: sub_kind,
        });
    }

    /// Type reference (`Foo`, `Foo<T>`, `crate::Foo`) → emit
    /// `USES_TYPE`. Skips the *trait* and *type* fields of an
    /// enclosing `impl_item` because those are already handled by
    /// `handle_impl` as `IMPLEMENTS`.
    fn handle_type_use(&mut self, node: Node<'_>, scope: &[String]) {
        if is_under_use_declaration(node) {
            return;
        }
        if is_impl_trait_or_type_field(node) {
            return;
        }
        let line = self.line_of(node);
        let target = match node.kind() {
            "type_identifier" => match self.text(node) {
                Some(name) => ResolutionTarget::SymbolName(name),
                None => return,
            },
            "scoped_type_identifier" => {
                let path = scoped_type_components(node, self.bytes);
                if path.is_empty() {
                    return;
                }
                if path.len() == 1 {
                    ResolutionTarget::SymbolName(path.into_iter().next().unwrap_or_default())
                } else {
                    ResolutionTarget::ModulePath(path)
                }
            }
            _ => return,
        };
        let from = self.enclosing_symbol_or_artifact(scope);
        self.edges.push(CodeEdge {
            from_node: from,
            edge_type: EdgeType::UsesType,
            to_target: target,
            source_line: line,
            kind: "type_use",
        });
    }

    fn enclosing_symbol_or_artifact(&self, scope: &[String]) -> NodeRef {
        if scope.is_empty() {
            self.artifact_node()
        } else {
            self.symbol_node(&scope.join("::"))
        }
    }
}

fn first_child_is_visibility_modifier(node: Node<'_>) -> bool {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        return cur.node().kind() == "visibility_modifier";
    }
    false
}

/// Recursive flattener for the `argument` of a `use_declaration`.
/// Handles `scoped_identifier`, `scoped_use_list`, `use_list`,
/// `use_as_clause`, `use_wildcard`, plain `identifier`, `crate`,
/// `self`, `super`.
fn flatten_use_tree(
    node: Node<'_>,
    bytes: &[u8],
    prefix: &[String],
    out: &mut Vec<Vec<String>>,
) {
    match node.kind() {
        "scoped_identifier" => {
            let parts = scoped_identifier_components(node, bytes);
            if !parts.is_empty() {
                let mut full = prefix.to_vec();
                full.extend(parts);
                out.push(full);
            }
        }
        "scoped_use_list" => {
            // `path::{a, b}` — gather path on the left, recurse on
            // the `list` field for each entry.
            let mut new_prefix = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                let parts = scoped_identifier_components(path, bytes);
                new_prefix.extend(parts);
            }
            if let Some(list) = node.child_by_field_name("list") {
                let mut cur = list.walk();
                for c in list.named_children(&mut cur) {
                    flatten_use_tree(c, bytes, &new_prefix, out);
                }
            }
        }
        "use_list" => {
            let mut cur = node.walk();
            for c in node.named_children(&mut cur) {
                flatten_use_tree(c, bytes, prefix, out);
            }
        }
        "use_as_clause" => {
            if let Some(p) = node.child_by_field_name("path") {
                flatten_use_tree(p, bytes, prefix, out);
            }
        }
        "use_wildcard" => {
            // `path::*` — emit the path itself; the resolver treats
            // wildcard imports as a glob over the target module.
            let mut full = prefix.to_vec();
            // First named child should be the path leading up to `*`.
            let mut cur = node.walk();
            for c in node.named_children(&mut cur) {
                let parts = scoped_identifier_components(c, bytes);
                if parts.is_empty() {
                    if let Some(t) = node_text(c, bytes) {
                        full.push(t);
                    }
                } else {
                    full.extend(parts);
                }
            }
            full.push("*".into());
            out.push(full);
        }
        "identifier" | "crate" | "self" | "super" | "primitive_type" => {
            if let Some(t) = node_text(node, bytes) {
                let mut full = prefix.to_vec();
                full.push(t);
                out.push(full);
            }
        }
        _ => {}
    }
}

/// Flatten a `scoped_identifier` (`a::b::c`) into its components.
fn scoped_identifier_components(node: Node<'_>, bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn rec(node: Node<'_>, bytes: &[u8], out: &mut Vec<String>) {
        match node.kind() {
            "scoped_identifier" => {
                if let Some(p) = node.child_by_field_name("path") {
                    rec(p, bytes, out);
                }
                if let Some(n) = node.child_by_field_name("name") {
                    if let Some(t) = node_text(n, bytes) {
                        out.push(t);
                    }
                }
            }
            _ => {
                if let Some(t) = node_text(node, bytes) {
                    out.push(t);
                }
            }
        }
    }
    rec(node, bytes, &mut out);
    out
}

/// Flatten a `scoped_type_identifier` (`Mod::Type` / `Mod::Type<T>`)
/// into its components.
fn scoped_type_components(node: Node<'_>, bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn rec(node: Node<'_>, bytes: &[u8], out: &mut Vec<String>) {
        match node.kind() {
            "scoped_type_identifier" => {
                if let Some(p) = node.child_by_field_name("path") {
                    rec(p, bytes, out);
                }
                if let Some(n) = node.child_by_field_name("name") {
                    if let Some(t) = node_text(n, bytes) {
                        out.push(t);
                    }
                }
            }
            "scoped_identifier" => {
                let parts = scoped_identifier_components(node, bytes);
                out.extend(parts);
            }
            _ => {
                if let Some(t) = node_text(node, bytes) {
                    out.push(t);
                }
            }
        }
    }
    rec(node, bytes, &mut out);
    out
}

fn node_text(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let s = node.start_byte();
    let e = node.end_byte();
    if e <= s || e > bytes.len() {
        return None;
    }
    std::str::from_utf8(&bytes[s..e])
        .ok()
        .map(|t| t.trim().to_string())
}

fn is_under_use_declaration(node: Node<'_>) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        if p.kind() == "use_declaration" {
            return true;
        }
        cur = p.parent();
    }
    false
}

/// Filter so a type identifier that lives in the `trait` or `type`
/// field of an `impl_item` does not double-emit `USES_TYPE` on top
/// of the `IMPLEMENTS` edge `handle_impl` already produced.
fn is_impl_trait_or_type_field(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "impl_item" {
        return false;
    }
    let trait_id = parent.child_by_field_name("trait").map(|n| n.id());
    let type_id = parent.child_by_field_name("type").map(|n| n.id());
    let id = node.id();
    Some(id) == trait_id || Some(id) == type_id
}

/// Parse a textual `Type::Sub::Leaf` string into components. Used
/// where we already pulled the raw text via [`Walker::text`] and
/// don't want a second tree walk just to split on `::`.
fn parse_scoped_path(raw: &str) -> Vec<String> {
    raw.split("::")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> Vec<CodeEdge> {
        RustAnalyzer::new().extract(source, "cortex", "src/lib.rs")
    }

    fn artifact_key() -> String {
        artifact_logical_key("cortex", "src/lib.rs")
    }

    fn symbol_key(qualified: &str) -> String {
        symbol_natural_key("cortex", "rust", qualified)
    }

    fn imports(edges: &[CodeEdge]) -> Vec<&CodeEdge> {
        edges
            .iter()
            .filter(|e| matches!(e.edge_type, EdgeType::ImportsFile | EdgeType::ImportsExternal))
            .collect()
    }

    fn calls(edges: &[CodeEdge]) -> Vec<&CodeEdge> {
        edges
            .iter()
            .filter(|e| matches!(e.edge_type, EdgeType::Calls))
            .collect()
    }

    fn type_uses(edges: &[CodeEdge]) -> Vec<&CodeEdge> {
        edges
            .iter()
            .filter(|e| matches!(e.edge_type, EdgeType::UsesType))
            .collect()
    }

    fn implements(edges: &[CodeEdge]) -> Vec<&CodeEdge> {
        edges
            .iter()
            .filter(|e| matches!(e.edge_type, EdgeType::Implements))
            .collect()
    }

    fn re_exports(edges: &[CodeEdge]) -> Vec<&CodeEdge> {
        edges
            .iter()
            .filter(|e| matches!(e.edge_type, EdgeType::ReExports))
            .collect()
    }

    /// (1) Bare `use foo::bar::Baz;` → one ImportsFile edge anchored
    /// at the artifact, with the full path as ModulePath components.
    #[test]
    fn use_decl_simple_path_emits_imports_file_edge() {
        let src = "use foo::bar::Baz;\n";
        let edges = extract(src);
        let imp = imports(&edges);
        assert_eq!(imp.len(), 1);
        let e = imp[0];
        assert_eq!(e.from_node.label, "Artifact");
        assert_eq!(e.from_node.natural_key, artifact_key());
        assert_eq!(e.edge_type, EdgeType::ImportsFile);
        assert_eq!(
            e.to_target,
            ResolutionTarget::ModulePath(vec!["foo".into(), "bar".into(), "Baz".into()])
        );
        assert_eq!(e.kind, "use_decl");
    }

    /// (2) Grouped `use foo::{a, b::C};` → two import edges, one per
    /// leaf, sharing the `foo` prefix.
    #[test]
    fn use_decl_grouped_emits_one_edge_per_leaf() {
        let src = "use foo::{a, b::C};\n";
        let edges = extract(src);
        let imp = imports(&edges);
        let paths: Vec<Vec<String>> = imp
            .iter()
            .map(|e| match &e.to_target {
                ResolutionTarget::ModulePath(p) => p.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert!(paths.contains(&vec!["foo".into(), "a".into()]));
        assert!(paths.contains(&vec!["foo".into(), "b".into(), "C".into()]));
        assert_eq!(paths.len(), 2);
    }

    /// (3) `pub use crate::Foo;` → ReExports, NOT ImportsFile.
    #[test]
    fn pub_use_emits_re_exports_edge() {
        let src = "pub use crate::Foo;\n";
        let edges = extract(src);
        let re = re_exports(&edges);
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].edge_type, EdgeType::ReExports);
        assert_eq!(re[0].kind, "re_export");
        assert_eq!(
            re[0].to_target,
            ResolutionTarget::ModulePath(vec!["crate".into(), "Foo".into()])
        );
        assert!(imports(&edges).is_empty());
    }

    /// (4) Bare-identifier call inside a function → Calls(SymbolName)
    /// anchored at the enclosing function.
    #[test]
    fn bare_call_anchored_at_enclosing_function() {
        let src = "fn outer() { helper(); }\n";
        let edges = extract(src);
        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].from_node.natural_key, symbol_key("outer"));
        assert_eq!(c[0].edge_type, EdgeType::Calls);
        assert_eq!(
            c[0].to_target,
            ResolutionTarget::SymbolName("helper".into())
        );
        assert_eq!(c[0].kind, "function_call");
    }

    /// (5) Method-style call → Calls(SymbolName(method_name)) with
    /// `kind == "method_call"`.
    #[test]
    fn method_call_emits_method_call_kind() {
        let src = "fn outer() { obj.do_thing(); }\n";
        let edges = extract(src);
        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "method_call");
        assert_eq!(
            c[0].to_target,
            ResolutionTarget::SymbolName("do_thing".into())
        );
    }

    /// (6) Scoped call `mod_a::helper()` → Calls(ModulePath).
    #[test]
    fn scoped_call_uses_module_path_target() {
        let src = "fn outer() { mod_a::helper(); }\n";
        let edges = extract(src);
        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].to_target,
            ResolutionTarget::ModulePath(vec!["mod_a".into(), "helper".into()])
        );
    }

    /// (7) Type reference inside a function signature → UsesType
    /// anchored at the function.
    #[test]
    fn signature_type_use_emits_uses_type() {
        let src = "fn outer(x: MyType) -> Other { todo!() }\n";
        let edges = extract(src);
        let t = type_uses(&edges);
        let names: Vec<String> = t
            .iter()
            .filter_map(|e| match &e.to_target {
                ResolutionTarget::SymbolName(n) => Some(n.clone()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"MyType".to_string()));
        assert!(names.contains(&"Other".to_string()));
        for e in &t {
            assert_eq!(e.from_node.natural_key, symbol_key("outer"));
        }
    }

    /// (8) Type reference inside a struct field → UsesType anchored
    /// at the struct's symbol.
    #[test]
    fn struct_field_type_use_anchored_at_struct() {
        let src = "struct Foo { inner: Bar }\n";
        let edges = extract(src);
        let t = type_uses(&edges);
        assert!(t
            .iter()
            .any(|e| e.from_node.natural_key == symbol_key("Foo")
                && e.to_target == ResolutionTarget::SymbolName("Bar".into())));
        // The struct's own name (`Foo`) must NOT produce a type-use
        // edge.
        assert!(!t.iter().any(|e| e.to_target
            == ResolutionTarget::SymbolName("Foo".into())));
    }

    /// (9) `impl Trait for Type` → IMPLEMENTS(Type → Trait); methods
    /// inside the block anchor at `Type::method`.
    #[test]
    fn impl_block_emits_implements_and_scopes_methods() {
        let src = "impl MyTrait for MyType { fn run(&self) { helper(); } }\n";
        let edges = extract(src);
        let imp = implements(&edges);
        assert_eq!(imp.len(), 1);
        assert_eq!(imp[0].from_node.natural_key, symbol_key("MyType"));
        assert_eq!(
            imp[0].to_target,
            ResolutionTarget::SymbolName("MyTrait".into())
        );
        assert_eq!(imp[0].kind, "impl_block");

        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].from_node.natural_key, symbol_key("MyType::run"));
    }

    /// (10) Inherent `impl Type { ... }` (no trait) → no IMPLEMENTS
    /// edge, but methods still anchor at `Type::method`.
    #[test]
    fn inherent_impl_does_not_emit_implements() {
        let src = "impl MyType { fn run(&self) { helper(); } }\n";
        let edges = extract(src);
        assert!(implements(&edges).is_empty());
        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].from_node.natural_key, symbol_key("MyType::run"));
    }

    /// (11) `use foo::*;` → emits a ModulePath ending in `*` so the
    /// resolver can treat it as a glob.
    #[test]
    fn use_wildcard_path_terminates_with_star() {
        let src = "use foo::bar::*;\n";
        let edges = extract(src);
        let imp = imports(&edges);
        assert_eq!(imp.len(), 1);
        let p = match &imp[0].to_target {
            ResolutionTarget::ModulePath(p) => p.clone(),
            _ => panic!("expected ModulePath"),
        };
        assert_eq!(p.last().map(|s| s.as_str()), Some("*"));
    }

    /// (12) Source line is captured 1-indexed for every edge class.
    #[test]
    fn source_line_is_recorded_for_each_edge() {
        let src = "\
use foo::Bar;
fn outer() {
    helper();
}
struct S { x: T }
impl Tr for S {}
";
        let edges = extract(src);
        for e in &edges {
            let line = e.source_line.expect("line should be set");
            // src is at most 6 lines; lines are 1-indexed.
            assert!((1..=6).contains(&line), "bad line {line}");
        }
        // A spot-check that `helper();` is on line 3.
        let c = calls(&edges);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].source_line, Some(3));
    }

    /// Empty / non-Rust input must short-circuit cleanly.
    #[test]
    fn empty_source_yields_no_edges() {
        assert!(extract("").is_empty());
    }

    /// Resolver stays orthogonal — the analyzer always emits `ModulePath`
    /// for `use` statements, never `ExternalPackage`. The §1.3 resolver
    /// is the one responsible for routing the path to an external crate.
    #[test]
    fn analyzer_never_emits_external_package_directly() {
        let src = "use serde::Deserialize;\nuse vectorizer_sdk::HnswSearch;\n";
        let edges = extract(src);
        for e in imports(&edges) {
            assert!(matches!(e.to_target, ResolutionTarget::ModulePath(_)));
        }
    }
}
