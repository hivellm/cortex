//! Phase11k §1.3 — workspace package map.
//!
//! Wraps the workspace's `[dependencies]` (plus the
//! `external_repos.toml` cross-repo registry) so the resolver can
//! decide on tier-3 whether an unresolved import path's first
//! component matches a known external package.
//!
//! For HiveLLM-internal SDKs the entry can carry a `local_path`
//! pointing at a sibling clone (`../Vectorizer`) and a `crate_name`
//! override — a future pass uses these to walk the local clone with
//! [`super::module_map::build_rust_module_map_from_root`] and promote
//! the edge from `:IMPORTS_EXTERNAL` to `:IMPORTS_FILE` (cross-repo).
//! That promotion is plumbed through §1.4's patch builder; the map
//! itself just stores the metadata.

use std::collections::HashMap;
use std::path::PathBuf;

use cortex_storage::external_repos::ExternalRepoIndex;

/// One package the workspace declares as an external dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPackage {
    /// Canonical package name as it appears in the import path
    /// (after Cargo's `-` → `_` normalisation rules — both forms are
    /// stored in the map's index so a lookup against either string
    /// hits the same entry).
    pub name: String,
    /// Natural key used on the `:ExternalPackage` Nexus node.
    /// Defaults to `name` but external_repos.toml may override.
    pub natural_key: String,
    /// Sibling-clone path, when the package is a HiveLLM-internal
    /// SDK with a local checkout the resolver can walk.
    pub local_path: Option<PathBuf>,
    /// Crate name override — used when the dependency name and the
    /// crate's `[lib].name` differ (e.g. `vectorizer-sdk` exposes a
    /// crate named `vectorizer`).
    pub crate_name: Option<String>,
}

/// Workspace dependency map. Looked up tier-3.
#[derive(Debug, Default, Clone)]
pub struct PackageMap {
    /// Stored entries keyed by `name` (and by every alias derived
    /// from `name`/`crate_name` so an import path's first component
    /// always lands a hit when the workspace declares the package).
    by_alias: HashMap<String, ExternalPackage>,
}

impl PackageMap {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when no entries are registered.
    pub fn is_empty(&self) -> bool {
        self.by_alias.is_empty()
    }

    /// Insert one package. Aliases registered:
    ///
    /// - the canonical [`ExternalPackage::name`]
    /// - the underscore-normalised form (`vectorizer-sdk` →
    ///   `vectorizer_sdk`)
    /// - the [`ExternalPackage::crate_name`] override, when set
    pub fn insert(&mut self, pkg: ExternalPackage) {
        let canonical = pkg.name.clone();
        let underscored = canonical.replace('-', "_");
        let crate_alias = pkg.crate_name.clone();
        self.by_alias.insert(canonical.clone(), pkg.clone());
        if underscored != canonical {
            self.by_alias.insert(underscored, pkg.clone());
        }
        if let Some(c) = crate_alias {
            self.by_alias.insert(c, pkg);
        }
    }

    /// Lookup by import-path head. Tries the raw form first, then
    /// the dash-normalised form.
    pub fn find(&self, alias: &str) -> Option<&ExternalPackage> {
        if let Some(p) = self.by_alias.get(alias) {
            return Some(p);
        }
        let dashed = alias.replace('_', "-");
        if dashed != alias {
            return self.by_alias.get(&dashed);
        }
        None
    }

    /// Number of distinct packages held (counts unique
    /// [`ExternalPackage::natural_key`] strings, not aliases).
    pub fn len(&self) -> usize {
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for pkg in self.by_alias.values() {
            seen.insert(pkg.natural_key.as_str());
        }
        seen.len()
    }

    /// Construct a map from a parsed workspace `[dependencies]`
    /// table and an [`ExternalRepoIndex`]. Entries declared in the
    /// index inherit `local_path` + `crate_name`; everything else
    /// becomes a plain external package.
    pub fn from_workspace(
        dependency_names: impl IntoIterator<Item = String>,
        index: &ExternalRepoIndex,
    ) -> Self {
        let mut map = Self::new();
        for dep in dependency_names {
            let entry = if let Some(decl) = index.get(&dep) {
                ExternalPackage {
                    name: dep.clone(),
                    natural_key: dep.clone(),
                    local_path: Some(decl.local_path.clone()),
                    crate_name: decl.crate_name.clone(),
                }
            } else {
                ExternalPackage {
                    name: dep.clone(),
                    natural_key: dep,
                    local_path: None,
                    crate_name: None,
                }
            };
            map.insert(entry);
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_to_underscore_alias_resolves() {
        let mut m = PackageMap::new();
        m.insert(ExternalPackage {
            name: "vectorizer-sdk".into(),
            natural_key: "vectorizer-sdk".into(),
            local_path: None,
            crate_name: None,
        });
        assert!(m.find("vectorizer-sdk").is_some());
        assert!(m.find("vectorizer_sdk").is_some());
    }

    #[test]
    fn crate_name_override_registers_extra_alias() {
        let mut m = PackageMap::new();
        m.insert(ExternalPackage {
            name: "vectorizer-sdk".into(),
            natural_key: "vectorizer-sdk".into(),
            local_path: None,
            crate_name: Some("vectorizer".into()),
        });
        let hit = m.find("vectorizer").expect("crate_name alias must hit");
        assert_eq!(hit.natural_key, "vectorizer-sdk");
    }

    #[test]
    fn missing_alias_returns_none() {
        let m = PackageMap::new();
        assert!(m.find("does-not-exist").is_none());
    }

    #[test]
    fn from_workspace_inherits_local_path_when_declared() {
        let toml = r#"
[vectorizer-sdk]
local_path = "../Vectorizer"
crate_name = "vectorizer"
"#;
        let index = ExternalRepoIndex::from_toml_str(toml).expect("parse");
        let map = PackageMap::from_workspace(
            ["vectorizer-sdk".to_string(), "tokio".to_string()],
            &index,
        );
        let v = map.find("vectorizer").expect("alias");
        assert_eq!(v.local_path.as_deref(), Some(std::path::Path::new("../Vectorizer")));
        let t = map.find("tokio").expect("tokio");
        assert!(t.local_path.is_none());
    }
}
