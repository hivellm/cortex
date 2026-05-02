//! Phase11k §1.3 — cross-repo SDK declarations.
//!
//! Operators declare HiveLLM-internal SDKs whose source code lives in
//! a sibling repo (e.g. `vectorizer-sdk` → `../Vectorizer`) in a TOML
//! file. The graph resolver uses these declarations to promote
//! `:IMPORTS_EXTERNAL` edges to `:IMPORTS_FILE` (cross-repo) — see
//! `crates/cortex-workers/src/graph/resolver/package_map.rs` and
//! `docs/analysis/graph/04-extraction-pipeline.md` §4.2.
//!
//! TOML shape:
//!
//! ```toml
//! [vectorizer-sdk]
//! local_path = "../Vectorizer"
//! crate_name = "vectorizer"
//!
//! [nexus-graph-sdk]
//! local_path = "../Nexus"
//! crate_name = "nexus_graph"
//! ```
//!
//! - `local_path` (required) — repo-relative path to the sibling
//!   clone. The resolver walks the clone with the same module-map
//!   builder it uses for the host workspace.
//! - `crate_name` (optional) — the crate's `[lib].name` when it
//!   diverges from the dependency name. Used as a search alias on
//!   the package map.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One declared sibling SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRepoDecl {
    /// Path on disk where the sibling repo is checked out.
    pub local_path: PathBuf,
    /// Optional override for the actual crate name inside the
    /// sibling (when the dependency name diverges from the
    /// `[lib].name` — e.g. `vectorizer-sdk` → `vectorizer`).
    pub crate_name: Option<String>,
}

/// Parsed `external_repos.toml` keyed by the declared package name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExternalRepoIndex {
    entries: BTreeMap<String, ExternalRepoDecl>,
}

impl ExternalRepoIndex {
    /// Empty index — nothing declared.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read + parse from `path`. Missing file produces an empty
    /// index (treated as soft-degrade per §05 risk register).
    pub fn load_from_file(path: &Path) -> Result<Self, ExternalRepoLoadError> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let raw = std::fs::read_to_string(path).map_err(ExternalRepoLoadError::Io)?;
        Self::from_toml_str(&raw)
    }

    /// Parse from a TOML string. Used by tests and any caller that
    /// has already pulled the bytes (e.g. through CAS).
    pub fn from_toml_str(toml_text: &str) -> Result<Self, ExternalRepoLoadError> {
        let parsed: BTreeMap<String, RawDecl> =
            toml::from_str(toml_text).map_err(ExternalRepoLoadError::Toml)?;
        let mut entries = BTreeMap::new();
        for (name, raw) in parsed {
            let decl = ExternalRepoDecl {
                local_path: PathBuf::from(raw.local_path),
                crate_name: raw.crate_name,
            };
            entries.insert(name, decl);
        }
        Ok(Self { entries })
    }

    /// Look up a declared sibling SDK by its dependency name.
    pub fn get(&self, name: &str) -> Option<&ExternalRepoDecl> {
        self.entries.get(name)
    }

    /// Iterate all declarations.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ExternalRepoDecl)> {
        self.entries.iter()
    }

    /// Number of declared entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no SDK is declared.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct RawDecl {
    local_path: String,
    #[serde(default)]
    crate_name: Option<String>,
}

/// Errors raised while loading [`ExternalRepoIndex`].
#[derive(Debug, thiserror::Error)]
pub enum ExternalRepoLoadError {
    /// I/O error reading the TOML file.
    #[error("read error: {0}")]
    Io(#[from] std::io::Error),
    /// Parser rejected the TOML.
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_entries_round_trip() {
        let src = r#"
[vectorizer-sdk]
local_path = "../Vectorizer"
crate_name = "vectorizer"

[nexus-graph-sdk]
local_path = "../Nexus"
"#;
        let idx = ExternalRepoIndex::from_toml_str(src).expect("parse");
        assert_eq!(idx.len(), 2);
        let v = idx.get("vectorizer-sdk").expect("vectorizer-sdk");
        assert_eq!(v.local_path, PathBuf::from("../Vectorizer"));
        assert_eq!(v.crate_name.as_deref(), Some("vectorizer"));
        let n = idx.get("nexus-graph-sdk").expect("nexus-graph-sdk");
        assert_eq!(n.local_path, PathBuf::from("../Nexus"));
        assert!(n.crate_name.is_none());
    }

    #[test]
    fn missing_file_yields_empty_index() {
        let p = std::path::Path::new("does-not-exist.toml");
        let idx = ExternalRepoIndex::load_from_file(p).expect("missing file is soft");
        assert!(idx.is_empty());
    }

    #[test]
    fn malformed_toml_returns_error() {
        let err = ExternalRepoIndex::from_toml_str("not valid = toml = here").unwrap_err();
        assert!(matches!(err, ExternalRepoLoadError::Toml(_)));
    }
}
