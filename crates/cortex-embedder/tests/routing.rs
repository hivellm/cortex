//! Integration tests for `cortex_embedder::routing`.

use cortex_core::events::Kind;
use cortex_embedder::collection_for;

#[test]
fn routes_known_kinds() {
    assert_eq!(collection_for(&Kind::ToolCall, "cortex"), "cortex-code");
    assert_eq!(collection_for(&Kind::Artifact, "cortex"), "cortex-docs");
    assert_eq!(collection_for(&Kind::Decision, "cortex"), "cortex-decisions");
    assert_eq!(collection_for(&Kind::Turn, "cortex"), "cortex-turns");
    assert_eq!(
        collection_for(&Kind::LawViolation, "cortex"),
        "cortex-governance"
    );
    assert_eq!(collection_for(&Kind::Memory, "cortex"), "cortex-misc");
}

#[test]
fn honors_prefix() {
    assert_eq!(collection_for(&Kind::Turn, "dev"), "dev-turns");
}
