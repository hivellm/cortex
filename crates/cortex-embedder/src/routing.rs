//! Per-`Kind` collection routing.
//!
//! The table follows `docs/specs/06-embedder.md` §Collections. The returned
//! string is `"{prefix}-{suffix}"` — e.g. `cortex-code`, `cortex-docs`.

use cortex_core::events::Kind;

/// Return the collection name for a given event kind and deployment prefix.
pub fn collection_for(kind: &Kind, prefix: &str) -> String {
    let suffix = match kind {
        // tool_call.* — code symbols (spec: "tool_call.* (code)" → cortex-code).
        Kind::ToolCall => "code",
        // artifact.doc — but we only have the coarse `Artifact` kind; docs live
        // in `cortex-docs`.
        Kind::Artifact => "docs",
        Kind::Decision => "decisions",
        Kind::Turn => "turns",
        Kind::LawViolation => "governance",
        // Catch-all for kinds not explicitly called out in the spec table.
        Kind::AgentCall | Kind::Memory | Kind::Analysis => "misc",
    };
    format!("{prefix}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
