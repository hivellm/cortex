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
    /// One or more required templates were missing from the loaded
    /// registry. Surfaced at startup so the worker refuses to consume
    /// events without the Cypher it needs.
    #[error("missing required cypher templates: {0:?}")]
    MissingRequired(Vec<String>),
}

/// Canonical template name for a node-upsert template, derived from the
/// node label. Spec 07 §Cypher generation calls for one template per
/// label; this helper computes the registry key so callers never
/// hand-roll the convention.
///
/// Convention: `node_<label_snake>` — e.g. `Session` → `node_session`,
/// `ToolCall` → `node_tool_call`, `LawViolation` → `node_law_violation`.
pub fn node_template_name(label: &str) -> String {
    format!("node_{}", to_snake_case(label))
}

/// Canonical template name for an edge-upsert template, derived from
/// the (from-label, relationship type, to-label) triple. Mirrors
/// `node_template_name` but for edges so the registry key is stable
/// across mapper, writer, and on-disk filename.
///
/// Convention: `edge_<from_snake>__<rel_lower>__<to_snake>` — e.g.
/// (`Session`, `HAS_TURN`, `Turn`) → `edge_session__has_turn__turn`.
pub fn edge_template_name(from_label: &str, rel_type: &str, to_label: &str) -> String {
    format!(
        "edge_{}__{}__{}",
        to_snake_case(from_label),
        rel_type.to_ascii_lowercase(),
        to_snake_case(to_label),
    )
}

