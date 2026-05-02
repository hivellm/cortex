//! Phase11k §5.5 — bootstrap + live trigger end-to-end IT.
//!
//! Synthesises a fixture workspace, runs the static analyzer +
//! patch builder twice (bootstrap pass + simulated live edit),
//! drives the resulting patches through the
//! [`PatchCoalescer::observe_static_emission`] dedup, and asserts:
//!
//! 1. The bootstrap pass emits a non-empty patch.
//! 2. A repeat run against the same content_hash dedupes via the
//!    coalescer's per-session table (no re-emission).
//! 3. A simulated live edit (different content_hash) bypasses the
//!    dedup and re-emits.
//! 4. The §5.4 dedup keys distinct paths separately so two edits
//!    to different files do not false-collapse.
//!
//! Pure unit-level — no Nexus / Synap dependencies. The §5.2 live
//! trigger and §5.4 coalescer are exercised directly via the
//! public surface.

use std::collections::BTreeMap;

use cortex_workers::graph::analyzer::{
    build_graph_patch, CodeAnalyzer, PatchBuildContext, RustAnalyzer,
};
use cortex_workers::graph::coalescer::PatchCoalescer;
use cortex_workers::graph::resolver::{
    ExternalPackage, LocalSymbols, ModuleEntry, ModuleMap, PackageMap, SymbolResolver,
};

const REPO: &str = "cortex";
const ANALYZER_V: &str = "phase11l.1";

fn fixtures() -> (ModuleMap, PackageMap) {
    let mut mm = ModuleMap::new();
    mm.insert(ModuleEntry {
        repo: REPO.into(),
        language: "rust",
        qualified_name: "crate::module_a::helper".into(),
        artifact_path: "src/module_a.rs".into(),
    });
    let mut pm = PackageMap::new();
    pm.insert(ExternalPackage {
        name: "tokio".into(),
        natural_key: "tokio".into(),
        local_path: None,
        crate_name: None,
    });
    (mm, pm)
}

fn run_analyzer(
    src: &str,
    rel_path: &str,
    content_hash: &str,
) -> cortex_workers::graph::patch::GraphPatch {
    let (mm, pm) = fixtures();
    let ls = LocalSymbols::new(REPO, "rust", rel_path);
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let edges = RustAnalyzer::new().extract(src, REPO, rel_path);
    let hashes: BTreeMap<(String, String), String> = BTreeMap::new();
    let lookup = move |repo: &str, path: &str| -> Option<String> {
        hashes.get(&(repo.to_string(), path.to_string())).cloned()
    };
    let ctx = PatchBuildContext {
        source_repo: REPO,
        source_path: rel_path,
        source_content_hash: content_hash,
        source_event_id: Some("evt-it"),
        resolver: &resolver,
        content_hash_for: &lookup,
        analyzer_version: ANALYZER_V,
    };
    build_graph_patch(&edges, &ctx)
}

#[test]
fn bootstrap_pass_emits_non_empty_patch() {
    let p = run_analyzer(
        "use crate::module_a::helper;\n",
        "src/lib.rs",
        "sha256:boot",
    );
    assert!(
        !p.is_empty(),
        "bootstrap pass must emit at least the source artifact + import edge"
    );
    let imp_count = p
        .edges
        .iter()
        .filter(|e| e.edge_type == "IMPORTS_FILE")
        .count();
    assert_eq!(
        imp_count, 1,
        "bootstrap pass must emit one IMPORTS_FILE edge"
    );
}

#[test]
fn repeat_run_against_same_content_hash_dedupes_via_coalescer() {
    let mut coalescer = PatchCoalescer::new();
    let path = "src/lib.rs";
    let hash = "sha256:boot";
    // First emission — coalescer must accept.
    assert!(
        coalescer.observe_static_emission(REPO, path, hash, ANALYZER_V),
        "first emission must NOT dedup"
    );
    // Build the patch (would be re-emitted in a search-and-replace burst).
    let _ = run_analyzer("use crate::module_a::helper;\n", path, hash);
    // Second emission with the same tuple — coalescer must dedup.
    assert!(
        !coalescer.observe_static_emission(REPO, path, hash, ANALYZER_V),
        "repeat emission must dedup"
    );
}

#[test]
fn live_edit_with_new_content_hash_bypasses_dedup() {
    let mut coalescer = PatchCoalescer::new();
    let path = "src/lib.rs";
    coalescer.observe_static_emission(REPO, path, "sha256:boot", ANALYZER_V);
    // Simulated live edit — a Write tool_call landed a new
    // content_hash. The §5.4 coalescer must NOT dedup, so the §5.2
    // live trigger re-runs the analyzer + ships a fresh patch.
    assert!(
        coalescer.observe_static_emission(REPO, path, "sha256:live", ANALYZER_V),
        "new content_hash must re-emit"
    );
    // Run the analyzer at the new hash to confirm it produces a
    // valid patch (proves the §5.2 live trigger contract).
    let p = run_analyzer("use crate::module_a::helper;\n", path, "sha256:live");
    assert!(!p.is_empty(), "live edit must produce a non-empty patch");
}

#[test]
fn distinct_paths_do_not_collapse_through_coalescer() {
    let mut coalescer = PatchCoalescer::new();
    coalescer.observe_static_emission(REPO, "src/a.rs", "sha256:abc", ANALYZER_V);
    // Different path — even with the same hash + version (rare but
    // possible in deduplicated archives), the tuple key keeps them
    // apart so each landing site gets its own emission.
    assert!(coalescer.observe_static_emission(REPO, "src/b.rs", "sha256:abc", ANALYZER_V));
    assert_eq!(coalescer.static_emission_count(), 2);
}

#[test]
fn version_bump_forces_re_emission_across_full_workspace() {
    let mut coalescer = PatchCoalescer::new();
    let paths = ["src/a.rs", "src/b.rs", "src/c.rs"];
    for p in &paths {
        coalescer.observe_static_emission(REPO, p, "sha256:boot", "phase11k.1");
    }
    // Bump the version stamp. Every prior emission MUST re-emit so
    // the bumped extraction logic actually lands.
    for p in &paths {
        assert!(
            coalescer.observe_static_emission(REPO, p, "sha256:boot", "phase11l.1"),
            "version bump must force re-emission for {p}"
        );
    }
    assert_eq!(
        coalescer.static_emission_count(),
        paths.len() * 2,
        "every path emits once per version"
    );
}
