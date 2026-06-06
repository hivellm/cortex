//! phase18 §5.1 — CROSS_PROJECT_REF backfill from manifests + ADR/doc version mentions.
//!
//! Two source families per proposal P4:
//!
//! 1. **Manifest deps** (Cargo.toml / package.json / lockfiles) →
//!    coarse project-anchor edges:
//!    `Branch{<this>:main}` --CROSS_PROJECT_REF--> `Branch{<dep>:main}`.
//! 2. **ADR / doc version mentions** → provenance edges:
//!    `Decision{<key>}` --CROSS_PROJECT_REF--> `Branch{<dep>:main}`.
//!
//! The heavy lifting lives here; the CLI wrapper is a thin caller
//! that lives in `cortex-cli` (shipped in a later phase item).

use std::collections::BTreeMap;

use regex::Regex;

use super::branch::BRANCH_LABEL;
use super::patch::EdgeOp;
use super::temporal_edges::build_cross_project_ref;

// ── Sibling-project registry ──────────────────────────────────────────────────

/// Maps a dependency name (Cargo crate or npm package) to the HiveLLM sibling
/// `project_id` it represents. Lower-cased project ids match the
/// `derive_project_id` convention in `bitemporal.rs`.
const SIBLING_DEPENDENCIES: &[(&str, &str)] = &[
    ("nexus-graph-sdk", "nexus"),
    ("vectorizer-sdk", "vectorizer"),
    ("synap-sdk", "synap"),
    ("lexum-sdk", "lexum"),
    ("expert-sdk", "expert"),
    ("@hivellm/nexus", "nexus"),
    ("@hivellm/vectorizer", "vectorizer"),
    ("@hivellm/synap", "synap"),
    ("@hivellm/lexum", "lexum"),
    ("@hivellm/expert", "expert"),
];

/// Project ids scanned for in free-text ADR/doc version mentions.
const SIBLING_PROJECTS: &[&str] = &["nexus", "vectorizer", "synap", "lexum", "expert"];

/// Look up the HiveLLM sibling project id for a dependency name.
///
/// Matching is case-insensitive on the dependency name side. Returns
/// `None` for unknown / non-sibling dependencies.
pub fn project_for_dependency(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    SIBLING_DEPENDENCIES
        .iter()
        .find(|(dep, _)| dep.to_lowercase() == lower)
        .map(|(_, project)| *project)
}

// ── Public types ──────────────────────────────────────────────────────────────

/// One discovered cross-project dependency edge candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossProjectDep {
    /// The raw dependency name as it appears in the manifest (e.g. `"nexus-graph-sdk"`).
    pub dep_name: String,
    /// The resolved HiveLLM sibling project id (e.g. `"nexus"`).
    pub to_project: String,
    /// Version string extracted from the manifest (e.g. `"2.1"`).
    pub version_constraint: String,
}

/// Aggregated backfill run statistics.
///
/// This is a plain data-transfer object. The CLI layer fills and logs it;
/// no business logic lives inside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrossProjectBackfillReport {
    /// Number of manifest-derived edges produced before deduplication.
    pub manifest_edges: usize,
    /// Number of ADR-mention edges produced before deduplication.
    pub adr_edges: usize,
    /// Total edges surviving deduplication.
    pub deduped_total: usize,
}

// ── Manifest parsers ──────────────────────────────────────────────────────────

