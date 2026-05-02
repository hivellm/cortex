//! Phase11k §2.2 — Python [`CodeAnalyzer`].
//!
//! Walks `tree_sitter_python::LANGUAGE` and emits:
//!
//! - **`import_statement`** / **`import_from_statement`** →
//!   [`EdgeType::ImportsFile`]; the dotted path becomes
//!   [`ResolutionTarget::ModulePath`] components.
//! - **`call`** → [`EdgeType::Calls`]; bare identifier or attribute
//!   access on a receiver. Attribute calls emit `kind = "method_call"`.
//! - **`class_definition` with bases** → [`EdgeType::Extends`] one
//!   edge per declared base class.

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use super::{
    artifact_logical_key, AnalyzerLanguage, CodeAnalyzer, CodeEdge, EdgeType, NodeRef,
    ResolutionTarget,
};
use crate::graph::identity::symbol_natural_key;

/// Stateless Python analyzer.
#[derive(Debug, Default, Clone, Copy)]
pub struct PythonAnalyzer;

impl PythonAnalyzer {
    /// Construct.
    pub const fn new() -> Self {
        Self
    }
}

fn py_language() -> Language {
    static CELL: OnceLock<Language> = OnceLock::new();
    CELL.get_or_init(|| tree_sitter_python::LANGUAGE.into())
        .clone()
}

impl CodeAnalyzer for PythonAnalyzer {
    fn language(&self) -> AnalyzerLanguage {
        AnalyzerLanguage::Python
    }

    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge> {
        let mut parser = Parser::new();
        if parser.set_language(&py_language()).is_err() {
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
        let mut edges = Vec::new();
        let mut walker = PyWalker {
            repo,
            path,
            bytes,
            edges: &mut edges,
        };
        let mut scope: Vec<String> = Vec::new();
        walker.walk(root, &mut scope);
        edges
    }
}

struct PyWalker<'a> {
    repo: &'a str,
    path: &'a str,
    bytes: &'a [u8],
    edges: &'a mut Vec<CodeEdge>,
}

impl PyWalker<'_> {
    fn artifact_node(&self) -> NodeRef {
        NodeRef {
            label: "Artifact".into(),
            natural_key: artifact_logical_key(self.repo, self.path),
        }
    }

    fn symbol_node(&self, qualified: &str) -> NodeRef {
        NodeRef {
            label: "Symbol".into(),
            natural_key: symbol_natural_key(self.repo, "python", qualified),
        }
    }

    fn text(&self, node: Node<'_>) -> Option<String> {
        let s = node.start_byte();
        let e = node.end_byte();
        if e <= s || e > self.bytes.len() {
            return None;
        }
        std::str::from_utf8(&self.bytes[s..e])
            .ok()
            .map(|t| t.trim().to_string())
    }

    fn line_of(&self, node: Node<'_>) -> Option<u32> {
        Some((node.start_position().row + 1) as u32)
    }

    fn enclosing_or_artifact(&self, scope: &[String]) -> NodeRef {
        if scope.is_empty() {
            self.artifact_node()
        } else {
            self.symbol_node(&scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node<'_>, scope: &mut Vec<String>) {
        match node.kind() {
            "import_statement" => {
                self.handle_import(node);
                return;
            }
            "import_from_statement" => {
                self.handle_import_from(node);
                return;
            }
            "class_definition" | "function_definition" => {
                let pushed = self.push_named(node, scope);
                if node.kind() == "class_definition" {
                    self.handle_class_bases(node, scope);
                }
                self.walk_named_children(node, scope, true);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "call" => {
                self.handle_call(node, scope);
            }
            _ => {}
        }
        self.walk_named_children(node, scope, false);
    }

    fn walk_named_children(&mut self, node: Node<'_>, scope: &mut Vec<String>, skip_name: bool) {
        let name_id = if skip_name {
            node.child_by_field_name("name").map(|n| n.id())
        } else {
            None
        };
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if Some(c.id()) == name_id {
                continue;
            }
            self.walk(c, scope);
        }
    }

    fn push_named(&self, node: Node<'_>, scope: &mut Vec<String>) -> bool {
        if let Some(name) = node.child_by_field_name("name").and_then(|n| self.text(n)) {
            scope.push(name);
            return true;
        }
        false
    }

    fn handle_import(&mut self, node: Node<'_>) {
        let line = self.line_of(node);
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            // `import_statement` children: dotted_name (and optional aliased_import).
            let path = match c.kind() {
                "dotted_name" => collect_dotted_name(c, self.bytes),
                "aliased_import" => {
                    if let Some(name_node) = c.child_by_field_name("name") {
                        collect_dotted_name(name_node, self.bytes)
                    } else {
                        Vec::new()
                    }
                }
                _ => continue,
            };
            if path.is_empty() {
                continue;
            }
            self.edges.push(CodeEdge {
                from_node: self.artifact_node(),
                edge_type: EdgeType::ImportsFile,
                to_target: ResolutionTarget::ModulePath(path),
                source_line: line,
                kind: "import",
            });
        }
    }

    fn handle_import_from(&mut self, node: Node<'_>) {
        let line = self.line_of(node);
        let module_path = node
            .child_by_field_name("module_name")
            .map(|n| collect_dotted_name(n, self.bytes))
            .unwrap_or_default();
        if module_path.is_empty() {
            return;
        }
        // For `from foo.bar import a, b` we emit one edge per imported
        // name with the full path `foo.bar.a` / `foo.bar.b`.
        let mut cur = node.walk();
        let mut emitted = false;
        for c in node.named_children(&mut cur) {
            if c.kind() != "dotted_name" && c.kind() != "aliased_import" {
                continue;
            }
            // Skip the module_name slot itself (already captured above).
            if let Some(mn) = node.child_by_field_name("module_name") {
                if c.id() == mn.id() {
                    continue;
                }
            }
            let name_path = if c.kind() == "aliased_import" {
                c.child_by_field_name("name")
                    .map(|n| collect_dotted_name(n, self.bytes))
                    .unwrap_or_default()
            } else {
                collect_dotted_name(c, self.bytes)
            };
            if name_path.is_empty() {
                continue;
            }
            let mut full = module_path.clone();
            full.extend(name_path);
            self.edges.push(CodeEdge {
                from_node: self.artifact_node(),
                edge_type: EdgeType::ImportsFile,
                to_target: ResolutionTarget::ModulePath(full),
                source_line: line,
                kind: "import_from",
            });
            emitted = true;
        }
        if !emitted {
            // `from foo import *` — emit the module path itself.
            self.edges.push(CodeEdge {
                from_node: self.artifact_node(),
                edge_type: EdgeType::ImportsFile,
                to_target: ResolutionTarget::ModulePath(module_path),
                source_line: line,
                kind: "import_from",
            });
        }
    }

    fn handle_class_bases(&mut self, node: Node<'_>, scope: &[String]) {
        let class_name = match scope.last() {
            Some(n) => n.clone(),
            None => return,
        };
        let line = self.line_of(node);
        let Some(superclasses) = node.child_by_field_name("superclasses") else {
            return;
        };
        let mut cur = superclasses.walk();
        for c in superclasses.named_children(&mut cur) {
            let Some(name) = self.text(c) else { continue };
            let target = if name.contains('.') {
                ResolutionTarget::ModulePath(name.split('.').map(|p| p.to_string()).collect())
            } else {
                ResolutionTarget::SymbolName(name)
            };
            self.edges.push(CodeEdge {
                from_node: self.symbol_node(&class_name),
                edge_type: EdgeType::Extends,
                to_target: target,
                source_line: line,
                kind: "extends",
            });
        }
    }

    fn handle_call(&mut self, node: Node<'_>, scope: &[String]) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let line = self.line_of(node);
        let (target, kind) = match func.kind() {
            "identifier" => match self.text(func) {
                Some(name) => (ResolutionTarget::SymbolName(name), "function_call"),
                None => return,
            },
            "attribute" => {
                let Some(attr) = func.child_by_field_name("attribute") else {
                    return;
                };
                let Some(name) = self.text(attr) else {
                    return;
                };
                (ResolutionTarget::SymbolName(name), "method_call")
            }
            _ => return,
        };
        let from = self.enclosing_or_artifact(scope);
        self.edges.push(CodeEdge {
            from_node: from,
            edge_type: EdgeType::Calls,
            to_target: target,
            source_line: line,
            kind,
        });
    }
}

