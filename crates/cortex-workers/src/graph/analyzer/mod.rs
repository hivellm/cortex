//! Phase11k §1 — static code analyzer surface.
//!
//! Each [`CodeAnalyzer`] impl walks one source-file's CST via
//! Tree-sitter and emits [`CodeEdge`] entries that the graph
//! mapper later turns into Nexus edges. The trait is intentionally
//! thin so a new language drops in alongside the existing ones
//! (Rust / TS / Py / Go) without reshaping the surrounding modules.
//!
//! Edge classes the analyzer produces:
//!
//! - [`EdgeType::ImportsFile`] / [`EdgeType::ImportsExternal`] —
//!   one edge per `use` / `import` statement. Resolved against the
//!   workspace's [`super::resolver::ModuleMap`] (intra-workspace) or
//!   [`super::resolver::PackageMap`] (external).
//! - [`EdgeType::Calls`] — one edge per call expression, anchored at
//!   the enclosing symbol.
//! - [`EdgeType::UsesType`] — one edge per type reference in a
//!   signature / struct field / generic bound.
//! - [`EdgeType::Implements`] — Rust `impl Trait for Type` blocks.
//! - [`EdgeType::Extends`] — TS / Py / Java class inheritance.
//! - [`EdgeType::ReExports`] — Rust `pub use` re-exports.
//!
//! The trait itself is sync + pure (no I/O, no async). The
//! Tree-sitter parse + query pass runs on the worker's tokio
//! blocking pool.

use cortex_core::events::Kind;

pub mod go;
pub mod patch_builder;
pub mod python;
pub mod rust;
pub mod typescript;

pub use go::GoAnalyzer;
pub use patch_builder::{
    build_graph_patch, ContentHashLookup, PatchBuildContext, UNKNOWN_CONTENT_HASH,
};
pub use python::PythonAnalyzer;
pub use rust::{artifact_logical_key, RustAnalyzer};
pub use typescript::TypescriptAnalyzer;

/// Closed enum of edge labels the static analyzer can emit. The
/// graph mapper translates these to Nexus edge type strings via
/// [`EdgeType::label`]. Adding a new variant requires updating
/// every per-language analyzer that produces it; the compiler's
/// exhaustive-match check enforces consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeType {
    /// File-level import resolved to another workspace artifact.
    ImportsFile,
    /// File-level import that did NOT resolve inside the workspace.
    /// Targets an [`ResolutionTarget::ExternalPackage`].
    ImportsExternal,
    /// Function / method call. Source = calling symbol, target =
    /// called symbol.
    Calls,
    /// Type reference in a signature / field / generic bound.
    UsesType,
    /// Rust `impl Trait for Type` block. Source = impl block's
    /// type, target = trait.
    Implements,
    /// Class inheritance (TS / Python / Java).
    Extends,
    /// Rust `pub use foo::Bar` re-export.
    ReExports,
    /// Import the resolver could not place inside the workspace or
    /// against any declared external package. Targets a sentinel
    /// `:UnresolvedImport` node so the dashboard can surface coverage
    /// holes.
    UnresolvedImport,
    /// Markdown link `[text](path)` whose target is another markdown
    /// document.
    LinksTo,
    /// Markdown link `[text](src/path.rs)` whose target is a
    /// recognised source-code artifact (the doc *documents* the file).
    Documents,
    /// Markdown link `[text](path#anchor)` or in-document
    /// `[text](#anchor)` — points at a `:DocSection`.
    LinksToSection,
    /// Backtick-token mention of a symbol inside markdown prose.
    Mentions,
    /// Fenced-code first-line `// path/to/file.rs` — the surrounding
    /// section *describes* the artifact at `path`.
    DescribesPath,
    /// Rust intra-doc reference (`/// ... [`crate::Sym`]`) — the
    /// section is documentation for the symbol.
    DocumentedBy,
    /// Reference inside a Rust doc comment that resolves to another
    /// symbol (the `:DOCSTRING_REFERENCES` edge from §3.6).
    DocstringReferences,
    /// `Decision`/`Knowledge`/`Learning` body cites another node by
    /// link. Fired by §4.1's body walker.
    Cites,
    /// `Consolidation.payload.source_event_ids[]` → derived edges
    /// (§4.3).
    DerivedFrom,
    /// `:DocSection` parent → child relationship for nested headings.
    Contains,
}

impl EdgeType {
    /// Nexus edge label string. Matches the SCREAMING_SNAKE_CASE
    /// convention from
    /// [`crate::graph::mapper::normalise_relation_label`].
    pub fn label(self) -> &'static str {
        match self {
            EdgeType::ImportsFile => "IMPORTS_FILE",
            EdgeType::ImportsExternal => "IMPORTS_EXTERNAL",
            EdgeType::Calls => "CALLS",
            EdgeType::UsesType => "USES_TYPE",
            EdgeType::Implements => "IMPLEMENTS",
            EdgeType::Extends => "EXTENDS",
            EdgeType::ReExports => "RE_EXPORTS",
            EdgeType::UnresolvedImport => "UNRESOLVED_IMPORT",
            EdgeType::LinksTo => "LINKS_TO",
            EdgeType::Documents => "DOCUMENTS",
            EdgeType::LinksToSection => "LINKS_TO_SECTION",
            EdgeType::Mentions => "MENTIONS",
            EdgeType::DescribesPath => "DESCRIBES_PATH",
            EdgeType::DocumentedBy => "DOCUMENTED_BY",
            EdgeType::DocstringReferences => "DOCSTRING_REFERENCES",
            EdgeType::Cites => "CITES",
            EdgeType::DerivedFrom => "DERIVED_FROM",
            EdgeType::Contains => "CONTAINS",
        }
    }
}

