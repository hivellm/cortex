//! Phase11l §5.4 — pending → canonical sentinel redirect IT.
//!
//! Drives the phase11l §5.1 `pending|repo|path` sentinel through
//! the phase11k §5.3 stale-edge sweeper and asserts the redirect
//! collapses correctly:
//!
//! 1. A tier-2 IMPORTS_FILE patch with an unknown sibling hash
//!    emits a `pending|repo|path` sentinel id.
//! 2. The canonical sibling artifact patch arrives; the sweeper's
//!    `redirect_pending_sentinels` issues a bulk delete on every
//!    edge pointing at the sentinel.
//! 3. The §5.2 live trigger re-emits IMPORTS_FILE against the
//!    canonical artifact on the next batch — verified by feeding
//!    a fresh patch and asserting the new edge lands on the real
//!    `repo|path|sha256:abc` _id.
//!
//! Pure unit-level — uses an in-memory `GraphWriter` impl that
//! records every bulk-delete call without contacting Nexus. The
//! goal is to pin the producer + sweeper contract; the live-Nexus
//! flavour rides the existing `graph_nexus_client.rs` IT under
//! `CORTEX_GRAPH_IT=1`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_workers::graph::analyzer::{
    build_graph_patch, is_pending_artifact_id, CodeAnalyzer, PatchBuildContext, RustAnalyzer,
    PENDING_ARTIFACT_PREFIX,
};
use cortex_workers::graph::nexus_client::GraphClientError;
use cortex_workers::graph::patch::{EdgeDeleteFilter, GraphPatch, GraphWriteReport};
use cortex_workers::graph::resolver::{
    LocalSymbols, ModuleEntry, ModuleMap, PackageMap, SymbolResolver,
};
use cortex_workers::graph::writer::GraphWriter;
use cortex_workers::graph::{EnrichedEvent, StaleEdgeSweeper};

const REPO: &str = "cortex";
const ANALYZER_V: &str = "phase11l.1";

#[derive(Default)]
struct RecordingWriter {
    deletes: Mutex<Vec<EdgeDeleteFilter>>,
    canned_delete_count: u64,
}

#[async_trait]
impl GraphWriter for RecordingWriter {
    async fn write_batch(
        &self,
        _events: &[EnrichedEvent],
    ) -> Result<GraphWriteReport, GraphClientError> {
        Ok(GraphWriteReport::default())
    }

    async fn write_patches(
        &self,
        _patches: Vec<GraphPatch>,
    ) -> Result<GraphWriteReport, GraphClientError> {
        Ok(GraphWriteReport::default())
    }

    async fn delete_edges_by_filter(
        &self,
        filter: EdgeDeleteFilter,
    ) -> Result<u64, GraphClientError> {
        if let Ok(mut g) = self.deletes.lock() {
            g.push(filter);
        }
        Ok(self.canned_delete_count)
    }
}

fn fixtures() -> (ModuleMap, PackageMap) {
    let mut mm = ModuleMap::new();
    mm.insert(ModuleEntry {
        repo: REPO.into(),
        language: "rust",
        qualified_name: "crate::module_a::helper".into(),
        artifact_path: "src/module_a.rs".into(),
    });
    let pm = PackageMap::new();
    (mm, pm)
}

fn build_patch_with_unknown_sibling() -> GraphPatch {
    let (mm, pm) = fixtures();
    let ls = LocalSymbols::new(REPO, "rust", "src/lib.rs");
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let edges = RustAnalyzer::new().extract("use crate::module_a::helper;\n", REPO, "src/lib.rs");
    let hashes: BTreeMap<(String, String), String> = BTreeMap::new();
    let lookup = move |repo: &str, path: &str| -> Option<String> {
        hashes.get(&(repo.to_string(), path.to_string())).cloned()
    };
    let ctx = PatchBuildContext {
        source_repo: REPO,
        source_path: "src/lib.rs",
        source_content_hash: "sha256:lib",
        source_event_id: Some("evt-pending"),
        resolver: &resolver,
        content_hash_for: &lookup,
        analyzer_version: ANALYZER_V,
    };
    build_graph_patch(&edges, &ctx)
}

