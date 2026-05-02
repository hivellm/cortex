//! Phase11k §1.3 — three-tier symbol resolver.
//!
//! Maps an unresolved [`crate::graph::analyzer::ResolutionTarget`] to
//! a concrete [`ResolvedTarget`] backed by either a workspace
//! `:Symbol` + `:Artifact`, an `:ExternalPackage`, or the sentinel
//! `:UnresolvedImport` fallback. The tiers (in dispatch order):
//!
//! 1. **Local-file symbol table** ([`ResolutionTier::LocalFile`]) —
//!    bare identifiers checked against the per-file
//!    [`LocalSymbols::by_name`] map.
//! 2. **Intra-crate module map** ([`ResolutionTier::IntraCrate`]) —
//!    scoped paths walked through [`ModuleMap`], with a fallback
//!    basename lookup for bare names that have a single workspace-
//!    wide match.
//! 3. **External package map** ([`ResolutionTier::External`]) — path
//!    heads matched against the workspace's `[dependencies]` tree
//!    and the `external_repos.toml` declarations from
//!    [`cortex_storage::external_repos`].
//! 4. **Unresolved fallback** ([`ResolutionTier::Unresolved`]) — every
//!    other case becomes an [`ResolvedTarget::Unresolved`] so the
//!    patch builder can attach an `:UNRESOLVED_IMPORT` edge.

pub mod intra_doc;
pub mod module_map;
pub mod package_map;

pub use module_map::{
    build_rust_module_map_from_root, extract_rust_module_entries, ModuleEntry, ModuleMap,
};
pub use package_map::{ExternalPackage, PackageMap};

use std::collections::BTreeMap;

use crate::graph::analyzer::{artifact_logical_key, NodeRef, ResolutionTarget};
use crate::graph::identity::symbol_natural_key;

/// Which tier resolved (or failed to resolve) a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionTier {
    /// Tier-1 — same-file symbol table.
    LocalFile,
    /// Tier-2 — workspace [`ModuleMap`].
    IntraCrate,
    /// Tier-3 — workspace `[dependencies]` + `external_repos.toml`.
    External,
    /// Fallback — nothing matched; routed to `:UnresolvedImport`.
    Unresolved,
}

/// Outcome of dispatching a [`ResolutionTarget`] through the resolver.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedTarget {
    /// Symbol that lives somewhere in the workspace. The owning
    /// artifact rides along so the patch builder can emit the
    /// `(:Artifact)-[:IMPORTS_FILE]->(:Artifact)` shape alongside the
    /// `(:Artifact)-[*]->(:Symbol)` one.
    Workspace {
        /// The resolved `:Symbol` node.
        symbol: NodeRef,
        /// The artifact that owns the symbol.
        artifact: NodeRef,
        /// Which tier matched.
        tier: ResolutionTier,
    },
    /// Resolved to an external package declared in the workspace's
    /// dependency manifest (or the cross-repo registry).
    ExternalPackage {
        /// The `:ExternalPackage` node (natural key = canonical
        /// package name).
        node: NodeRef,
    },
    /// Nothing matched — the patch builder will attach an
    /// `:UNRESOLVED_IMPORT` edge whose target is a sentinel node
    /// keyed on [`hint`](Self::Unresolved::hint).
    Unresolved {
        /// Raw target identifier joined with `::` for diagnostics.
        hint: String,
    },
    /// Caller already produced a [`NodeRef`] — pass through.
    PreResolved {
        /// The fully-formed target node.
        node: NodeRef,
    },
}

/// Per-file symbol table consulted on tier-1.
#[derive(Debug, Clone)]
pub struct LocalSymbols {
    /// Maps a bare identifier defined in the current file to its
    /// fully qualified name (`helper` → `crate::module_a::helper`).
    pub by_name: BTreeMap<String, String>,
    /// Repo this file belongs to.
    pub repo: String,
    /// Source language identifier (matches
    /// [`crate::graph::analyzer::AnalyzerLanguage::label`]).
    pub language: &'static str,
    /// Repo-relative path to the file.
    pub artifact_path: String,
}

impl LocalSymbols {
    /// Construct an empty symbol table for `(repo, language, path)`.
    pub fn new(repo: impl Into<String>, language: &'static str, path: impl Into<String>) -> Self {
        Self {
            by_name: BTreeMap::new(),
            repo: repo.into(),
            language,
            artifact_path: path.into(),
        }
    }