fn collect_dotted_name(node: Node<'_>, bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = node.walk();
    if node.kind() == "identifier" {
        if let Ok(s) = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()]) {
            out.push(s.trim().to_string());
        }
        return out;
    }
    for c in node.named_children(&mut cur) {
        if c.kind() == "identifier" {
            if let Ok(s) = std::str::from_utf8(&bytes[c.start_byte()..c.end_byte()]) {
                out.push(s.trim().to_string());
            }
        }
    }
    if out.is_empty() {
        if let Ok(text) = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()]) {
            for part in text.trim().split('.') {
                if !part.is_empty() {
                    out.push(part.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(src: &str) -> Vec<CodeEdge> {
        PythonAnalyzer::new().extract(src, "cortex", "src/main.py")
    }

    #[test]
    fn plain_import_statement_emits_imports_file_edge() {
        let edges = extract("import os.path\n");
        let imp: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::ImportsFile)
            .collect();
        assert_eq!(imp.len(), 1);
        assert_eq!(
            imp[0].to_target,
            ResolutionTarget::ModulePath(vec!["os".into(), "path".into()])
        );
    }

    #[test]
    fn import_from_emits_one_edge_per_name() {
        let edges = extract("from foo.bar import baz, qux\n");
        let imp: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::ImportsFile)
            .collect();
        assert_eq!(imp.len(), 2);
        let paths: Vec<Vec<String>> = imp
            .iter()
            .map(|e| match &e.to_target {
                ResolutionTarget::ModulePath(p) => p.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert!(paths.contains(&vec!["foo".into(), "bar".into(), "baz".into()]));
        assert!(paths.contains(&vec!["foo".into(), "bar".into(), "qux".into()]));
    }

    #[test]
    fn class_with_bases_emits_extends_edge() {
        let src = "class Worker(BaseWorker, Mixin):\n    pass\n";
        let edges = extract(src);
        let ext: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Extends)
            .collect();
        assert_eq!(ext.len(), 2);
    }

    #[test]
    fn method_call_emits_method_call_kind() {
        let src = "def outer():\n    obj.do_thing()\n";
        let edges = extract(src);
        let calls: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, "method_call");
        assert_eq!(
            calls[0].to_target,
            ResolutionTarget::SymbolName("do_thing".into())
        );
    }
}
