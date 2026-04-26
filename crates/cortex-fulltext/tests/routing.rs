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
fn index_for_uses_prefix_and_family() {
    assert_eq!(index_for("cortex-", Kind::ToolCall), "cortex-code");
    assert_eq!(index_for("cortex-", Kind::Decision), "cortex-decisions");
    assert_eq!(index_for("cortex-", Kind::Turn), "cortex-turns");
    assert_eq!(index_for("cortex-", Kind::LawViolation), "cortex-governance");
    assert_eq!(index_for("cortex-", Kind::Artifact), "cortex-docs");
    assert_eq!(index_for("cortex-", Kind::Memory), "cortex-misc");
    assert_eq!(index_for("staging-", Kind::Turn), "staging-turns");
}