/// Reference to a graph node by `(label, natural_key)`. Carried on
/// the source side of every [`CodeEdge`] (the analyzer always
/// knows the source artifact / symbol up front; the target needs
/// resolver dispatch).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeRef {
    /// Nexus label (`Artifact`, `Symbol`, `Spec`, …).
    pub label: String,
    /// Natural key the schema-bootstrap constraint enforces unique.
    pub natural_key: String,
}

/// Discriminated target the [`super::resolver::SymbolResolver`] dereferences.
/// The analyzer chooses the variant based on the call site's syntactic
/// shape; the resolver picks the actual node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolutionTarget {
    /// Bare identifier (`foo`, `parse`). Tier-1 lookup against the
    /// per-file symbol table; falls through to tier-2 (crate index)
    /// when the identifier is module-qualified.
    SymbolName(String),
    /// Path components (`["std", "io", "Read"]`). Tier-2 lookup; the
    /// resolver walks the module map and produces a Symbol natural
    /// key when the leaf resolves locally.
    ModulePath(Vec<String>),
    /// Pre-resolved external package name. Tier-3 (no further lookup).
    /// Surfaces as `(:Artifact)-[:IMPORTS_EXTERNAL]->(:ExternalPackage)`.
    ExternalPackage {
        /// Crate / package name as declared in the source workspace's
        /// dependency manifest.
        name: String,
    },
    /// Already-resolved [`NodeRef`]. Used by markdown edges (§3)
    /// whose target is a DocSection / Spec / Decision keyed on a
    /// composite the resolver does not synthesise. The patch builder
    /// emits the edge straight from `from_node` to this `NodeRef`
    /// without consulting [`crate::graph::resolver::SymbolResolver`].
    Resolved(NodeRef),
}

/// One edge the analyzer wants to emit. The resolver dereferences
/// `to_target` into a concrete `NodeRef` before the patch is built.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeEdge {
    /// Source node (always known up-front — file-level for imports,
    /// enclosing symbol for calls / type uses).
    pub from_node: NodeRef,
    /// Edge label.
    pub edge_type: EdgeType,
    /// Target the resolver dereferences.
    pub to_target: ResolutionTarget,
    /// 1-indexed line number in the source file. `None` when the
    /// analyzer could not locate the originating byte (defensive
    /// fallback — every Tree-sitter capture provides one).
    pub source_line: Option<u32>,
    /// Sub-discriminator the dashboard uses to colour-code (e.g.
    /// `"function_call"`, `"method_call"`, `"type_use"`,
    /// `"use_decl"`, `"impl_block"`).
    pub kind: &'static str,
}

/// Languages the analyzer infrastructure understands. Mirrors the
/// chunker's `CodeLanguage` (cortex-workers/src/embedder/) but
/// scoped to the analyzer's needs — we only enumerate languages
/// that have a per-language analyzer impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalyzerLanguage {
    /// Rust source (`.rs`).
    Rust,
    /// TypeScript / TSX source.
    Typescript,
    /// Python source.
    Python,
    /// Go source.
    Go,
}

impl AnalyzerLanguage {
    /// Resolve a path's extension to an [`AnalyzerLanguage`].
    /// Returns `None` for unsupported extensions.
    pub fn from_path_extension(path: &str) -> Option<Self> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())?;
        Some(match ext.as_str() {
            "rs" => AnalyzerLanguage::Rust,
            "ts" | "tsx" => AnalyzerLanguage::Typescript,
            "py" => AnalyzerLanguage::Python,
            "go" => AnalyzerLanguage::Go,
            _ => return None,
        })
    }

    /// Snake-case identifier the resolver carries on the
    /// `:Symbol.language` prop.
    pub fn label(self) -> &'static str {
        match self {
            AnalyzerLanguage::Rust => "rust",
            AnalyzerLanguage::Typescript => "typescript",
            AnalyzerLanguage::Python => "python",
            AnalyzerLanguage::Go => "go",
        }
    }
}

/// Per-language analyzer surface. Implementations are pure +
/// stateless so a single instance can run against many files
/// concurrently from the worker pool.
pub trait CodeAnalyzer: Send + Sync {
    /// Language this analyzer covers.
    fn language(&self) -> AnalyzerLanguage;

    /// Walk the source file's CST and emit unresolved [`CodeEdge`]
    /// entries. The resolver runs in a separate pass so the
    /// analyzer itself never touches the workspace-wide module
    /// map (which is built once and shared).
    ///
    /// `repo` and `path` are repo-relative — the analyzer carries
    /// them in `from_node` natural keys so the resolver doesn't
    /// have to re-thread context.
    fn extract(&self, source: &str, repo: &str, path: &str) -> Vec<CodeEdge>;
}