    /// Insert one bare-name → qualified-name mapping. Returns the
    /// previous binding (if any) so callers can detect shadowing.
    pub fn insert(&mut self, bare: impl Into<String>, qualified: impl Into<String>) -> Option<String> {
        self.by_name.insert(bare.into(), qualified.into())
    }
}

/// Three-tier resolver. Holds borrowed references to the workspace's
/// shared maps so a single resolver can dispatch many edges per call.
pub struct SymbolResolver<'a> {
    /// Workspace-wide module map (tier-2).
    pub module_map: &'a ModuleMap,
    /// Workspace dependency map (tier-3).
    pub package_map: &'a PackageMap,
    /// Per-file symbol table (tier-1).
    pub local_symbols: &'a LocalSymbols,
}

impl<'a> SymbolResolver<'a> {
    /// Build a resolver bound to the three input tables.
    pub fn new(
        module_map: &'a ModuleMap,
        package_map: &'a PackageMap,
        local_symbols: &'a LocalSymbols,
    ) -> Self {
        Self {
            module_map,
            package_map,
            local_symbols,
        }
    }

    /// Dispatch a single target against the three tiers in order.
    pub fn resolve(&self, target: &ResolutionTarget) -> ResolvedTarget {
        match target {
            ResolutionTarget::SymbolName(name) => self.resolve_bare_name(name),
            ResolutionTarget::ModulePath(path) => self.resolve_module_path(path),
            ResolutionTarget::ExternalPackage { name } => ResolvedTarget::ExternalPackage {
                node: NodeRef {
                    label: "ExternalPackage".into(),
                    natural_key: name.clone(),
                },
            },
            ResolutionTarget::Resolved(node) => ResolvedTarget::PreResolved {
                node: node.clone(),
            },
        }
    }

    fn resolve_bare_name(&self, name: &str) -> ResolvedTarget {
        if let Some(qualified) = self.local_symbols.by_name.get(name) {
            return ResolvedTarget::Workspace {
                symbol: NodeRef {
                    label: "Symbol".into(),
                    natural_key: symbol_natural_key(
                        &self.local_symbols.repo,
                        self.local_symbols.language,
                        qualified,
                    ),
                },
                artifact: NodeRef {
                    label: "Artifact".into(),
                    natural_key: artifact_logical_key(
                        &self.local_symbols.repo,
                        &self.local_symbols.artifact_path,
                    ),
                },
                tier: ResolutionTier::LocalFile,
            };
        }
        if let Some(entry) = self.module_map.find_by_basename(name) {
            return ResolvedTarget::Workspace {
                symbol: NodeRef {
                    label: "Symbol".into(),
                    natural_key: symbol_natural_key(&entry.repo, entry.language, &entry.qualified_name),
                },
                artifact: NodeRef {
                    label: "Artifact".into(),
                    natural_key: artifact_logical_key(&entry.repo, &entry.artifact_path),
                },
                tier: ResolutionTier::IntraCrate,
            };
        }
        ResolvedTarget::Unresolved {
            hint: name.to_string(),
        }
    }

    fn resolve_module_path(&self, path: &[String]) -> ResolvedTarget {
        if path.is_empty() {
            return ResolvedTarget::Unresolved {
                hint: String::new(),
            };
        }
        if let Some(entry) = self.module_map.resolve_path(path) {
            return ResolvedTarget::Workspace {
                symbol: NodeRef {
                    label: "Symbol".into(),
                    natural_key: symbol_natural_key(&entry.repo, entry.language, &entry.qualified_name),
                },
                artifact: NodeRef {
                    label: "Artifact".into(),
                    natural_key: artifact_logical_key(&entry.repo, &entry.artifact_path),
                },
                tier: ResolutionTier::IntraCrate,
            };
        }
        if let Some(head) = path.first() {
            if let Some(pkg) = self.package_map.find(head) {
                return ResolvedTarget::ExternalPackage {
                    node: NodeRef {
                        label: "ExternalPackage".into(),
                        natural_key: pkg.natural_key.clone(),
                    },
                };
            }
        }
        ResolvedTarget::Unresolved {
            hint: path.join("::"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (ModuleMap, PackageMap, LocalSymbols) {
        let mut mm = ModuleMap::new();
        mm.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::module_a::helper".into(),
            artifact_path: "crates/foo/src/module_a.rs".into(),
        });
        mm.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::module_a::Special".into(),
            artifact_path: "crates/foo/src/module_a.rs".into(),
        });

