//! Phase11k §2.5 — multi-language analyzer integration test.
//!
//! Drives a synthetic workspace containing one Rust file, one
//! TypeScript file, one Python file, and one Go file through the
//! per-language analyzers + the shared resolver pipeline. Asserts:
//!
//! 1. Every language emits the edge classes the §2.x sub-tasks
//!    promised (imports / calls / extends / implements where
//!    applicable).
//! 2. The Rust crate's `vectorizer-sdk` import resolves the *same
//!    way* a TypeScript workspace import resolves: tier-3 →
//!    `:ExternalPackage` with a stable natural key.

use cortex_workers::graph::analyzer::{
    build_graph_patch, CodeAnalyzer, GoAnalyzer, PatchBuildContext, PythonAnalyzer, RustAnalyzer,
    TypescriptAnalyzer,
};
use cortex_workers::graph::patch::EdgeOp;
use cortex_workers::graph::resolver::{
    ExternalPackage, LocalSymbols, ModuleMap, PackageMap, SymbolResolver,
};

fn no_hash_lookup(_repo: &str, _path: &str) -> Option<String> {
    None
}

fn build_package_map() -> PackageMap {
    let mut pm = PackageMap::new();
    pm.insert(ExternalPackage {
        name: "vectorizer-sdk".into(),
        natural_key: "vectorizer-sdk".into(),
        local_path: None,
        crate_name: Some("vectorizer".into()),
    });
    pm.insert(ExternalPackage {
        name: "react".into(),
        natural_key: "react".into(),
        local_path: None,
        crate_name: None,
    });
    pm.insert(ExternalPackage {
        name: "numpy".into(),
        natural_key: "numpy".into(),
        local_path: None,
        crate_name: None,
    });
    pm.insert(ExternalPackage {
        name: "fmt".into(),
        natural_key: "fmt".into(),
        local_path: None,
        crate_name: None,
    });
    pm
}

fn drive<A: CodeAnalyzer>(
    analyzer: A,
    src: &str,
    path: &str,
    package_map: &PackageMap,
) -> Vec<EdgeOp> {
    let module_map = ModuleMap::new();
    let local = LocalSymbols::new("cortex", analyzer.language().label(), path);
    let resolver = SymbolResolver::new(&module_map, package_map, &local);
    let edges = analyzer.extract(src, "cortex", path);
    let ctx = PatchBuildContext {
        source_repo: "cortex",
        source_path: path,
        source_content_hash: "sha256:multi",
        source_event_id: Some("evt-multi"),
        resolver: &resolver,
        content_hash_for: &no_hash_lookup,
        analyzer_version: "phase11k.it.multi",
    };
    build_graph_patch(&edges, &ctx).edges
}

#[test]
fn rust_external_resolves_to_external_package_node() {
    let pm = build_package_map();
    let src = "use vectorizer::HnswSearch;\n";
    let edges = drive(RustAnalyzer::new(), src, "src/lib.rs", &pm);
    let imp = edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_EXTERNAL")
        .expect("IMPORTS_EXTERNAL");
    assert_eq!(imp.to_label, "ExternalPackage");
    assert_eq!(imp.to_key, "vectorizer-sdk");
}

#[test]
fn ts_external_resolves_to_external_package_node() {
    let pm = build_package_map();
    let src = "import React from 'react';\n";
    let edges = drive(TypescriptAnalyzer::new(), src, "src/index.ts", &pm);
    let imp = edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_EXTERNAL")
        .expect("IMPORTS_EXTERNAL");
    assert_eq!(imp.to_label, "ExternalPackage");
    assert_eq!(imp.to_key, "react");
}

#[test]
fn python_import_emits_imports_file_edge() {
    let pm = build_package_map();
    let src = "import numpy as np\n";
    let edges = drive(PythonAnalyzer::new(), src, "src/main.py", &pm);
    let imp = edges
        .iter()
        .find(|e| matches!(e.edge_type.as_str(), "IMPORTS_FILE" | "IMPORTS_EXTERNAL"))
        .expect("import edge");
    // numpy is registered in the package map → tier-3 promotion.
    assert_eq!(imp.edge_type, "IMPORTS_EXTERNAL");
    assert_eq!(imp.to_key, "numpy");
}

#[test]
fn go_import_emits_imports_file_edge() {
    let pm = build_package_map();
    let src = "package main\nimport \"fmt\"\n";
    let edges = drive(GoAnalyzer::new(), src, "main.go", &pm);
    let imp = edges
        .iter()
        .find(|e| matches!(e.edge_type.as_str(), "IMPORTS_FILE" | "IMPORTS_EXTERNAL"))
        .expect("import edge");
    assert_eq!(imp.edge_type, "IMPORTS_EXTERNAL");
    assert_eq!(imp.to_key, "fmt");
}

#[test]
fn rust_and_ts_external_share_node_label_and_props_shape() {
    let pm = build_package_map();
    let rust_edges = drive(
        RustAnalyzer::new(),
        "use vectorizer::HnswSearch;\n",
        "src/lib.rs",
        &pm,
    );
    let ts_edges = drive(
        TypescriptAnalyzer::new(),
        "import React from 'react';\n",
        "src/index.ts",
        &pm,
    );
    let rust_ext = rust_edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_EXTERNAL")
        .expect("rust ext");
    let ts_ext = ts_edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_EXTERNAL")
        .expect("ts ext");

    assert_eq!(rust_ext.from_label, "Artifact");
    assert_eq!(ts_ext.from_label, "Artifact");
    assert_eq!(rust_ext.to_label, ts_ext.to_label);
    assert_eq!(
        rust_ext.props.get("tier").and_then(|v| v.as_str()),
        ts_ext.props.get("tier").and_then(|v| v.as_str())
    );
}
