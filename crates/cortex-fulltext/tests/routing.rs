//! Integration tests for the per-kind index routing.

use cortex_core::events::Kind;
use cortex_fulltext::{family_for, index_for, FAMILIES};

#[test]
fn each_kind_routes_to_a_known_family() {
    for kind in [
        Kind::ToolCall,
        Kind::AgentCall,
        Kind::Decision,
        Kind::Turn,
        Kind::LawViolation,
        Kind::Artifact,
        Kind::Memory,
        Kind::Analysis,
    ] {
        let family = family_for(kind);
        assert!(
            FAMILIES.contains(&family),
            "family {family} for kind {kind:?} not in FAMILIES"
        );
    }
}

#[test]
fn index_for_uses_prefix_repo_slug_and_family() {
    let repo = Some("Cortex");
    assert_eq!(index_for("cortex-", Kind::ToolCall, repo), "cortex-cortex-code");
    assert_eq!(index_for("cortex-", Kind::Decision, repo), "cortex-cortex-decisions");
    assert_eq!(index_for("cortex-", Kind::Turn, repo), "cortex-cortex-turns");
    assert_eq!(
        index_for("cortex-", Kind::LawViolation, repo),
        "cortex-cortex-governance"
    );
    assert_eq!(index_for("cortex-", Kind::Artifact, repo), "cortex-cortex-docs");
    assert_eq!(index_for("cortex-", Kind::Memory, repo), "cortex-cortex-misc");
    assert_eq!(
        index_for("staging-", Kind::Turn, Some("Tml")),
        "staging-tml-turns"
    );
}

#[test]
fn missing_repo_routes_to_unknown_slug() {
    assert_eq!(
        index_for("cortex-", Kind::Artifact, None),
        "cortex-unknown-docs"
    );
}
