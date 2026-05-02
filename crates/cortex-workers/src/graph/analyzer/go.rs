//! Phase11k §2.3 — Go [`CodeAnalyzer`].
//!
//! Walks `tree_sitter_go::LANGUAGE` and emits:
//!
//! - **`import_declaration`** → [`EdgeType::ImportsFile`]; one edge
//!   per import_spec (single-line + grouped `import (...)` blocks).
//! - **`call_expression`** → [`EdgeType::Calls`]; bare identifier
//!   (function call) or `pkg.Sym(...)` selector (qualified call).
//! - **`type_spec`** → [`EdgeType::UsesType`] when the right-hand
//!   side references another named type (struct fields, interface
//!   methods receiving foreign types, etc.).

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use super::{
    artifact_logical_key, AnalyzerLanguage, CodeAnalyzer, CodeEdge, EdgeType, NodeRef,
    ResolutionTarget,
};
use crate::graph::identity::symbol_natural_key;

/// Stateless Go analyzer.
#[derive(Debug, Default, Clone, Copy)]
pub struct GoAnalyzer;

impl GoAnalyzer {
    /// Construct.
    pub const fn new() -> Self {
        Self
    }
}

fn go_language() -> Language {
    static CELL: OnceLock<Language> = OnceLock::new();
    CELL.get_or_init(|| tree_sitter_go::LANGUAGE.into()).clone()
}

impl CodeAnalyzer for GoAnalyzer {
    fn language(&self) -> AnalyzerLanguage {
        AnalyzerLanguage::Go
    }

    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge> {
        let mut parser = Parser::new();
        if parser.set_language(&go_language()).is_err() {
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
        let mut walker = GoWalker {
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

struct GoWalker<'a> {
    repo: &'a str,
    path: &'a str,
    bytes: &'a [u8],
    edges: &'a mut Vec<CodeEdge>,
}

impl GoWalker<'_> {
    fn artifact_node(&self) -> NodeRef {
        NodeRef {
            label: "Artifact".into(),
            natural_key: artifact_logical_key(self.repo, self.path),
        }
    }

    fn symbol_node(&self, qualified: &str) -> NodeRef {
        NodeRef {
            label: "Symbol".into(),
            natural_key: symbol_natural_key(self.repo, "go", qualified),
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
            "import_declaration" => {
                self.handle_imports(node);
                return;
            }
            "function_declaration" | "method_declaration" => {
                let pushed = self.push_named(node, scope);
                self.walk_named_children(node, scope, true);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "type_spec" => {
                self.handle_type_spec(node, scope);
            }
            "call_expression" => {
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

    fn handle_imports(&mut self, node: Node<'_>) {
        let line = self.line_of(node);
        // Collect every import_spec / import_spec_list under this
        // declaration.
        self.collect_import_specs(node, line);
    }

    fn collect_import_specs(&mut self, node: Node<'_>, line: Option<u32>) {
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            match c.kind() {
                "import_spec" => self.emit_import_spec(c, line),
                "import_spec_list" => self.collect_import_specs(c, line),
                _ => {}
            }
        }
    }

    fn emit_import_spec(&mut self, node: Node<'_>, line: Option<u32>) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path_node).unwrap_or_default();
        let stripped = raw.trim_matches('"');
        if stripped.is_empty() {
            return;
        }
        let path: Vec<String> = stripped
            .split('/')
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if path.is_empty() {
            return;
        }
        self.edges.push(CodeEdge {
            from_node: self.artifact_node(),
            edge_type: EdgeType::ImportsFile,
            to_target: ResolutionTarget::ModulePath(path),
            source_line: line,
            kind: "import",
        });
    }

    fn handle_type_spec(&mut self, node: Node<'_>, _scope: &[String]) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(type_name) = self.text(name_node) else {
            return;
        };
        let line = self.line_of(node);
        // Walk the type expression to find every referenced named
        // type. Skip the declarator's own name node.
        let name_id = name_node.id();
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if c.id() == name_id {
                continue;
            }
            self.scan_type_refs(c, &type_name, line);
        }
    }

    fn scan_type_refs(&mut self, node: Node<'_>, owner: &str, line: Option<u32>) {
        match node.kind() {
            "type_identifier" | "qualified_type" => {
                if let Some(name) = self.text(node) {
                    let target = if name.contains('.') {
                        ResolutionTarget::ModulePath(
                            name.split('.').map(|s| s.to_string()).collect(),
                        )
                    } else {
                        ResolutionTarget::SymbolName(name)
                    };
                    self.edges.push(CodeEdge {
                        from_node: self.symbol_node(owner),
                        edge_type: EdgeType::UsesType,
                        to_target: target,
                        source_line: line,
                        kind: "type_use",
                    });
                }
                return;
            }
            _ => {}
        }
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            self.scan_type_refs(c, owner, line);
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
            "selector_expression" => {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(src: &str) -> Vec<CodeEdge> {
        GoAnalyzer::new().extract(src, "cortex", "main.go")
    }

    #[test]
    fn single_import_emits_imports_file_edge() {
        let edges = extract("package main\nimport \"fmt\"\n");
        let imp: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::ImportsFile)
            .collect();
        assert_eq!(imp.len(), 1);
        assert_eq!(
            imp[0].to_target,
            ResolutionTarget::ModulePath(vec!["fmt".into()])
        );
    }

    #[test]
    fn grouped_import_block_emits_one_edge_per_spec() {
        let src = "\
package main
import (
    \"fmt\"
    \"net/http\"
)
";
        let edges = extract(src);
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
        assert!(paths.contains(&vec!["fmt".into()]));
        assert!(paths.contains(&vec!["net".into(), "http".into()]));
    }

    #[test]
    fn function_call_emits_calls_edge() {
        let src = "package main\nfunc outer() { helper() }\n";
        let edges = extract(src);
        let calls: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].to_target,
            ResolutionTarget::SymbolName("helper".into())
        );
    }

    #[test]
    fn selector_call_emits_method_call_kind() {
        let src = "package main\nfunc outer() { fmt.Println(\"hi\") }\n";
        let edges = extract(src);
        let calls: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, "method_call");
        assert_eq!(
            calls[0].to_target,
            ResolutionTarget::SymbolName("Println".into())
        );
    }

    #[test]
    fn type_spec_with_struct_fields_emits_type_uses() {
        let src = "\
package main
type Worker struct {
    inner *BaseWorker
}
";
        let edges = extract(src);
        let t: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::UsesType)
            .collect();
        assert!(t
            .iter()
            .any(|e| e.to_target == ResolutionTarget::SymbolName("BaseWorker".into())));
    }
}
