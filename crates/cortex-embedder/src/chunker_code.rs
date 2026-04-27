//! Tree-sitter code chunker.
//!
//! Loads language grammars lazily (one [`OnceLock`] per language) and emits
//! one chunk per top-level declaration — fn/struct/impl/trait/class/interface/
//! module. Nested items live inside their parent. Declarations whose raw byte
//! length exceeds [`OVERSIZE_THRESHOLD_BYTES`] are passed to the fallback
//! chunker and emitted as a run of `FallbackWindow` chunks tagged with the
//! original language.
//!
//! Language dispatch is driven by `event.context_path`'s extension. Unknown
//! extensions and parse errors both short-circuit to `Ok(vec![])` so the
//! orchestrator can fall through to [`crate::DocChunker`] or
//! [`crate::FallbackChunker`].

use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter::{Language, Node, Parser};

use crate::chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
use crate::chunker_fallback::{event_text, sha256_hex, FallbackChunker};
use crate::embedder::EnrichedEvent;
use crate::identity::dedup_key;
use crate::routing::collection_for;

/// Top-level declarations whose raw byte length exceeds this are windowed
/// via the fallback chunker (spec 06 §Chunkers point 1, oversize fallback).
pub const OVERSIZE_THRESHOLD_BYTES: usize = 8 * 1024;

/// Which source language is being chunked — used for grammar selection,
/// node-kind filtering, and chunk metadata labelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeLanguage {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
    Json,
    Yaml,
    Toml,
}

impl CodeLanguage {
    fn label(self) -> &'static str {
        match self {
            CodeLanguage::Rust => "rust",
            CodeLanguage::TypeScript => "typescript",
            CodeLanguage::Tsx => "tsx",
            CodeLanguage::JavaScript => "javascript",
            CodeLanguage::Python => "python",
            CodeLanguage::Go => "go",
            CodeLanguage::Java => "java",
            CodeLanguage::C => "c",
            CodeLanguage::Cpp => "cpp",
            CodeLanguage::Json => "json",
            CodeLanguage::Yaml => "yaml",
            CodeLanguage::Toml => "toml",
        }
    }

    /// Load (once) the tree-sitter [`Language`] handle for this grammar.
    fn language(self) -> Language {
        match self {
            CodeLanguage::Rust => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_rust::LANGUAGE.into()).clone()
            }
            CodeLanguage::TypeScript => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                    .clone()
            }
            CodeLanguage::Tsx => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_typescript::LANGUAGE_TSX.into())
                    .clone()
            }
            CodeLanguage::JavaScript => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_javascript::LANGUAGE.into())
                    .clone()
            }
            CodeLanguage::Python => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_python::LANGUAGE.into())
                    .clone()
            }
            CodeLanguage::Go => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_go::LANGUAGE.into()).clone()
            }
            CodeLanguage::Java => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_java::LANGUAGE.into())
                    .clone()
            }
            CodeLanguage::C => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_c::LANGUAGE.into()).clone()
            }
            CodeLanguage::Cpp => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_cpp::LANGUAGE.into()).clone()
            }
            CodeLanguage::Json => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_json::LANGUAGE.into())
                    .clone()
            }
            CodeLanguage::Yaml => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_yaml::LANGUAGE.into())
                    .clone()
            }
            CodeLanguage::Toml => {
                static CELL: OnceLock<Language> = OnceLock::new();
                CELL.get_or_init(|| tree_sitter_toml_ng::LANGUAGE.into())
                    .clone()
            }
        }
    }

    /// Return `true` if `kind` is a top-level declaration we want to emit as
    /// its own chunk.
    fn is_top_level_decl(self, kind: &str) -> bool {
        match self {
            CodeLanguage::Rust => matches!(
                kind,
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "impl_item"
                    | "trait_item"
                    | "mod_item"
                    | "const_item"
                    | "static_item"
            ),
            CodeLanguage::TypeScript | CodeLanguage::Tsx | CodeLanguage::JavaScript => matches!(
                kind,
                "function_declaration"
                    | "class_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "export_statement"
                    | "lexical_declaration"
                    | "variable_declaration"
            ),
            CodeLanguage::Python => matches!(kind, "function_definition" | "class_definition"),
            CodeLanguage::Go => matches!(
                kind,
                "function_declaration"
                    | "method_declaration"
                    | "type_declaration"
                    | "var_declaration"
                    | "const_declaration"
            ),
            CodeLanguage::Java => matches!(
                kind,
                "class_declaration"
                    | "interface_declaration"
                    | "enum_declaration"
                    | "method_declaration"
            ),
            CodeLanguage::C => matches!(
                kind,
                "function_definition" | "struct_specifier" | "declaration"
            ),
            CodeLanguage::Cpp => matches!(
                kind,
                "function_definition"
                    | "struct_specifier"
                    | "class_specifier"
                    | "namespace_definition"
                    | "declaration"
            ),
            CodeLanguage::Json => matches!(kind, "pair" | "object" | "array"),
            CodeLanguage::Yaml => matches!(kind, "block_mapping_pair" | "document"),
            CodeLanguage::Toml => matches!(kind, "table" | "table_array_element" | "pair"),
        }
    }
}

