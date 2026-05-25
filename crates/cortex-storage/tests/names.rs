//! Integration tests for `cortex_storage::names`.

use cortex_storage::{ALL_INDEXES, ALL_STREAMS};

#[test]
fn streams_are_unique() {
    let mut v = ALL_STREAMS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), ALL_STREAMS.len());
}

#[test]
fn indexes_are_unique() {
    let mut v = ALL_INDEXES.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), ALL_INDEXES.len());
}

#[test]
fn all_streams_prefixed() {
    for s in ALL_STREAMS {
        assert!(
            s.starts_with("cortex."),
            "stream `{s}` missing cortex. prefix"
        );
    }
}

#[test]
fn all_indexes_prefixed() {
    for i in ALL_INDEXES {
        assert!(
            i.starts_with("cortex_"),
            "index `{i}` missing cortex_ prefix"
        );
    }
}
