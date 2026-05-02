//! Phase11k §2.1 — TypeScript / TSX [`CodeAnalyzer`].
//!
//! Walks the CST produced by `tree_sitter_typescript`'s
//! `LANGUAGE_TYPESCRIPT` grammar and emits the same edge classes the
//! Rust analyzer produces, mapped to TS' surface syntax:
//!
//! - **`import_statement`** → [`EdgeType::ImportsFile`] anchored at
//!   the file artifact, target = [`ResolutionTarget::ModulePath`]
//!   built from the bare-string module specifier.
//! - **`call_expression`** → [`EdgeType::Calls`]; bare identifiers
//!   become [`ResolutionTarget::SymbolName`], `obj.method()` calls
//!   become `SymbolName(method_name)` with `kind = "method_call"`.
//! - **`class_declaration` with `class_heritage`** →
//!   [`EdgeType::Extends`] anchored at the class symbol pointing at
//!   the parent class.
//! - **`class_declaration` with `implements_clause`** →
//!   [`EdgeType::Implements`] one edge per implemented interface.
//!
//! The TSX variant is handled by passing the same source through the
//! `LANGUAGE_TSX` grammar — JSX nodes are skipped automatically since
//! they do not match any of the dispatch arms above.

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use super::{
    artifact_logical_key, AnalyzerLanguage, CodeAnalyzer, CodeEdge, EdgeType, NodeRef,
    ResolutionTarget,
};
use crate::graph::identity::symbol_natural_key;

/// Stateless TypeScript analyzer (handles both `.ts` and `.tsx`).
#[derive(Debug, Default, Clone, Copy)]
pub struct TypescriptAnalyzer;

impl TypescriptAnalyzer {
    /// Construct.
    pub const fn new() -> Self {
        Self
    }
}