/// Convert a CamelCase label to snake_case. Pure ASCII because every
/// label in the Cortex graph schema (architecture §4.2) is ASCII.
fn to_snake_case(label: &str) -> String {
    let mut out = String::with_capacity(label.len() + 4);
    for (idx, ch) in label.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Required template names the worker must have loaded before it will
/// accept events. Mirrors the (label × incoming-edge) patterns the
/// mapper currently emits — additions to the mapper bring matching
/// additions to this list, otherwise the worker fails fast at startup.
pub const REQUIRED_TEMPLATES: &[&str] = &[
    // Nodes — one entry per Label currently emitted by `mapper.rs`.
    "node_session",
    "node_turn",
    "node_tool_call",
    "node_artifact",
    "node_decision",
    "node_memory",
    "node_analysis",
    "node_law_violation",
    "node_law",
    "node_repo",
    // phase4c — code symbol nodes extracted by the embedder's
    // CodeChunker on artifact events.
    "node_symbol",
    // Edges — one entry per (from, rel, to) currently emitted by
    // `mapper.rs`. Keep in lockstep with the mapper.
    "edge_session__has_turn__turn",
    "edge_session__has_tool_call__tool_call",
    "edge_turn__has_tool_call__tool_call",
    "edge_session__remembers__memory",
    "edge_artifact__in_repo__repo",
    "edge_tool_call__touched__artifact",
    "edge_turn__linked_to__decision",
    "edge_decision__supersedes__decision",
    "edge_law_violation__of__law",
    "edge_law_violation__observed_in__turn",
    "edge_law_violation__observed_in__tool_call",
    // phase4c — Symbol → Artifact via DEFINES.
    "edge_symbol__defines__artifact",
];

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
        self.templates.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Fail-fast check that every name in `required` is present.
    ///
    /// Returns the list of missing names — the worker is expected to
    /// abort startup when this list is non-empty so production never
    /// runs against a half-loaded template registry.
    pub fn missing_required<'a>(&self, required: &'a [&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|name| !self.templates.contains_key(*name))
            .collect()
    }

    /// Convenience wrapper that turns [`Self::missing_required`] into a
    /// [`CypherLoadError::MissingRequired`] error so the binary can
    /// `?`-propagate the failure straight out of `main`.
    pub fn ensure_required(&self, required: &[&str]) -> Result<(), CypherLoadError> {
        let missing = self.missing_required(required);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(CypherLoadError::MissingRequired(
                missing.into_iter().map(String::from).collect(),
            ))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_handles_camel_and_already_snake() {
        assert_eq!(to_snake_case("Session"), "session");
        assert_eq!(to_snake_case("ToolCall"), "tool_call");
        assert_eq!(to_snake_case("LawViolation"), "law_violation");
        assert_eq!(to_snake_case("Repo"), "repo");
    }

    #[test]
    fn node_template_name_uses_label_snake_case() {
        assert_eq!(node_template_name("Session"), "node_session");
        assert_eq!(node_template_name("ToolCall"), "node_tool_call");
        assert_eq!(node_template_name("LawViolation"), "node_law_violation");
    }

    #[test]
    fn edge_template_name_uses_endpoints_and_rel() {
        assert_eq!(
            edge_template_name("Session", "HAS_TURN", "Turn"),
            "edge_session__has_turn__turn"
        );
        assert_eq!(
            edge_template_name("Artifact", "IN_REPO", "Repo"),
            "edge_artifact__in_repo__repo"
        );
        assert_eq!(
            edge_template_name("Session", "HAS_TOOL_CALL", "ToolCall"),
            "edge_session__has_tool_call__tool_call"
        );
    }

    #[test]
    fn missing_required_lists_absent_names() {
        let mut map = BTreeMap::new();
        map.insert("node_session".to_string(), "MERGE (n:Session)".to_string());
        let reg = CypherTemplates::from_map(map);
        let missing = reg.missing_required(&["node_session", "node_turn"]);
        assert_eq!(missing, vec!["node_turn"]);
    }

    #[test]
    fn ensure_required_errors_when_anything_missing() {
        let reg = CypherTemplates::default();
        let err = reg.ensure_required(&["node_session"]).unwrap_err();
        match err {
            CypherLoadError::MissingRequired(names) => {
                assert_eq!(names, vec!["node_session".to_string()]);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn ensure_required_passes_with_full_registry() {
        let mut map = BTreeMap::new();
        for name in REQUIRED_TEMPLATES {
            map.insert((*name).to_string(), "MERGE (n)".to_string());
        }
        let reg = CypherTemplates::from_map(map);
        reg.ensure_required(REQUIRED_TEMPLATES)
            .expect("registry should satisfy REQUIRED_TEMPLATES");
    }

    #[test]
    fn shipped_cypher_dir_satisfies_required_set() {
        // The on-disk template set in `crates/cortex-graph/cypher/`
        // must always satisfy `REQUIRED_TEMPLATES` — otherwise the
        // worker fails fast at startup. This test catches regressions
        // before they reach runtime.
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let registry =
            load_from_dir(&manifest_dir.join("cypher")).expect("load shipped cypher dir");
        registry
            .ensure_required(REQUIRED_TEMPLATES)
            .expect("shipped cypher/ must satisfy REQUIRED_TEMPLATES");
    }

    #[test]
    fn shipped_query_templates_for_pre_thinking_renderer() {
        // Phase11k §6.1 — three read-side query templates ship in
        // cypher/ for the pre-thinking renderer's graph-traversal
        // sub-blocks: code_callers (CALLS 1-hop), doc_trail (CITES
        // chain), blast_radius (IMPORTS_FILE*1..2). The renderer
        // loads them via the same template registry the writer
        // uses; a regression in their on-disk shape would break the
        // `Past sessions` / `Consolidated context` augmentation.
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cypher_dir = manifest_dir.join("cypher");
        for (name, must_contain) in [
            ("code_callers", &["MATCH", "CALLS", "$target_id"][..]),
            ("doc_trail", &["MATCH", "CITES", "$root_id"][..]),
            ("blast_radius", &["MATCH", "IMPORTS_FILE", "$source_id"][..]),
        ] {
            let path = cypher_dir.join(format!("{name}.cypher"));
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            for needle in must_contain {
                assert!(
                    body.contains(needle),
                    "{name}.cypher must contain {needle:?}, got:\n{body}"
                );
            }
            // Every read-side template must include a $limit
            // parameter so the renderer can cap result size — the
            // pre-thinking budget is a hard constraint.
            assert!(
                body.contains("$limit"),
                "{name}.cypher must accept a $limit parameter"
            );
        }
    }

    #[test]
    fn shipped_node_templates_use_external_id_create_on_conflict_match() {
        // Phase11l §3.3 — every shipped node_*.cypher template MUST
        // route the upsert through Nexus 2.1's reserved `_id` slot
        // (`CREATE (n:Label {_id: row.key}) ON CONFLICT MATCH`). The
        // legacy `MERGE { natural_key | id | name: row.key }` shape is
        // a regression: it bypasses the external-id index seek, costs
        // the per-template hash-vs-property comparison, and reintroduces
        // the silent-overwrite class the §2 ConflictPolicy enum was
        // designed to make explicit.
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cypher_dir = manifest_dir.join("cypher");
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&cypher_dir)
            .expect("read cypher/ dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("node_") && n.ends_with(".cypher"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        assert!(
            !entries.is_empty(),
            "cypher/ must ship at least one node_*.cypher template"
        );
        for path in entries {
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                body.contains("_id: row.key"),
                "{name}: must key on Nexus reserved `_id` slot, got:\n{body}"
            );
            assert!(
                body.contains("ON CONFLICT MATCH"),
                "{name}: must carry an explicit `ON CONFLICT MATCH` modifier, got:\n{body}"
            );
            assert!(
                body.contains("CREATE (n:"),
                "{name}: must use CREATE form, got:\n{body}"
            );
            assert!(
                !body.contains("MERGE (n:"),
                "{name}: legacy MERGE shape regressed, got:\n{body}"
            );
        }
    }
}