/// Match an extension (without leading dot, lower-cased) to a language.
pub(crate) fn detect_language_from_path(path: &str) -> Option<CodeLanguage> {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    Some(match ext {
        "rs" => CodeLanguage::Rust,
        "ts" => CodeLanguage::TypeScript,
        "tsx" => CodeLanguage::Tsx,
        "js" | "mjs" | "cjs" => CodeLanguage::JavaScript,
        "py" => CodeLanguage::Python,
        "go" => CodeLanguage::Go,
        "java" => CodeLanguage::Java,
        "c" | "h" => CodeLanguage::C,
        "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" => CodeLanguage::Cpp,
        "json" => CodeLanguage::Json,
        "yaml" | "yml" => CodeLanguage::Yaml,
        "toml" => CodeLanguage::Toml,
        _ => return None,
    })
}

/// Code chunker backed by Tree-sitter grammars for the Phase-1 language set.
#[derive(Debug, Default, Clone, Copy)]
pub struct CodeChunker;

impl CodeChunker {
    /// Create a new code chunker.
    pub fn new() -> Self {
        Self
    }
}

impl Chunker for CodeChunker {
    fn chunk(&self, event: &EnrichedEvent) -> Result<Vec<Chunk>> {
        let Some(path) = event.context_path.as_deref() else {
            return Ok(Vec::new());
        };
        let Some(lang) = detect_language_from_path(path) else {
            return Ok(Vec::new());
        };
        let text = event_text(event);
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let mut parser = Parser::new();
        if parser.set_language(&lang.language()).is_err() {
            tracing::debug!(lang = lang.label(), "failed to load grammar; falling back");
            return Ok(Vec::new());
        }
        let Some(tree) = parser.parse(&text, None) else {
            tracing::debug!(lang = lang.label(), "tree-sitter parse returned None");
            return Ok(Vec::new());
        };
        let root = tree.root_node();
        if root.has_error() && root.named_child_count() == 0 {
            tracing::debug!(lang = lang.label(), "parse produced no usable root children");
            return Ok(Vec::new());
        }

        let collection = collection_for(&event.kind, "cortex", event.context_repo.as_deref());
        let fallback = FallbackChunker::default();
        let mut out: Vec<Chunk> = Vec::new();
        let mut ordinal: u32 = 0;
        let bytes = text.as_bytes();

        // Walk top-level named children of the root node.
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if !lang.is_top_level_decl(child.kind()) {
                continue;
            }
            let start = child.start_byte();
            let end = child.end_byte();
            if end <= start || end > bytes.len() {
                continue;
            }
            let slice_bytes = &bytes[start..end];
            let Ok(decl_text) = std::str::from_utf8(slice_bytes) else {
                continue;
            };

            let normalized_len = normalized_len(decl_text);
            if normalized_len > OVERSIZE_THRESHOLD_BYTES {
                // Oversize top-level declaration: window via fallback chunker,
                // keep the language label so observers can still see it came
                // from a known grammar.
                let sub = fallback.chunk_text(
                    event,
                    decl_text,
                    Some(lang.label().to_string()),
                    ordinal,
                    "cortex",
                );
                ordinal = ordinal.saturating_add(sub.len() as u32);
                // Sub-chunk byte ranges from fallback are relative to its
                // input text; shift them to the original document's byte
                // coordinate system.
                for mut chunk in sub {
                    if let Some((s, e)) = chunk.metadata.byte_range {
                        chunk.metadata.byte_range = Some((
                            (s as usize + start).min(u32::MAX as usize) as u32,
                            (e as usize + start).min(u32::MAX as usize) as u32,
                        ));
                    }
                    out.push(chunk);
                }
                continue;
            }

            let symbol = extract_symbol(&child, bytes, lang);
            let chunk_hash = sha256_hex(decl_text);
            let key = dedup_key(&event.event_id, ordinal, &chunk_hash);
            out.push(Chunk {
                dedup_key: key,
                parent_event_id: event.event_id.clone(),
                parent_content_hash: event.content_hash.clone(),
                chunk_content_hash: chunk_hash,
                collection: collection.clone(),
                text: decl_text.to_string(),
                metadata: ChunkMetadata {
                    kind: event.kind,
                    topics: event.classifier.topics.clone(),
                    severity: event.classifier.severity,
                    repo: event.context_repo.clone(),
                    path: event.context_path.clone(),
                    symbol: Some(symbol),
                    byte_range: Some((
                        start.min(u32::MAX as usize) as u32,
                        end.min(u32::MAX as usize) as u32,
                    )),
                    language: Some(lang.label().to_string()),
                    source: ChunkSource::Code,
                    prompt_version: None,
                },
            });
            ordinal = ordinal.saturating_add(1);
        }

