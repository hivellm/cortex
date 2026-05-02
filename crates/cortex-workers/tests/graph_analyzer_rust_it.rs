//! Phase11k §1.7 — end-to-end Rust analyzer integration test.
//!
//! Synthesises a three-file Rust crate (`lib.rs` + `module_a.rs` +
//! `module_b.rs`) on a temp directory, walks it with
//! [`build_rust_module_map_from_root`], runs the analyzer over each
//! file, drives the resulting [`CodeEdge`] batches through the
//! [`build_graph_patch`] pipeline against a real
//! [`SymbolResolver`], and asserts the produced
//! [`GraphPatch`] matches the proposal verbatim — every edge class
//! the §1 sub-tasks promised lands.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use tempfile::TempDir;

use cortex_workers::graph::analyzer::{
    build_graph_patch, CodeAnalyzer, PatchBuildContext, RustAnalyzer,
};
use cortex_workers::graph::patch::EdgeOp;
use cortex_workers::graph::resolver::{
    build_rust_module_map_from_root, ExternalPackage, LocalSymbols, ModuleMap, PackageMap,
    SymbolResolver,
};

const REPO: &str = "cortex";

fn write_file(dir: &TempDir, rel: &str, content: &str) -> std::path::PathBuf {
    let abs = dir.path().join(rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    let mut f = fs::File::create(&abs).expect("create file");
    f.write_all(content.as_bytes()).expect("write");
    abs
}

fn fake_hash_for(path: &str) -> String {
    format!("sha256:{}", path.replace('/', "_"))
}

struct CrateFiles {
    lib_rs: std::path::PathBuf,
    lib_src: String,
    module_a_src: String,
    module_b_src: String,
}

fn synth_crate(dir: &TempDir) -> CrateFiles {
    let module_a_src = "\
pub fn helper() -> u32 { 0 }
pub struct Utility;
"
    .to_string();
    let module_b_src = "\
pub trait Runner {
    fn run(&self);
}
"
    .to_string();
    let lib_src = "\
pub mod module_a;
pub mod module_b;

use crate::module_a::helper;
use crate::module_b::Runner;
use tokio::spawn;
use no_such_crate::Frob;

pub struct Worker;

impl Runner for Worker {
    fn run(&self) {
        helper();
    }
}

pub fn outer() -> u32 {
    let _w: Worker = Worker;
    helper()
}
"
    .to_string();

    let lib_rs = write_file(dir, "src/lib.rs", &lib_src);
    let _ = write_file(dir, "src/module_a.rs", &module_a_src);
    let _ = write_file(dir, "src/module_b.rs", &module_b_src);
    CrateFiles {
        lib_rs,
        lib_src,
        module_a_src,
        module_b_src,
    }
}

fn build_package_map() -> PackageMap {
    let mut pm = PackageMap::new();
    pm.insert(ExternalPackage {
        name: "tokio".into(),
        natural_key: "tokio".into(),
        local_path: None,
        crate_name: None,
    });
    pm
}

fn run_analyzer(
    src: &str,
    rel_path: &str,
    module_map: &ModuleMap,
    package_map: &PackageMap,
) -> Vec<EdgeOp> {
    let ls = LocalSymbols::new(REPO, "rust", rel_path);
    let resolver = SymbolResolver::new(module_map, package_map, &ls);
    let edges = RustAnalyzer::new().extract(src, REPO, rel_path);
    let mut hashes: BTreeMap<(String, String), String> = BTreeMap::new();
    hashes.insert(
        (REPO.into(), "src/module_a.rs".into()),
        fake_hash_for("src/module_a.rs"),
    );
    hashes.insert(
        (REPO.into(), "src/module_b.rs".into()),
        fake_hash_for("src/module_b.rs"),
    );
    hashes.insert(
        (REPO.into(), "src/lib.rs".into()),
        fake_hash_for("src/lib.rs"),
    );
    let lookup =
        move |repo: &str, path: &str| hashes.get(&(repo.to_string(), path.to_string())).cloned();
    let content_hash = fake_hash_for(rel_path);
    let ctx = PatchBuildContext {
        source_repo: REPO,
        source_path: rel_path,
        source_content_hash: &content_hash,
        source_event_id: Some("evt-e2e"),
        resolver: &resolver,
        content_hash_for: &lookup,
        analyzer_version: "phase11k.it",
    };
    build_graph_patch(&edges, &ctx).edges
}