/// Decide whether an event is a candidate for code analysis.
/// Centralised here so the worker's dispatch path stays in lock-
/// step with the analyzer registry.
pub fn is_analyzable_artifact(kind: Kind, path: Option<&str>) -> bool {
    if !matches!(kind, Kind::Artifact) {
        return false;
    }
    match path {
        Some(p) => AnalyzerLanguage::from_path_extension(p).is_some(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_type_labels_match_screaming_snake_case() {
        assert_eq!(EdgeType::ImportsFile.label(), "IMPORTS_FILE");
        assert_eq!(EdgeType::ImportsExternal.label(), "IMPORTS_EXTERNAL");
        assert_eq!(EdgeType::Calls.label(), "CALLS");
        assert_eq!(EdgeType::UsesType.label(), "USES_TYPE");
        assert_eq!(EdgeType::Implements.label(), "IMPLEMENTS");
        assert_eq!(EdgeType::Extends.label(), "EXTENDS");
        assert_eq!(EdgeType::ReExports.label(), "RE_EXPORTS");
        assert_eq!(EdgeType::UnresolvedImport.label(), "UNRESOLVED_IMPORT");
        assert_eq!(EdgeType::LinksTo.label(), "LINKS_TO");
        assert_eq!(EdgeType::Documents.label(), "DOCUMENTS");
        assert_eq!(EdgeType::LinksToSection.label(), "LINKS_TO_SECTION");
        assert_eq!(EdgeType::Mentions.label(), "MENTIONS");
        assert_eq!(EdgeType::DescribesPath.label(), "DESCRIBES_PATH");
        assert_eq!(EdgeType::DocumentedBy.label(), "DOCUMENTED_BY");
        assert_eq!(EdgeType::DocstringReferences.label(), "DOCSTRING_REFERENCES");
        assert_eq!(EdgeType::Cites.label(), "CITES");
        assert_eq!(EdgeType::DerivedFrom.label(), "DERIVED_FROM");
        assert_eq!(EdgeType::Contains.label(), "CONTAINS");
    }

    #[test]
    fn analyzer_language_resolves_known_extensions() {
        assert_eq!(
            AnalyzerLanguage::from_path_extension("crates/cortex-api/src/lib.rs"),
            Some(AnalyzerLanguage::Rust)
        );
        assert_eq!(
            AnalyzerLanguage::from_path_extension("gui/src/App.tsx"),
            Some(AnalyzerLanguage::Typescript)
        );
        assert_eq!(
            AnalyzerLanguage::from_path_extension("script.PY"),
            Some(AnalyzerLanguage::Python),
            "extension matching must be case-insensitive"
        );
        assert_eq!(AnalyzerLanguage::from_path_extension("README.md"), None);
        assert_eq!(AnalyzerLanguage::from_path_extension("Cargo.toml"), None);
        assert_eq!(AnalyzerLanguage::from_path_extension("noext"), None);
    }

    #[test]
    fn analyzer_language_label_is_snake_case() {
        assert_eq!(AnalyzerLanguage::Rust.label(), "rust");
        assert_eq!(AnalyzerLanguage::Typescript.label(), "typescript");
        assert_eq!(AnalyzerLanguage::Python.label(), "python");
        assert_eq!(AnalyzerLanguage::Go.label(), "go");
    }

    #[test]
    fn is_analyzable_artifact_only_accepts_known_languages() {
        assert!(is_analyzable_artifact(Kind::Artifact, Some("src/lib.rs")));
        assert!(is_analyzable_artifact(
            Kind::Artifact,
            Some("frontend/index.ts")
        ));
        assert!(!is_analyzable_artifact(Kind::Artifact, Some("README.md")));
        assert!(!is_analyzable_artifact(Kind::Artifact, None));
        assert!(!is_analyzable_artifact(Kind::Turn, Some("src/lib.rs")));
    }

    #[test]
    fn code_edge_round_trips_through_clone_and_eq() {
        let e = CodeEdge {
            from_node: NodeRef {
                label: "Artifact".into(),
                natural_key: "cortex|src/lib.rs|sha256:abc".into(),
            },
            edge_type: EdgeType::Calls,
            to_target: ResolutionTarget::SymbolName("foo".into()),
            source_line: Some(42),
            kind: "function_call",
        };
        let clone = e.clone();
        assert_eq!(e, clone);
    }

    #[test]
    fn resolution_target_variants_are_distinguishable() {
        let bare = ResolutionTarget::SymbolName("foo".into());
        let scoped = ResolutionTarget::ModulePath(vec!["std".into(), "io".into()]);
        let external = ResolutionTarget::ExternalPackage {
            name: "tokio".into(),
        };
        let resolved = ResolutionTarget::Resolved(NodeRef {
            label: "DocSection".into(),
            natural_key: "cortex|docs/spec.md#title".into(),
        });
        assert_ne!(bare, scoped);
        assert_ne!(scoped, external);
        assert_ne!(bare, external);
        assert_ne!(resolved, bare);
    }
}