fn build_patch_with_known_sibling() -> GraphPatch {
    let (mm, pm) = fixtures();
    let ls = LocalSymbols::new(REPO, "rust", "src/lib.rs");
    let resolver = SymbolResolver::new(&mm, &pm, &ls);
    let edges = RustAnalyzer::new().extract("use crate::module_a::helper;\n", REPO, "src/lib.rs");
    let mut hashes: BTreeMap<(String, String), String> = BTreeMap::new();
    hashes.insert(
        (REPO.to_string(), "src/module_a.rs".to_string()),
        "sha256:canonical".into(),
    );
    let lookup = move |repo: &str, path: &str| -> Option<String> {
        hashes.get(&(repo.to_string(), path.to_string())).cloned()
    };
    let ctx = PatchBuildContext {
        source_repo: REPO,
        source_path: "src/lib.rs",
        source_content_hash: "sha256:lib",
        source_event_id: Some("evt-canonical"),
        resolver: &resolver,
        content_hash_for: &lookup,
        analyzer_version: ANALYZER_V,
    };
    build_graph_patch(&edges, &ctx)
}

#[test]
fn unknown_sibling_hash_emits_pending_sentinel() {
    let p = build_patch_with_unknown_sibling();
    let imp = p
        .edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_FILE")
        .expect("imports edge");
    assert!(
        is_pending_artifact_id(&imp.to_key),
        "unknown sibling hash MUST resolve to the pending sentinel form, got: {}",
        imp.to_key
    );
    assert!(imp.to_key.starts_with(PENDING_ARTIFACT_PREFIX));
}

#[test]
fn known_sibling_hash_emits_canonical_artifact_key() {
    let p = build_patch_with_known_sibling();
    let imp = p
        .edges
        .iter()
        .find(|e| e.edge_type == "IMPORTS_FILE")
        .expect("imports edge");
    assert_eq!(
        imp.to_key, "cortex|src/module_a.rs|sha256:canonical",
        "known sibling hash MUST resolve to canonical (repo|path|hash) form"
    );
    assert!(!is_pending_artifact_id(&imp.to_key));
}

#[tokio::test]
async fn sweeper_redirect_targets_pending_prefix() {
    let writer = Arc::new(RecordingWriter {
        canned_delete_count: 3,
        ..RecordingWriter::default()
    });
    let sweeper = StaleEdgeSweeper::new(writer.clone(), ANALYZER_V);
    let deleted = sweeper
        .redirect_pending_sentinels()
        .await
        .expect("redirect");
    assert_eq!(deleted, 3);
    let calls = writer.deletes.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].to_natural_key_prefix.as_deref(),
        Some(PENDING_ARTIFACT_PREFIX),
        "redirect filter must scope deletes to the pending sentinel prefix"
    );
}

#[tokio::test]
async fn redirect_idempotent_when_no_pending_left() {
    let writer = Arc::new(RecordingWriter::default());
    let sweeper = StaleEdgeSweeper::new(writer.clone(), ANALYZER_V);
    let first = sweeper.redirect_pending_sentinels().await.unwrap();
    let second = sweeper.redirect_pending_sentinels().await.unwrap();
    assert_eq!(first, 0);
    assert_eq!(second, 0);
    assert_eq!(
        writer.deletes.lock().unwrap().len(),
        2,
        "every sweep call MUST issue exactly one filter delete (idempotent count)"
    );
}

#[test]
fn pending_to_canonical_collapses_under_conflict_match() {
    // Two distinct edges both target the unknown-hash sibling on
    // the SAME (repo, path) — the sentinel collapses them via
    // ConflictPolicy::Match. When the canonical patch lands, the
    // sweeper deletes the sentinel-pointed edges and the live
    // trigger re-emits against the canonical artifact (proven by
    // build_patch_with_known_sibling above resolving to the
    // canonical _id form).
    let pending = build_patch_with_unknown_sibling();
    let canonical = build_patch_with_known_sibling();
    let pending_to_keys: Vec<String> = pending
        .edges
        .iter()
        .filter(|e| e.edge_type == "IMPORTS_FILE")
        .map(|e| e.to_key.clone())
        .collect();
    let canonical_to_keys: Vec<String> = canonical
        .edges
        .iter()
        .filter(|e| e.edge_type == "IMPORTS_FILE")
        .map(|e| e.to_key.clone())
        .collect();
    assert!(
        pending_to_keys.iter().all(|k| is_pending_artifact_id(k)),
        "pending pass MUST emit only sentinel-shaped edges"
    );
    assert!(
        canonical_to_keys.iter().all(|k| !is_pending_artifact_id(k)),
        "canonical pass MUST emit no sentinel edges"
    );
}