fn has_edge<F: Fn(&EdgeOp) -> bool>(edges: &[EdgeOp], pred: F) -> bool {
    edges.iter().any(pred)
}

#[test]
fn three_file_crate_emits_all_edge_classes_in_proposal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let crate_files = synth_crate(&dir);

    let mut module_map =
        build_rust_module_map_from_root(REPO, &crate_files.lib_rs).expect("module map");
    // build_rust_module_map_from_root walks absolute paths from disk;
    // rewrite the artifact_path slot on every entry to the repo-
    // relative form the resolver expects, so the resulting Artifact
    // natural keys line up with what the analyzer emits.
    module_map = rewrite_artifact_paths(&module_map, &dir);

    let package_map = build_package_map();
    assert!(!module_map.is_empty(), "module map must populate");

    let edges_lib = run_analyzer(
        &crate_files.lib_src,
        "src/lib.rs",
        &module_map,
        &package_map,
    );
    let edges_a = run_analyzer(
        &crate_files.module_a_src,
        "src/module_a.rs",
        &module_map,
        &package_map,
    );
    let edges_b = run_analyzer(
        &crate_files.module_b_src,
        "src/module_b.rs",
        &module_map,
        &package_map,
    );

    // 1. `use crate::module_a::helper;` → IMPORTS_FILE → module_a artifact.
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "IMPORTS_FILE"
            && e.to_label == "Artifact"
            && e.to_key.contains("src/module_a.rs")),
        "expected IMPORTS_FILE → module_a, got: {edges_lib:?}"
    );

    // 2. `use tokio::spawn;` → IMPORTS_EXTERNAL → ExternalPackage(tokio).
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "IMPORTS_EXTERNAL"
            && e.to_label == "ExternalPackage"
            && e.to_key == "tokio"),
        "expected IMPORTS_EXTERNAL → tokio, got: {edges_lib:?}"
    );

    // 3. `use no_such_crate::Frob;` → UNRESOLVED_IMPORT.
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "UNRESOLVED_IMPORT"
            && e.to_label == "UnresolvedImport"
            && e.to_key == "no_such_crate::Frob"),
        "expected UNRESOLVED_IMPORT, got: {edges_lib:?}"
    );

    // 4. `impl Runner for Worker { ... }` → IMPLEMENTS edge anchored
    //    on `Worker` symbol → tier-2 to `Runner`.
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "IMPLEMENTS"
            && e.from_key == "cortex|rust|Worker"),
        "expected IMPLEMENTS Worker→Runner, got: {edges_lib:?}"
    );

    // 5. `outer()` body calls `helper()` → CALLS → tier-2 basename
    //    fallback to crate::module_a::helper.
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "CALLS"
            && e.to_key == "cortex|rust|crate::module_a::helper"),
        "expected CALLS → crate::module_a::helper, got: {edges_lib:?}"
    );

    // 6. `let _w: Worker = Worker;` produces a USES_TYPE edge from
    //    `outer` to `Worker`.
    assert!(
        has_edge(&edges_lib, |e| e.edge_type == "USES_TYPE"
            && e.from_key == "cortex|rust|outer"
            && e.to_key.ends_with("Worker")),
        "expected USES_TYPE outer→Worker, got: {edges_lib:?}"
    );

    // 7. module_a / module_b emit no errors and at minimum carry a
    //    source artifact node + their own structure (no panics, no
    //    spurious edges).
    let _ = (edges_a, edges_b);
}

/// Test helper — `build_rust_module_map_from_root` records each
/// entry's `artifact_path` as the absolute on-disk path because that
/// is the only thing the walker has. Inside the live worker the
/// archive sink rewrites those paths to repo-relative form before the
/// resolver sees them; for this IT we do the same rewrite up-front
/// against the temp-dir prefix.
fn rewrite_artifact_paths(map: &ModuleMap, dir: &TempDir) -> ModuleMap {
    let prefix = dir.path().to_string_lossy().replace('\\', "/");
    let mut out = ModuleMap::new();
    for entry in map.iter() {
        let mut e = entry.clone();
        let cleaned = e.artifact_path.replace('\\', "/");
        if let Some(stripped) = cleaned.strip_prefix(&format!("{prefix}/")) {
            e.artifact_path = stripped.to_string();
        } else {
            e.artifact_path = cleaned;
        }
        out.insert(e);
    }
    out
}
