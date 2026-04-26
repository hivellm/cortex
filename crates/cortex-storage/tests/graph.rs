//! Integration tests for `cortex_storage::graph`.

use cortex_storage::graph::{BOOTSTRAP_STATEMENTS, LABELS, RELATIONSHIPS};

#[test]
fn labels_are_unique() {
    let mut v = LABELS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), LABELS.len());
}

#[test]
fn relationships_are_unique() {
    let mut v = RELATIONSHIPS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), RELATIONSHIPS.len());
}

#[test]
fn bootstrap_statements_are_idempotent() {
    for stmt in BOOTSTRAP_STATEMENTS {
        assert!(
            stmt.contains("IF NOT EXISTS"),
            "non-idempotent statement: {stmt}"
        );
    }
}