/// Parse a `Cargo.toml` source string and return every sibling dependency
/// found in `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
/// and `[workspace.dependencies]`.
///
/// - A plain string value (`nexus-graph-sdk = "2.1"`) is used verbatim.
/// - An inline-table value uses its `version` field
///   (`nexus-graph-sdk = { version = "2.1", features = [] }`).
/// - Entries with no resolvable version or whose key does not map to a
///   sibling project are silently skipped.
/// - Any `toml` parse failure returns an empty `Vec` without panicking.
pub fn parse_cargo_deps(toml_src: &str) -> Vec<CrossProjectDep> {
    let value: toml::Value = match toml::from_str(toml_src) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let tables_to_check: &[&[&str]] = &[
        &["dependencies"],
        &["dev-dependencies"],
        &["build-dependencies"],
        &["workspace", "dependencies"],
    ];

    let mut deps: Vec<CrossProjectDep> = Vec::new();

    for path in tables_to_check {
        let table = match navigate_toml(&value, path) {
            Some(toml::Value::Table(t)) => t,
            _ => continue,
        };

        for (key, val) in table {
            let version = match val {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => match t.get("version") {
                    Some(toml::Value::String(s)) => s.clone(),
                    _ => continue,
                },
                _ => continue,
            };
            if let Some(project) = project_for_dependency(key) {
                deps.push(CrossProjectDep {
                    dep_name: key.clone(),
                    to_project: project.to_string(),
                    version_constraint: version,
                });
            }
        }
    }

    deps
}

/// Walk a dotted key path through a `toml::Value` tree.
fn navigate_toml<'a>(mut v: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    for key in path {
        v = v.as_table()?.get(*key)?;
    }
    Some(v)
}

/// Parse a `package.json` source string and return every sibling dependency
/// found in `dependencies` and `devDependencies`.
///
/// Version range prefixes (`^`, `~`, `>=`, `=`) are stripped so that
/// `"^2.1.0"` becomes `"2.1.0"`. Any `serde_json` parse failure returns an
/// empty `Vec` without panicking.
pub fn parse_package_json(json_src: &str) -> Vec<CrossProjectDep> {
    let obj: serde_json::Value = match serde_json::from_str(json_src) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let sections = ["dependencies", "devDependencies"];
    let mut deps: Vec<CrossProjectDep> = Vec::new();

    for section in &sections {
        let map = match obj.get(*section).and_then(|v| v.as_object()) {
            Some(m) => m,
            None => continue,
        };
        for (key, val) in map {
            let raw = match val.as_str() {
                Some(s) => s,
                None => continue,
            };
            if let Some(project) = project_for_dependency(key) {
                let version = strip_version_prefix(raw);
                deps.push(CrossProjectDep {
                    dep_name: key.clone(),
                    to_project: project.to_string(),
                    version_constraint: version.to_string(),
                });
            }
        }
    }

    deps
}

/// Strip common npm version range prefixes from a version string.
///
/// Handles `^`, `~`, `>=`, `=` at the start of the string.
fn strip_version_prefix(v: &str) -> &str {
    let v = v.trim_start_matches('=');
    let v = v.trim_start_matches(">=");
    let v = v.trim_start_matches('^');
    let v = v.trim_start_matches('~');
    v
}

// ── ADR / doc text scanner ────────────────────────────────────────────────────

/// Scan free-text (ADR body, doc prose) for cross-project version mentions.
///
/// For each sibling project in [`SIBLING_PROJECTS`] the scanner matches
/// the project name followed within ~16 characters by a semver-ish version
/// (`MAJOR.MINOR[.PATCH]`). Returns deduplicated `(project_id, version)`
/// pairs in the order they are first encountered.
///
/// Example triggers:
/// - `"pinned nexus-graph-sdk to 2.1 in the lockfile"` → `("nexus", "2.1")`
/// - `"Vectorizer 3.3.0 shipped"` → `("vectorizer", "3.3.0")`
pub fn scan_version_mentions(text: &str) -> Vec<(String, String)> {
    // Build one Regex per project: `(?i)\b<project>\b[^\n0-9]{0,16}?v?(\d+\.\d+(?:\.\d+)?)`
    let patterns: Vec<(&str, Regex)> = SIBLING_PROJECTS
        .iter()
        .filter_map(|&proj| {
            let pat = format!(
                r"(?i)\b{proj}\b[^\n0-9]{{0,16}}?v?(\d+\.\d+(?:\.\d+)?)"
            );
            Regex::new(&pat).ok().map(|re| (proj, re))
        })
        .collect();

    let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();
    let mut result: Vec<(String, String)> = Vec::new();

    for (proj, re) in &patterns {
        for cap in re.captures_iter(text) {
            if let Some(ver) = cap.get(1) {
                let key = (proj.to_string(), ver.as_str().to_string());
                if seen.insert(key.clone(), ()).is_none() {
                    result.push(key);
                }
            }
        }
    }

    result
}