        Ok(out)
    }
}

/// Return the byte length of `s` after collapsing runs of ASCII whitespace
/// to a single space. Used only for the oversize threshold comparison.
fn normalized_len(s: &str) -> usize {
    let mut count = 0usize;
    let mut in_ws = false;
    for b in s.bytes() {
        let is_ws = b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
        if is_ws {
            if !in_ws {
                count += 1;
                in_ws = true;
            }
        } else {
            count += 1;
            in_ws = false;
        }
    }
    count
}

/// Extract a human-readable symbol name from a top-level node. Falls back to
/// `anon@Lstart-Lend` when no name field is available.
fn extract_symbol(node: &Node<'_>, bytes: &[u8], lang: CodeLanguage) -> String {
    if let Some(name) = node_name_text(node, bytes, lang) {
        return name;
    }
    let start_line = node.start_position().row + 1;
    let end_line = node.end_position().row + 1;
    format!("anon@L{start_line}-L{end_line}")
}

/// Best-effort recovery of the declared-identifier text for `node`, based on
/// the grammar. We try the common `name` field first, then fall through to a
/// grammar-specific named-child scan.
fn node_name_text(node: &Node<'_>, bytes: &[u8], lang: CodeLanguage) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        if let Ok(s) = std::str::from_utf8(&bytes[name.start_byte()..name.end_byte()]) {
            return Some(s.trim().to_string());
        }
    }

    // Per-language fallbacks for nodes that don't expose a `name` field
    // uniformly.
    match lang {
        CodeLanguage::Rust => {
            // impl_item / trait_item: `type` field, or first `type_identifier`
            // child. const_item / static_item: `name` field is present — the
            // branch above catches them. impl_item example:
            //   impl <generics>? <trait for>? <type_identifier> { ... }
            if node.kind() == "impl_item" {
                if let Some(ty) = node.child_by_field_name("type") {
                    if let Ok(s) = std::str::from_utf8(&bytes[ty.start_byte()..ty.end_byte()]) {
                        return Some(s.trim().to_string());
                    }
                }
            }
        }
        CodeLanguage::Go => {
            if node.kind() == "method_declaration" {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Ok(s) =
                        std::str::from_utf8(&bytes[name.start_byte()..name.end_byte()])
                    {
                        return Some(s.trim().to_string());
                    }
                }
            }
        }
        CodeLanguage::C | CodeLanguage::Cpp => {
            // function_definition: name lives under declarator → ... → identifier.
            if matches!(node.kind(), "function_definition") {
                if let Some(decl) = node.child_by_field_name("declarator") {
                    if let Some(ident) = find_first_identifier(&decl, bytes) {
                        return Some(ident);
                    }
                }
            }
        }
        CodeLanguage::TypeScript
        | CodeLanguage::Tsx
        | CodeLanguage::JavaScript
        | CodeLanguage::Python
        | CodeLanguage::Java
        | CodeLanguage::Json
        | CodeLanguage::Yaml
        | CodeLanguage::Toml => {}
    }

    // Generic named-child scan for any node with an `identifier`-ish child.
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        let k = c.kind();
        if matches!(
            k,
            "identifier" | "type_identifier" | "property_identifier" | "field_identifier"
        ) {
            if let Ok(s) = std::str::from_utf8(&bytes[c.start_byte()..c.end_byte()]) {
                return Some(s.trim().to_string());
            }
        }
    }

    None
}

/// Recursive depth-first search for the first `identifier` node in a subtree.
fn find_first_identifier(node: &Node<'_>, bytes: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "field_identifier" | "type_identifier"
        ) {
            if let Ok(s) = std::str::from_utf8(&bytes[child.start_byte()..child.end_byte()]) {
                return Some(s.trim().to_string());
            }
        }
        if let Some(found) = find_first_identifier(&child, bytes) {
            return Some(found);
        }
    }
    None
}

