//! Integration tests for `cortex_workers::graph::schema`.
//!
//! Moved out of `src/schema.rs` so every test in this crate lives under
//! `tests/`. Exercises the public schema bootstrap surface only.

use cortex_workers::graph::schema::{statements, SCHEMA_STATEMENTS};

#[test]
fn statements_are_non_empty_and_owned() {
    let owned = statements();
    assert_eq!(owned.len(), SCHEMA_STATEMENTS.len());
    assert!(!owned.is_empty());
    for (b, o) in SCHEMA_STATEMENTS.iter().zip(owned.iter()) {
        assert_eq!(*b, o.as_str());
    }
}

#[test]
fn schema_covers_every_expected_label() {
    let joined = SCHEMA_STATEMENTS.join("\n");
    for label in [
        "Session",
        "Turn",
        "ToolCall",
        "Artifact",
        "Decision",
        "Memory",
        "Analysis",
        "Law",
        "LawViolation",
        "Repo",
    ] {
        assert!(
            joined.contains(label),
            "schema bootstrap missing label `{label}`"
        );
    }
}
