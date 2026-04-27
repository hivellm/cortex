//! Integration tests for `cortex_embedder::routing`.

use cortex_core::events::Kind;
use cortex_embedder::collection_for;

#[test]
fn routes_known_kinds_with_repo_slug() {
    let r = Some("Cortex");
    assert_eq!(
        collection_for(&Kind::ToolCall, "cortex", r),
        "cortex-cortex-code"
    );
    assert_eq!(
        collection_for(&Kind::Artifact, "cortex", r),
        "cortex-cortex-docs"
    );
    assert_eq!(
        collection_for(&Kind::Decision, "cortex", r),
        "cortex-cortex-decisions"
    );
    assert_eq!(
        collection_for(&Kind::Turn, "cortex", r),
        "cortex-cortex-turns"
    );
    assert_eq!(
        collection_for(&Kind::LawViolation, "cortex", r),
        "cortex-cortex-governance"
    );
    assert_eq!(
        collection_for(&Kind::Memory, "cortex", r),
        "cortex-cortex-misc"
    );
}

#[test]
fn honors_prefix_and_slug() {
    assert_eq!(
        collection_for(&Kind::Turn, "dev", Some("Tml")),
        "dev-tml-turns"
    );
}

#[test]
fn missing_repo_falls_back_to_unknown_slug() {
    assert_eq!(
        collection_for(&Kind::Artifact, "cortex", None),
        "cortex-unknown-docs"
    );
}
