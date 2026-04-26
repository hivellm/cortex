//! Cypher template registry.
//!
//! Per spec 07 §Cypher generation, every Cypher write goes through a
//! parametrized `UNWIND $rows AS row MERGE ...` template loaded from a
//! source-controlled `.cypher` file. There is no string concatenation
//! of user data — security (injection) plus auditability.
//!
//! [`CypherTemplates`] is a name-keyed registry of those templates.
//! Templates live in `crates/cortex-graph/cypher/`; one file per
//! `(label × incoming edge)` pattern. The registry is loaded once at
//! startup and held read-only for the worker's lifetime.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use thiserror::Error;

/// Failure modes raised while loading templates from disk.
#[derive(Debug, Error)]
pub enum CypherLoadError {
    /// The `cypher/` directory does not exist or cannot be read.
    #[error("cypher template directory unreadable: {0}")]
    Io(#[from] std::io::Error),
}

/// Read-only registry of Cypher templates keyed by name.
///
/// The name is the template file's stem — `tool_call.cypher` is loaded
/// under the name `tool_call`. Names are stable identifiers used by the
/// mapper / writer to pick a template per `(label × edge)` pattern.
#[derive(Debug, Clone, Default)]
pub struct CypherTemplates {
    templates: BTreeMap<String, String>,
}

impl CypherTemplates {
    /// Build a registry from an in-memory map. Useful for tests.
    pub fn from_map(templates: BTreeMap<String, String>) -> Self {
        Self { templates }
    }

    /// Look up a template by name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.templates.get(name).map(String::as_str)
    }

    /// True when the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Number of templates in the registry.
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// Iterate over `(name, body)` pairs in lexicographic order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.templates
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Load all `*.cypher` files from `path` into a registry.
///
/// File names are taken as template names (stem only). Subdirectories
/// are not descended; spec 07 calls for a flat directory.
pub fn load_from_dir(path: &Path) -> Result<CypherTemplates, CypherLoadError> {
    let mut templates: BTreeMap<String, String> = BTreeMap::new();
    if !path.exists() {
        return Ok(CypherTemplates::default());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if !entry_path.is_file() {
            continue;
        }
        let is_cypher = entry_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("cypher"))
            .unwrap_or(false);
        if !is_cypher {
            continue;
        }
        let name = match entry_path.file_stem().and_then(|s| s.to_str()) {
            Some(stem) => stem.to_string(),
            None => continue,
        };
        let body = fs::read_to_string(&entry_path)?;
        templates.insert(name, body);
    }
    Ok(CypherTemplates { templates })
}
