//! Phase11j §3.9 — `Kind::Consolidation` end-to-end routing IT.
//!
//! A single Consolidation envelope must land in:
//!
//! 1. Family `"consolidations"` from the fulltext router.
//! 2. Per-repo Vectorizer collection `cortex-{slug}-consolidations`
//!    via the embedder router (matches the fulltext family for the
//!    `cortex-{slug}-{family}` naming convention).
//! 3. Per-repo Meili index `cortex-{slug}-consolidations` via
//!    `index_for` / `index_for_event`.
//! 4. Global Meili index name `cortex_consolidations` available in
//!    `cortex_storage::names::INDEX_CONSOLIDATIONS` (the dashboard's
//!    cross-repo lane reads from this one).
//! 5. The matching Vectorizer collection name constants exposed by
//!    `cortex_storage::names`.
//!
//! Drift across any of those surfaces silently breaks the
//! consolidator → indexer hand-off so the contract is pinned here.

use cortex_core::events::Kind;
use cortex_storage::names::{
    COLLECTION_CONSOLIDATION_FP32, COLLECTION_CONSOLIDATION_PQ, INDEX_CONSOLIDATIONS,
};
use cortex_workers::embedder::routing::collection_for as embedder_collection_for;
use cortex_workers::fulltext::{family_for, family_for_event, index_for, FAMILIES};

#[test]
fn fulltext_router_maps_consolidation_to_consolidations_family() {
    assert_eq!(family_for(Kind::Consolidation), "consolidations");
    assert_eq!(
        family_for_event(Kind::Consolidation, &[], None),
        "consolidations"
    );
    assert!(
        FAMILIES.contains(&"consolidations"),
        "FAMILIES must enumerate `consolidations` so the bootstrap settings push covers it"
    );
}

#[test]
fn embedder_router_maps_consolidation_to_consolidations_collection() {
    // Embedder collection naming follows the fulltext family
    // (`cortex-{slug}-{family}`); a divergence here means a
    // consolidation is fulltext-indexed but never embedded.
    // Embedder prefix is `"cortex"` (no trailing dash); the indexer
    // surface uses `"cortex-"`. Both reach the same `consolidations`
    // family suffix — pin the embedder convention here so a future
    // prefix bump does not accidentally route the kind to `misc`.
    assert_eq!(
        embedder_collection_for(&Kind::Consolidation, "cortex", Some("Cortex")),
        "cortex-cortex-consolidations"
    );
    assert_eq!(
        embedder_collection_for(&Kind::Consolidation, "cortex", None),
        "cortex-unknown-consolidations"
    );
}

#[test]
fn index_for_consolidation_lands_in_per_repo_consolidations_index() {
    let repo = Some("Cortex");
    assert_eq!(
        index_for("cortex-", Kind::Consolidation, repo),
        "cortex-cortex-consolidations"
    );
    assert_eq!(
        index_for("cortex-", Kind::Consolidation, Some("Tml")),
        "cortex-tml-consolidations"
    );
    assert_eq!(
        index_for("cortex-", Kind::Consolidation, None),
        "cortex-unknown-consolidations"
    );
}

#[test]
fn global_consolidation_storage_constants_match_naming_convention() {
    // The dashboard's cross-repo lane reads from the global
    // `cortex_consolidations` index; the embedder needs the two
    // tier-specific collection names. Pin both shapes against the
    // family the routers emit.
    assert_eq!(INDEX_CONSOLIDATIONS, "cortex_consolidations");
    assert_eq!(COLLECTION_CONSOLIDATION_FP32, "cortex.consolidation.fp32");
    assert_eq!(COLLECTION_CONSOLIDATION_PQ, "cortex.consolidation.pq");
}
