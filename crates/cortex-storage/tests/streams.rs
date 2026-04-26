//! Integration tests for `cortex_storage::streams`.

use cortex_storage::streams::{KV_NAMESPACES, STREAMS};
use cortex_storage::ALL_STREAMS;
use std::collections::HashSet;

#[test]
fn streams_cover_every_name() {
    let declared: HashSet<_> = STREAMS.iter().map(|s| s.name).collect();
    for expected in ALL_STREAMS {
        assert!(declared.contains(expected), "missing stream config for `{expected}`");
    }
}

#[test]
fn partitions_are_positive() {
    for s in STREAMS {
        assert!(s.partitions >= 1);
    }
}

#[test]
fn kv_namespaces_have_ttl() {
    for ns in KV_NAMESPACES {
        assert!(ns.ttl_seconds > 0);
    }
}
