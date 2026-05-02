//! Phase11k §1.6 — resolver integration test.
//!
//! Eight cases pinning the three-tier dispatch surface plus the
//! `:UNRESOLVED_IMPORT` fallback. Each test drives the analyzer →
//! resolver → patch builder pipeline end-to-end against an in-memory
//! `ModuleMap` + `PackageMap` so the wire shape (`EdgeOp.edge_type`,
//! `to_label`, `to_key`, `props.tier`) is what the live writer would
//! see.

use cortex_workers::graph::analyzer::{
    build_graph_patch, CodeAnalyzer, PatchBuildContext, RustAnalyzer,
};
use cortex_workers::graph::patch::EdgeOp;
use cortex_workers::graph::resolver::{
    ExternalPackage, LocalSymbols, ModuleEntry, ModuleMap, PackageMap, SymbolResolver,
};

fn no_hash_lookup(_repo: &str, _path: &str) -> Option<String> {
    None
}

fn build_fixtures() -> (ModuleMap, PackageMap) {
    let mut mm = ModuleMap::new();
    mm.insert(ModuleEntry {
        repo: "cortex".into(),
        language: "rust",
        qualified_name: "crate::workers::run_worker".into(),
        artifact_path: "src/workers.rs".into(),
    });
    mm.insert(ModuleEntry {
        repo: "cortex".into(),
        language: "rust",
        qualified_name: "crate::util::Helper".into(),
        artifact_path: "src/util.rs".into(),
    });
    mm.insert(ModuleEntry {
        repo: "cortex".into(),
        language: "rust",
        qualified_name: "crate::util::unique_basename".into(),
        artifact_path: "src/util.rs".into(),
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
    (mm, pm)
}

fn run_with_local(source: &str, local_seed: &[(&str, &str)]) -> Vec<EdgeOp> {
    let (mm, pm) = build_fixtures();
    let mut ls = LocalSymbols::new("cortex", "rust", "src/lib.rs");
    for (bare, qualified) in local_seed {
        ls.insert(*bare, *qualified);
    }
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let edges = RustAnalyzer::new().extract(source, "cortex", "src/lib.rs");
    let ctx = PatchBuildContext {
        source_repo: "cortex",
        source_path: "src/lib.rs",
        source_content_hash: "sha256:abc",
        source_event_id: Some("evt-it"),
        resolver: &resolver,
        content_hash_for: &no_hash_lookup,
        analyzer_version: "phase11k.it",
    };
    build_graph_patch(&edges, &ctx).edges
}

fn first_edge_of(edges: &[EdgeOp], edge_type: &str) -> EdgeOp {
    edges
        .iter()
        .find(|e| e.edge_type == edge_type)
        .cloned()
        .unwrap_or_else(|| panic!("no edge of type {edge_type} found in {edges:?}"))
}

/// Case 1 — Tier 1: bare-name call hits the local-file symbol table
/// and resolves to its qualified Symbol natural key.
#[test]
fn tier1_local_bare_call_resolves_to_local_symbol() {
    let edges = run_with_local(
        "fn outer() { sibling(); }\n",
        &[("sibling", "crate::lib::sibling")],
    );
    let call = first_edge_of(&edges, "CALLS");
    assert_eq!(call.to_label, "Symbol");
    assert_eq!(call.to_key, "cortex|rust|crate::lib::sibling");
    assert_eq!(
        call.props.get("tier").and_then(|v| v.as_str()),
        Some("local_file")
    );
}

/// Case 2 — Tier 2: scoped use_decl walks the ModuleMap and yields
/// an `IMPORTS_FILE` edge from the source artifact to the artifact
/// owning the resolved symbol.
#[test]
fn tier2_module_path_use_resolves_to_workspace_artifact() {
    let edges = run_with_local("use crate::workers::run_worker;\n", &[]);
    let imp = first_edge_of(&edges, "IMPORTS_FILE");
    assert_eq!(imp.from_key, "cortex|src/lib.rs|sha256:abc");
    assert_eq!(imp.to_label, "Artifact");
    assert_eq!(imp.to_key, "cortex|src/workers.rs|*");
    assert_eq!(
        imp.props.get("tier").and_then(|v| v.as_str()),
        Some("intra_crate")
    );
}

/// Case 3 — Tier 2 basename fallback: a bare-name call whose name has
/// exactly one workspace-wide match resolves via the basename index.
#[test]
fn tier2_basename_fallback_resolves_unique_match() {
    let edges = run_with_local("fn outer() { unique_basename(); }\n", &[]);
    let call = first_edge_of(&edges, "CALLS");
    assert_eq!(call.to_label, "Symbol");
    assert_eq!(call.to_key, "cortex|rust|crate::util::unique_basename");
    assert_eq!(
        call.props.get("tier").and_then(|v| v.as_str()),
        Some("intra_crate")
    );
}

/// Case 4 — Tier 3: `use tokio::spawn` matches the package map and
/// emits an `IMPORTS_EXTERNAL` edge against an `:ExternalPackage`.
#[test]
fn tier3_external_package_promotes_imports_external() {
    let edges = run_with_local("use tokio::spawn;\n", &[]);
    let imp = first_edge_of(&edges, "IMPORTS_EXTERNAL");
    assert_eq!(imp.to_label, "ExternalPackage");
    assert_eq!(imp.to_key, "tokio");
    assert_eq!(
        imp.props.get("tier").and_then(|v| v.as_str()),
        Some("external")
    );
}

/// Case 5 — Tier 3 alias: `vectorizer-sdk` declared with a
/// `crate_name = "vectorizer"` override resolves either spelling.
#[test]
fn tier3_external_alias_resolves_via_crate_name() {
    let edges = run_with_local("use vectorizer::HnswSearch;\n", &[]);
    let imp = first_edge_of(&edges, "IMPORTS_EXTERNAL");
    assert_eq!(imp.to_label, "ExternalPackage");
    assert_eq!(imp.to_key, "vectorizer-sdk");
}

/// Case 6 — UNRESOLVED_IMPORT: nothing matches; the patch builder
/// drops the edge onto a sentinel `:UnresolvedImport` node keyed on
/// the joined-path hint.
#[test]
fn unresolved_import_falls_back_to_sentinel_node() {
    let edges = run_with_local("use no_such_crate::Frob;\n", &[]);
    let imp = first_edge_of(&edges, "UNRESOLVED_IMPORT");
    assert_eq!(imp.to_label, "UnresolvedImport");
    assert_eq!(imp.to_key, "no_such_crate::Frob");
    assert_eq!(
        imp.props.get("tier").and_then(|v| v.as_str()),
        Some("unresolved")
    );
}

/// Case 7 — Tier-1 wins over tier-2 when both could match.
#[test]
fn tier1_local_wins_over_tier2_basename_match() {
    let edges = run_with_local(
        "fn outer() { unique_basename(); }\n",
        &[("unique_basename", "crate::lib::unique_basename")],
    );
    let call = first_edge_of(&edges, "CALLS");
    assert_eq!(call.to_key, "cortex|rust|crate::lib::unique_basename");
    assert_eq!(
        call.props.get("tier").and_then(|v| v.as_str()),
        Some("local_file")
    );
}

/// Case 8 — `pub use crate::Symbol;` keeps the `RE_EXPORTS` edge
/// label even when tier-2 finds the symbol's owning artifact.
#[test]
fn pub_use_keeps_re_exports_label_through_resolution() {
    let edges = run_with_local("pub use crate::workers::run_worker;\n", &[]);
    let re = first_edge_of(&edges, "RE_EXPORTS");
    assert_eq!(re.to_label, "Artifact");
    assert_eq!(re.to_key, "cortex|src/workers.rs|*");
    assert_eq!(
        re.props.get("tier").and_then(|v| v.as_str()),
        Some("intra_crate")
    );
}