        let mut pm = PackageMap::new();
        pm.insert(ExternalPackage {
            name: "tokio".into(),
            natural_key: "tokio".into(),
            local_path: None,
            crate_name: None,
        });
        pm.insert(ExternalPackage {
            name: "vectorizer-sdk".into(),
            natural_key: "vectorizer-sdk".into(),
            local_path: Some("../Vectorizer".into()),
            crate_name: Some("vectorizer".into()),
        });

        let mut ls = LocalSymbols::new("cortex", "rust", "crates/foo/src/lib.rs");
        ls.insert("helper", "crate::lib::helper");
        (mm, pm, ls)
    }

    #[test]
    fn tier1_local_bare_name_hits_local_symbol_table() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::SymbolName("helper".into()));
        match out {
            ResolvedTarget::Workspace { tier, symbol, artifact } => {
                assert_eq!(tier, ResolutionTier::LocalFile);
                assert_eq!(symbol.natural_key, "cortex|rust|crate::lib::helper");
                assert_eq!(artifact.natural_key, "cortex|crates/foo/src/lib.rs");
            }
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[test]
    fn tier2_module_path_resolves_via_module_map() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::ModulePath(vec![
            "crate".into(),
            "module_a".into(),
            "helper".into(),
        ]));
        match out {
            ResolvedTarget::Workspace { tier, symbol, .. } => {
                assert_eq!(tier, ResolutionTier::IntraCrate);
                assert_eq!(symbol.natural_key, "cortex|rust|crate::module_a::helper");
            }
            other => panic!("expected Workspace IntraCrate, got {other:?}"),
        }
    }

    #[test]
    fn tier2_basename_fallback_for_unique_match() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::SymbolName("Special".into()));
        match out {
            ResolvedTarget::Workspace { tier, symbol, .. } => {
                assert_eq!(tier, ResolutionTier::IntraCrate);
                assert_eq!(symbol.natural_key, "cortex|rust|crate::module_a::Special");
            }
            other => panic!("expected Workspace IntraCrate, got {other:?}"),
        }
    }

    #[test]
    fn tier3_external_package_root_matches() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::ModulePath(vec![
            "tokio".into(),
            "spawn".into(),
        ]));
        match out {
            ResolvedTarget::ExternalPackage { node } => {
                assert_eq!(node.label, "ExternalPackage");
                assert_eq!(node.natural_key, "tokio");
            }
            other => panic!("expected ExternalPackage, got {other:?}"),
        }
    }

    #[test]
    fn tier3_external_package_variant_passthrough() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::ExternalPackage {
            name: "serde".into(),
        });
        match out {
            ResolvedTarget::ExternalPackage { node } => {
                assert_eq!(node.natural_key, "serde");
            }
            other => panic!("expected ExternalPackage, got {other:?}"),
        }
    }

    #[test]
    fn unresolved_when_nothing_matches() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::SymbolName("does_not_exist".into()));
        assert!(matches!(out, ResolvedTarget::Unresolved { .. }));
    }

    #[test]
    fn unresolved_module_path_carries_joined_hint() {
        let (mm, pm, ls) = fixtures();
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::ModulePath(vec![
            "no_such_crate".into(),
            "Frobulator".into(),
        ]));
        match out {
            ResolvedTarget::Unresolved { hint } => {
                assert_eq!(hint, "no_such_crate::Frobulator");
            }
            other => panic!("expected Unresolved, got {other:?}"),
        }
    }

    #[test]
    fn local_table_wins_over_module_map_basename() {
        let (mut mm, pm, ls) = fixtures();
        mm.insert(ModuleEntry {
            repo: "cortex".into(),
            language: "rust",
            qualified_name: "crate::other_mod::helper".into(),
            artifact_path: "crates/foo/src/other_mod.rs".into(),
        });
        let r = SymbolResolver::new(&mm, &pm, &ls);
        let out = r.resolve(&ResolutionTarget::SymbolName("helper".into()));
        match out {
            ResolvedTarget::Workspace { tier, .. } => {
                assert_eq!(
                    tier,
                    ResolutionTier::LocalFile,
                    "tier-1 must always beat tier-2 even when both could match"
                );
            }
            other => panic!("expected Workspace LocalFile, got {other:?}"),
        }
    }
}
