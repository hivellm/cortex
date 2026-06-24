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
#[allow(deprecated)]
pub use patch_builder::UNKNOWN_CONTENT_HASH;
pub use patch_builder::{
    build_graph_patch, is_pending_artifact_id, pending_artifact_id, ContentHashLookup,
    PatchBuildContext, PENDING_ARTIFACT_PREFIX,
};
pub use python::PythonAnalyzer;
pub use rust::{artifact_logical_key, RustAnalyzer};
pub use typescript::TypescriptAnalyzer;

/// Closed enum of edge labels the static analyzer can emit. The
/// graph mapper translates these to Nexus edge type strings via
/// [`EdgeType::label`]. Adding a new variant requires updating
/// every per-language analyzer that produces it; the compiler's
/// exhaustive-match check enforces consistency.
///
/// **UA-adopted variants** (phase23a ADR #35): the 15 new variants at
/// the bottom are part of the Understand-Anything taxonomy. Existing
/// variants are unchanged for backward compat. Conceptual aliases:
/// `ImportsFile` = UA `imports`, `Documents`/`DocumentedBy` = UA
/// `documents`, `Cites` = UA `cites`, `Contains` = UA `contains`,
/// `Implements` = UA `implements`, `Extends` = UA `inherits`,
/// `Calls` = UA `calls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeType {
    // ── Existing variants (backward-compat, unchanged) ────────────────
    /// File-level import resolved to another workspace artifact.
    /// UA alias: `imports`.
    ImportsFile,
    /// File-level import that did NOT resolve inside the workspace.
    /// Targets a [`ResolutionTarget::ExternalPackage`]. Cortex-only.
    ImportsExternal,
    /// Function / method call. Source = calling symbol, target =
    /// called symbol. UA: `calls`.
    Calls,
    /// Type reference in a signature / field / generic bound.
    /// Cortex-only (finer-grained than UA `depends_on`).
    UsesType,
    /// Rust `impl Trait for Type` block. Source = impl block's
    /// type, target = trait. UA: `implements`.
    Implements,
    /// Class inheritance (TS / Python / Java). UA: `inherits`.
    Extends,
    /// Rust `pub use foo::Bar` re-export. Cortex-only.
    ReExports,
    /// Import the resolver could not place inside the workspace or
    /// against any declared external package. Targets a sentinel
    /// `:UnresolvedImport` node so the dashboard can surface coverage
    /// holes. Cortex-only.
    UnresolvedImport,
    /// Markdown link `[text](path)` whose target is another markdown
    /// document. Cortex-only.
    LinksTo,
    /// Markdown link `[text](src/path.rs)` whose target is a
    /// recognised source-code artifact (the doc *documents* the file).
    /// UA alias: `documents`.
    Documents,
    /// Markdown link `[text](path#anchor)` or in-document
    /// `[text](#anchor)` — points at a `:DocSection`. Cortex-only.
    LinksToSection,
    /// Backtick-token mention of a symbol inside markdown prose.
    /// Cortex-only.
    Mentions,
    /// Fenced-code first-line `// path/to/file.rs` — the surrounding
    /// section *describes* the artifact at `path`. Cortex-only.
    DescribesPath,
    /// Rust intra-doc reference (`/// ... [`crate::Sym`]`) — the
    /// section is documentation for the symbol. UA alias: `documents`.
    DocumentedBy,
    /// Reference inside a Rust doc comment that resolves to another
    /// symbol (the `:DOCSTRING_REFERENCES` edge from §3.6). Cortex-only.
    DocstringReferences,
    /// `Decision`/`Knowledge`/`Learning` body cites another node by
    /// link. Fired by §4.1's body walker. UA: `cites`.
    Cites,
    /// `Consolidation.payload.source_event_ids[]` → derived edges
    /// (§4.3). Cortex-only.
    DerivedFrom,
    /// `:DocSection` parent → child relationship for nested headings.
    /// UA: `contains`.
    Contains,

    // ── UA-adopted variants (phase23a ADR #35) ─────────────────────────
    /// Module / file publicly re-exports a symbol. UA `exports`.
    Exports,
    /// Code reads from a table / resource. UA `reads_from`.
    ReadsFrom,
    /// Code writes to a table / resource. UA `writes_to`.
    WritesTo,
    /// Generic dependency edge (package / library). UA `depends_on`.
    DependsOn,
    /// Test artifact covers a source artifact. UA `tested_by`.
    TestedBy,
    /// Config artifact configures a service / component. UA `configures`.
    Configures,
    /// Deploys a service to an environment. UA `deploys`.
    Deploys,
    /// IaC resource provisions an infra resource. UA `provisions`.
    Provisions,
    /// Hook / pipeline triggers another pipeline / job. UA `triggers`.
    Triggers,
    /// SQL migration applies to a schema / table. UA `migrates`.
    Migrates,
    /// Router routes a path to an endpoint handler. UA `routes`.
    Routes,
    /// Protobuf / GraphQL / DDL file defines a schema. UA `defines_schema`.
    DefinesSchema,
    /// One claim contradicts another. High-value knowledge edge. UA `contradicts`.
    Contradicts,
    /// Claim / learning builds on (extends) another. UA `builds_on`.
    BuildsOn,
    /// Node is categorized under a topic. UA `categorized_under`.
    CategorizedUnder,
}