// ── Edge builders ─────────────────────────────────────────────────────────────

/// Build a `CROSS_PROJECT_REF` edge from `<this_project>:main` to
/// `<dep.to_project>:main` using the dep's `version_constraint`.
pub fn manifest_edge(this_project: &str, dep: &CrossProjectDep, valid_from: &str) -> EdgeOp {
    build_cross_project_ref(
        BRANCH_LABEL,
        &format!("{this_project}:main"),
        BRANCH_LABEL,
        &format!("{}:main", dep.to_project),
        &dep.version_constraint,
        valid_from,
        None,
    )
}

/// Build a `CROSS_PROJECT_REF` edge from a `Decision` node to
/// `<to_project>:main`, carrying the version the ADR cites.
pub fn adr_mention_edge(
    decision_key: &str,
    to_project: &str,
    version: &str,
    valid_from: &str,
) -> EdgeOp {
    build_cross_project_ref(
        "Decision",
        decision_key,
        BRANCH_LABEL,
        &format!("{to_project}:main"),
        version,
        valid_from,
        None,
    )
}

/// Stable-dedup a `Vec<EdgeOp>` by the tuple
/// `(from_label, from_key, to_label, to_key, version_constraint prop)`.
///
/// First-seen order is preserved. Edges without a `version_constraint`
/// prop are treated as having an empty version string for dedup purposes.
pub fn dedup_edges(edges: Vec<EdgeOp>) -> Vec<EdgeOp> {
    let mut seen: BTreeMap<(String, String, String, String, String), ()> = BTreeMap::new();
    let mut result: Vec<EdgeOp> = Vec::new();

    for edge in edges {
        let vc = edge
            .props
            .get("version_constraint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let key = (
            edge.from_label.clone(),
            edge.from_key.clone(),
            edge.to_label.clone(),
            edge.to_key.clone(),
            vc,
        );
        if seen.insert(key, ()).is_none() {
            result.push(edge);
        }
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::temporal_edges::CROSS_PROJECT_REF;

    #[test]
    fn project_for_dependency_maps_known_and_rejects_unknown() {
        assert_eq!(project_for_dependency("nexus-graph-sdk"), Some("nexus"));
        assert_eq!(
            project_for_dependency("@hivellm/vectorizer"),
            Some("vectorizer")
        );
        assert_eq!(project_for_dependency("serde"), None);
        // case-insensitive
        assert_eq!(
            project_for_dependency("Nexus-Graph-SDK"),
            Some("nexus")
        );
    }

    #[test]
    fn parse_cargo_deps_handles_string_and_inline_table() {
        let src = r#"
[workspace.dependencies]
nexus-graph-sdk = "2.1"
serde = "1"

[dependencies]
vectorizer-sdk = { version = "3.3.0", features = ["full"] }
anyhow = "1"
"#;
        let deps = parse_cargo_deps(src);
        assert_eq!(deps.len(), 2, "expected nexus + vectorizer, got {:?}", deps);

        let nexus = deps.iter().find(|d| d.dep_name == "nexus-graph-sdk").unwrap();
        assert_eq!(nexus.to_project, "nexus");
        assert_eq!(nexus.version_constraint, "2.1");

        let vec_dep = deps.iter().find(|d| d.dep_name == "vectorizer-sdk").unwrap();
        assert_eq!(vec_dep.to_project, "vectorizer");
        assert_eq!(vec_dep.version_constraint, "3.3.0");
    }

    #[test]
    fn parse_cargo_deps_empty_on_garbage() {
        let deps = parse_cargo_deps("this is NOT toml ][[ @@@@");
        assert!(deps.is_empty(), "expected empty, got {:?}", deps);
    }

    #[test]
    fn parse_package_json_reads_deps_and_devdeps_and_strips_range() {
        let src = r#"{
            "dependencies": { "@hivellm/nexus": "^2.1.0" },
            "devDependencies": { "@hivellm/synap": "~0.12.0" }
        }"#;
        let deps = parse_package_json(src);
        assert_eq!(deps.len(), 2, "expected nexus + synap, got {:?}", deps);

        let nexus = deps.iter().find(|d| d.dep_name == "@hivellm/nexus").unwrap();
        assert_eq!(nexus.to_project, "nexus");
        assert_eq!(nexus.version_constraint, "2.1.0");

        let synap = deps.iter().find(|d| d.dep_name == "@hivellm/synap").unwrap();
        assert_eq!(synap.to_project, "synap");
        assert_eq!(synap.version_constraint, "0.12.0");
    }

    #[test]
    fn scan_version_mentions_finds_project_version() {
        let text = "pinned nexus-graph-sdk to 2.1 in the lockfile. Also Vectorizer 3.3.0 shipped.";
        let mentions = scan_version_mentions(text);

        assert!(
            mentions.contains(&("nexus".to_string(), "2.1".to_string())),
            "expected (nexus, 2.1) in {:?}",
            mentions
        );
        assert!(
            mentions.contains(&("vectorizer".to_string(), "3.3.0".to_string())),
            "expected (vectorizer, 3.3.0) in {:?}",
            mentions
        );
    }

    #[test]
    fn manifest_edge_composes_main_branch_keys() {
        let dep = CrossProjectDep {
            dep_name: "nexus-graph-sdk".to_string(),
            to_project: "nexus".to_string(),
            version_constraint: "2.1".to_string(),
        };
        let valid_from = "2026-05-29T07:00:00Z";
        let edge = manifest_edge("cortex", &dep, valid_from);

        assert_eq!(edge.edge_type, CROSS_PROJECT_REF);
        assert_eq!(edge.from_label, BRANCH_LABEL);
        assert_eq!(edge.from_key, "cortex:main");
        assert_eq!(edge.to_label, BRANCH_LABEL);
        assert_eq!(edge.to_key, "nexus:main");
        assert_eq!(
            edge.props.get("version_constraint").and_then(|v| v.as_str()),
            Some("2.1")
        );
        assert_eq!(
            edge.props
                .get(crate::graph::bitemporal::keys::VALID_FROM)
                .and_then(|v| v.as_str()),
            Some(valid_from)
        );
    }

    #[test]
    fn dedup_edges_collapses_identical_pairs() {
        let dep = CrossProjectDep {
            dep_name: "nexus-graph-sdk".to_string(),
            to_project: "nexus".to_string(),
            version_constraint: "2.1".to_string(),
        };
        let valid_from = "2026-05-29T07:00:00Z";

        let e1 = manifest_edge("cortex", &dep, valid_from);
        let e2 = manifest_edge("cortex", &dep, valid_from);

        let dep_v2 = CrossProjectDep {
            dep_name: "nexus-graph-sdk".to_string(),
            to_project: "nexus".to_string(),
            version_constraint: "2.2".to_string(),
        };
        let e3 = manifest_edge("cortex", &dep_v2, valid_from);

        let result = dedup_edges(vec![e1, e2, e3]);
        assert_eq!(result.len(), 2, "expected 2 distinct edges, got {:?}", result);

        assert_eq!(
            result[0]
                .props
                .get("version_constraint")
                .and_then(|v| v.as_str()),
            Some("2.1")
        );
        assert_eq!(
            result[1]
                .props
                .get("version_constraint")
                .and_then(|v| v.as_str()),
            Some("2.2")
        );
    }
}
