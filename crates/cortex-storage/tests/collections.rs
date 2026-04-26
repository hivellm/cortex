//! Integration tests for `cortex_storage::collections`.

use cortex_storage::collections::EMBED_DIM;
use cortex_storage::{CollectionTier, COLLECTIONS};

#[test]
fn collections_cover_every_name() {
    for sc in COLLECTIONS {
        assert!(sc.name.starts_with("cortex."));
    }
}

#[test]
fn tiers_are_consistent_with_encoding() {
    for sc in COLLECTIONS {
        match sc.tier {
            CollectionTier::Hot => assert_eq!(sc.encoding, "fp32"),
            CollectionTier::Warm => assert_eq!(sc.encoding, "pq"),
            CollectionTier::Cold => assert_eq!(sc.encoding, "binary"),
        }
    }
}

#[test]
fn all_collections_use_same_dim() {
    for sc in COLLECTIONS {
        assert_eq!(sc.dim, EMBED_DIM);
    }
}

#[test]
fn json_render() {
    let json = serde_json::to_string(&COLLECTIONS[0]).unwrap();
    assert!(json.contains("cortex.turn.fp32"));
    assert!(json.contains("\"fp32\""));
}