impl EdgeType {
    /// Nexus edge label string (SCREAMING_SNAKE_CASE convention).
    pub fn label(self) -> &'static str {
        match self {
            // existing variants — unchanged
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
            // UA-adopted variants
            EdgeType::Exports => "EXPORTS",
            EdgeType::ReadsFrom => "READS_FROM",
            EdgeType::WritesTo => "WRITES_TO",
            EdgeType::DependsOn => "DEPENDS_ON",
            EdgeType::TestedBy => "TESTED_BY",
            EdgeType::Configures => "CONFIGURES",
            EdgeType::Deploys => "DEPLOYS",
            EdgeType::Provisions => "PROVISIONS",
            EdgeType::Triggers => "TRIGGERS",
            EdgeType::Migrates => "MIGRATES",
            EdgeType::Routes => "ROUTES",
            EdgeType::DefinesSchema => "DEFINES_SCHEMA",
            EdgeType::Contradicts => "CONTRADICTS",
            EdgeType::BuildsOn => "BUILDS_ON",
            EdgeType::CategorizedUnder => "CATEGORIZED_UNDER",
        }
    }

    /// Reverse-lookup from a Nexus relation string. Handles both canonical
    /// labels and legacy alias strings so reads of pre-phase23a graph data
    /// still resolve (§4.2 backward-compat requirement).
    ///
    /// Legacy aliases accepted:
    /// - `"IMPORTS"` → [`EdgeType::ImportsFile`]
    /// - `"DOCUMENTED_BY"` → [`EdgeType::DocumentedBy`]
    /// - `"IMPORTS_FILE"` → [`EdgeType::ImportsFile`]
    pub fn from_nexus_label(label: &str) -> Option<Self> {
        Some(match label {
            // existing / legacy
            "IMPORTS_FILE" | "IMPORTS" => EdgeType::ImportsFile,
            "IMPORTS_EXTERNAL" => EdgeType::ImportsExternal,
            "CALLS" => EdgeType::Calls,
            "USES_TYPE" => EdgeType::UsesType,
            "IMPLEMENTS" => EdgeType::Implements,
            "EXTENDS" => EdgeType::Extends,
            "RE_EXPORTS" => EdgeType::ReExports,
            "UNRESOLVED_IMPORT" => EdgeType::UnresolvedImport,
            "LINKS_TO" => EdgeType::LinksTo,
            "DOCUMENTS" => EdgeType::Documents,
            "LINKS_TO_SECTION" => EdgeType::LinksToSection,
            "MENTIONS" => EdgeType::Mentions,
            "DESCRIBES_PATH" => EdgeType::DescribesPath,
            "DOCUMENTED_BY" => EdgeType::DocumentedBy,
            "DOCSTRING_REFERENCES" => EdgeType::DocstringReferences,
            "CITES" => EdgeType::Cites,
            "DERIVED_FROM" => EdgeType::DerivedFrom,
            "CONTAINS" => EdgeType::Contains,
            // UA-adopted
            "EXPORTS" => EdgeType::Exports,
            "READS_FROM" => EdgeType::ReadsFrom,
            "WRITES_TO" => EdgeType::WritesTo,
            "DEPENDS_ON" => EdgeType::DependsOn,
            "TESTED_BY" => EdgeType::TestedBy,
            "CONFIGURES" => EdgeType::Configures,
            "DEPLOYS" => EdgeType::Deploys,
            "PROVISIONS" => EdgeType::Provisions,
            "TRIGGERS" => EdgeType::Triggers,
            "MIGRATES" => EdgeType::Migrates,
            "ROUTES" => EdgeType::Routes,
            "DEFINES_SCHEMA" => EdgeType::DefinesSchema,
            "CONTRADICTS" => EdgeType::Contradicts,
            "BUILDS_ON" => EdgeType::BuildsOn,
            "CATEGORIZED_UNDER" => EdgeType::CategorizedUnder,
            _ => return None,
        })
    }

    /// UA ontology name for variants that map to the UA taxonomy, `None`
    /// for Cortex-only variants. Used in documentation and crosswalk
    /// tooling; the Nexus wire format always uses [`Self::label`].
    pub fn ua_name(self) -> Option<&'static str> {
        Some(match self {
            EdgeType::ImportsFile => "imports",
            EdgeType::Calls => "calls",
            EdgeType::Implements => "implements",
            EdgeType::Extends => "inherits",
            EdgeType::Documents | EdgeType::DocumentedBy => "documents",
            EdgeType::Cites => "cites",
            EdgeType::Contains => "contains",
            EdgeType::Exports => "exports",
            EdgeType::ReadsFrom => "reads_from",
            EdgeType::WritesTo => "writes_to",
            EdgeType::DependsOn => "depends_on",
            EdgeType::TestedBy => "tested_by",
            EdgeType::Configures => "configures",
            EdgeType::Deploys => "deploys",
            EdgeType::Provisions => "provisions",
            EdgeType::Triggers => "triggers",
            EdgeType::Migrates => "migrates",
            EdgeType::Routes => "routes",
            EdgeType::DefinesSchema => "defines_schema",
            EdgeType::Contradicts => "contradicts",
            EdgeType::BuildsOn => "builds_on",
            EdgeType::CategorizedUnder => "categorized_under",
            // Cortex-only — no UA equivalent
            EdgeType::ImportsExternal
            | EdgeType::UsesType
            | EdgeType::ReExports
            | EdgeType::UnresolvedImport
            | EdgeType::LinksTo
            | EdgeType::LinksToSection
            | EdgeType::Mentions
            | EdgeType::DescribesPath
            | EdgeType::DocstringReferences
            | EdgeType::DerivedFrom => return None,
        })
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
        assert_eq!(
            EdgeType::DocstringReferences.label(),
            "DOCSTRING_REFERENCES"
        );
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

    // ---------- Phase23a — EdgeType UA vocabulary ----------

    /// Every EdgeType round-trips through label() → from_nexus_label().
    #[test]
    fn edge_type_label_round_trips() {
        let all = [
            // existing
            EdgeType::ImportsFile,
            EdgeType::ImportsExternal,
            EdgeType::Calls,
            EdgeType::UsesType,
            EdgeType::Implements,
            EdgeType::Extends,
            EdgeType::ReExports,
            EdgeType::UnresolvedImport,
            EdgeType::LinksTo,
            EdgeType::Documents,
            EdgeType::LinksToSection,
            EdgeType::Mentions,
            EdgeType::DescribesPath,
            EdgeType::DocumentedBy,
            EdgeType::DocstringReferences,
            EdgeType::Cites,
            EdgeType::DerivedFrom,
            EdgeType::Contains,
            // UA-adopted
            EdgeType::Exports,
            EdgeType::ReadsFrom,
            EdgeType::WritesTo,
            EdgeType::DependsOn,
            EdgeType::TestedBy,
            EdgeType::Configures,
            EdgeType::Deploys,
            EdgeType::Provisions,
            EdgeType::Triggers,
            EdgeType::Migrates,
            EdgeType::Routes,
            EdgeType::DefinesSchema,
            EdgeType::Contradicts,
            EdgeType::BuildsOn,
            EdgeType::CategorizedUnder,
        ];
        for et in all {
            let lbl = et.label();
            let parsed = EdgeType::from_nexus_label(lbl);
            assert_eq!(
                parsed,
                Some(et),
                "EdgeType::{et:?} round-trip failed: label={lbl:?}, from_nexus_label={parsed:?}"
            );
        }
    }

    /// Legacy aliases resolve to the canonical variant (§4.2 backward-compat).
    #[test]
    fn edge_type_legacy_aliases_resolve() {
        assert_eq!(
            EdgeType::from_nexus_label("IMPORTS"),
            Some(EdgeType::ImportsFile),
            "IMPORTS alias must resolve to ImportsFile"
        );
        assert_eq!(
            EdgeType::from_nexus_label("IMPORTS_FILE"),
            Some(EdgeType::ImportsFile),
            "IMPORTS_FILE canonical must still resolve"
        );
        assert_eq!(
            EdgeType::from_nexus_label("DOCUMENTED_BY"),
            Some(EdgeType::DocumentedBy),
            "DOCUMENTED_BY canonical must resolve"
        );
        assert_eq!(
            EdgeType::from_nexus_label("CITES"),
            Some(EdgeType::Cites),
            "CITES canonical must resolve"
        );
    }

    /// from_nexus_label returns None for unknown strings (forward-compat).
    #[test]
    fn edge_type_unknown_label_returns_none() {
        assert!(EdgeType::from_nexus_label("UNKNOWN_FUTURE_EDGE").is_none());
        assert!(EdgeType::from_nexus_label("").is_none());
    }

    /// UA-mapped variants report their ua_name; Cortex-only ones return None.
    #[test]
    fn edge_type_ua_name() {
        assert_eq!(EdgeType::ImportsFile.ua_name(), Some("imports"));
        assert_eq!(EdgeType::Contradicts.ua_name(), Some("contradicts"));
        assert_eq!(EdgeType::Contains.ua_name(), Some("contains"));
        assert!(EdgeType::ImportsExternal.ua_name().is_none());
        assert!(EdgeType::UsesType.ua_name().is_none());
        assert!(EdgeType::DerivedFrom.ua_name().is_none());
    }
}