fn ts_language(path: &str) -> Language {
    if path.to_ascii_lowercase().ends_with(".tsx") {
        static CELL: OnceLock<Language> = OnceLock::new();
        CELL.get_or_init(|| tree_sitter_typescript::LANGUAGE_TSX.into())
            .clone()
    } else {
        static CELL: OnceLock<Language> = OnceLock::new();
        CELL.get_or_init(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .clone()
    }
}

impl CodeAnalyzer for TypescriptAnalyzer {
    fn language(&self) -> AnalyzerLanguage {
        AnalyzerLanguage::Typescript
    }

    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge> {
        let mut parser = Parser::new();
        if parser.set_language(&ts_language(path)).is_err() {
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
        let mut walker = TsWalker { repo, path, bytes, edges: &mut edges };
        let mut scope: Vec<String> = Vec::new();
        walker.walk(root, &mut scope);
        edges
    }
}

struct TsWalker<'a> {
    repo: &'a str,
    path: &'a str,
    bytes: &'a [u8],
    edges: &'a mut Vec<CodeEdge>,
}

impl TsWalker<'_> {
    fn artifact_node(&self) -> NodeRef {
        NodeRef {
            label: "Artifact".into(),
            natural_key: artifact_logical_key(self.repo, self.path),
        }
    }

    fn symbol_node(&self, qualified: &str) -> NodeRef {
        NodeRef {
            label: "Symbol".into(),
            natural_key: symbol_natural_key(self.repo, "typescript", qualified),
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
            "class_declaration" | "abstract_class_declaration" => {
                let pushed = self.handle_class(node, scope);
                self.walk_named_children(node, scope, true);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "function_declaration"
            | "method_definition"
            | "interface_declaration"
            | "enum_declaration" => {
                let pushed = self.push_named(node, scope);
                self.walk_named_children(node, scope, true);
                if pushed {
                    scope.pop();
                }
                return;
            }
            "call_expression" => {
                self.handle_call(node, scope);
            }
            "type_identifier" => {
                self.handle_type_use(node, scope);
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
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        let raw = self.text(source).unwrap_or_default();
        let stripped = raw.trim_matches(|c: char| c == '\'' || c == '"' || c == '`');
        if stripped.is_empty() {
            return;
        }
        let path: Vec<String> = stripped
            .split('/')
            .map(|p| p.to_string())
            .filter(|p| !p.is_empty())
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

    fn handle_class(&mut self, node: Node<'_>, scope: &mut Vec<String>) -> bool {
        let Some(name_node) = node.child_by_field_name("name") else {
            return false;
        };
        let Some(class_name) = self.text(name_node) else {
            return false;
        };
        let line = self.line_of(node);

        // class_heritage child holds extends_clause / implements_clause.
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if c.kind() == "class_heritage" {
                self.handle_heritage(c, &class_name, line);
            }
        }

        scope.push(class_name);
        true
    }

    fn handle_heritage(&mut self, heritage: Node<'_>, class_name: &str, line: Option<u32>) {
        let mut cur = heritage.walk();
        for c in heritage.named_children(&mut cur) {
            match c.kind() {
                "extends_clause" => {
                    let mut inner = c.walk();
                    for v in c.named_children(&mut inner) {
                        if let Some(name) = self.text(v) {
                            let target = parse_target(&name);
                            self.edges.push(CodeEdge {
                                from_node: self.symbol_node(class_name),
                                edge_type: EdgeType::Extends,
                                to_target: target,
                                source_line: line,
                                kind: "extends",
                            });
                        }
                    }
                }
                "implements_clause" => {
                    let mut inner = c.walk();
                    for v in c.named_children(&mut inner) {
                        if let Some(name) = self.text(v) {
                            let target = parse_target(&name);
                            self.edges.push(CodeEdge {
                                from_node: self.symbol_node(class_name),
                                edge_type: EdgeType::Implements,
                                to_target: target,
                                source_line: line,
                                kind: "implements",
                            });
                        }
                    }
                }
                _ => {}
            }
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
            "member_expression" => {
                let Some(prop) = func.child_by_field_name("property") else {
                    return;
                };
                let Some(name) = self.text(prop) else {
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

    fn handle_type_use(&mut self, node: Node<'_>, scope: &[String]) {
        // Skip declarator-name positions.
        if let Some(parent) = node.parent() {
            if parent
                .child_by_field_name("name")
                .map(|n| n.id() == node.id())
                .unwrap_or(false)
            {
                return;
            }
        }
        let Some(name) = self.text(node) else {
            return;
        };
        let line = self.line_of(node);
        self.edges.push(CodeEdge {
            from_node: self.enclosing_or_artifact(scope),
            edge_type: EdgeType::UsesType,
            to_target: ResolutionTarget::SymbolName(name),
            source_line: line,
            kind: "type_use",
        });
    }
}

fn parse_target(raw: &str) -> ResolutionTarget {
    let parts: Vec<String> = raw
        .split('.')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() == 1 {
        ResolutionTarget::SymbolName(parts.into_iter().next().unwrap_or_default())
    } else {
        ResolutionTarget::ModulePath(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(src: &str, path: &str) -> Vec<CodeEdge> {
        TypescriptAnalyzer::new().extract(src, "cortex", path)
    }

    #[test]
    fn import_statement_emits_imports_file_edge() {
        let edges = extract("import { foo } from './util/helpers';\n", "src/index.ts");
        let imports: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::ImportsFile)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(
            imports[0].to_target,
            ResolutionTarget::ModulePath(vec![".".into(), "util".into(), "helpers".into()])
        );
    }

    #[test]
    fn external_package_import_keeps_single_component_path() {
        let edges = extract("import React from 'react';\n", "src/index.tsx");
        let imports: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::ImportsFile)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(
            imports[0].to_target,
            ResolutionTarget::ModulePath(vec!["react".into()])
        );
    }

    #[test]
    fn class_extends_emits_extends_edge() {
        let src = "class Worker extends BaseWorker { run() {} }\n";
        let edges = extract(src, "src/worker.ts");
        let ext: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Extends)
            .collect();
        assert_eq!(ext.len(), 1);
        assert_eq!(
            ext[0].to_target,
            ResolutionTarget::SymbolName("BaseWorker".into())
        );
        assert_eq!(ext[0].from_node.label, "Symbol");
    }

    #[test]
    fn method_call_emits_method_call_kind() {
        let src = "function outer() { obj.doThing(); }\n";
        let edges = extract(src, "src/index.ts");
        let calls: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, "method_call");
        assert_eq!(
            calls[0].to_target,
            ResolutionTarget::SymbolName("doThing".into())
        );
    }

    #[test]
    fn class_implements_emits_implements_edge() {
        let src = "class Worker implements Runner { run() {} }\n";
        let edges = extract(src, "src/worker.ts");
        let imp: Vec<&CodeEdge> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Implements)
            .collect();
        assert_eq!(imp.len(), 1);
        assert_eq!(
            imp[0].to_target,
            ResolutionTarget::SymbolName("Runner".into())
        );
    }
}
