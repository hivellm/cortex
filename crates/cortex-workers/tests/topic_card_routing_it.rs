//! Phase11r §3.6 — `Kind::TopicCard` end-to-end routing IT.
//!
//! A single TopicCard envelope must land in:
//!
//! 1. Family `"topic_cards"` from the fulltext router.
//! 2. Per-repo Vectorizer collection `cortex-{slug}-topic_cards`
//!    via the embedder router (matches the fulltext family for the
//!    `cortex-{slug}-{family}` naming convention).
//! 3. Per-repo Meili index `cortex-{slug}-topic_cards` via
//!    `index_for` / `index_for_event`.
//! 4. Global Meili index name `cortex_topic_cards` available in
//!    `cortex_storage::names::INDEX_TOPIC_CARDS` (the dashboard's
//!    cross-repo lane + the renderer's keyword fan-out read from
//!    this one).
//! 5. The matching Vectorizer collection name constants
//!    (`cortex.topic_card.fp32` + `cortex.topic_card.pq`) exposed
//!    by `cortex_storage::names`.
//!
//! Drift across any of those surfaces silently breaks the
//! topic-card synthesiser → indexer hand-off so the contract is
//! pinned here. Mirrors the phase11j §3.9 consolidation IT.

use cortex_core::events::Kind;
use cortex_storage::names::{
    COLLECTION_TOPIC_CARD_FP32, COLLECTION_TOPIC_CARD_PQ, INDEX_TOPIC_CARDS,
};
use cortex_workers::embedder::routing::collection_for as embedder_collection_for;
use cortex_workers::fulltext::{family_for, family_for_event, index_for, FAMILIES};

#[test]
fn fulltext_router_maps_topic_card_to_topic_cards_family() {
    assert_eq!(family_for(Kind::TopicCard), "topic_cards");
    assert_eq!(family_for_event(Kind::TopicCard, &[], None), "topic_cards");
    assert!(
        FAMILIES.contains(&"topic_cards"),
        "FAMILIES must enumerate `topic_cards` so the bootstrap settings push covers it"
    );
}

#[test]
fn embedder_router_maps_topic_card_to_topic_cards_collection() {
    // Embedder collection naming follows the fulltext family
    // (`cortex-{slug}-{family}`); divergence means a topic card is
    // fulltext-indexed but never embedded. Pin the embedder
    // convention so a future prefix bump cannot accidentally
    // route the kind to `misc`.
    assert_eq!(
        embedder_collection_for(&Kind::TopicCard, "cortex", Some("Cortex")),
        "cortex-cortex-topic_cards"
    );
    assert_eq!(
        embedder_collection_for(&Kind::TopicCard, "cortex", None),
        "cortex-unknown-topic_cards"
    );
}

#[test]
fn index_for_topic_card_lands_in_per_repo_topic_cards_index() {
    let repo = Some("Cortex");
    assert_eq!(
        index_for("cortex-", Kind::TopicCard, repo),
        "cortex-cortex-topic_cards"
    );
    assert_eq!(
        index_for("cortex-", Kind::TopicCard, Some("Tml")),
        "cortex-tml-topic_cards"
    );
    assert_eq!(
        index_for("cortex-", Kind::TopicCard, None),
        "cortex-unknown-topic_cards"
    );
}

#[test]
fn global_topic_card_storage_constants_match_naming_convention() {
    // The renderer's topic-card lane fans out to the global
    // `cortex_topic_cards` index for cross-repo reads; the embedder
    // needs the two tier-specific collection names. Pin both
    // shapes against the family the routers emit.
    assert_eq!(INDEX_TOPIC_CARDS, "cortex_topic_cards");
    assert_eq!(COLLECTION_TOPIC_CARD_FP32, "cortex.topic_card.fp32");
    assert_eq!(COLLECTION_TOPIC_CARD_PQ, "cortex.topic_card.pq");
}
