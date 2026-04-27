//! Per-kind, per-repo index routing.
//!
//! Spec 08 §Indexes lays out the family-to-index mapping. Index names
//! embed the owning repo's slug so each project lives in isolation —
//! `cortex-{repo_slug}-{family}` — and queries can scope deterministically
//! to a single project.

use cortex_core::events::Kind;
use cortex_storage::names::{slug_for_repo, UNKNOWN_REPO_SLUG};

/// Family suffix for the index a `Kind` routes to. Spec 08 §Indexes:
/// `tool_call` ⇒ `code`, `decision` ⇒ `decisions`, `turn` ⇒ `turns`,
/// `law_violation` ⇒ `governance`, `artifact` (default) ⇒ `docs`,
/// anything else ⇒ `misc`.
pub fn family_for(kind: Kind) -> &'static str {
    match kind {
        Kind::ToolCall | Kind::AgentCall => "code",
        Kind::Decision => "decisions",
        Kind::Turn => "turns",
        Kind::LawViolation => "governance",
        Kind::Artifact => "docs",
        Kind::Memory | Kind::Analysis => "misc",
    }
}

/// Compose the full index name from a prefix, repo slug, and family.
/// `prefix` is the deployment namespace (default `"cortex-"`); the
/// trailing dash is honoured so the result is `cortex-{slug}-{family}`.
pub fn index_name(prefix: &str, repo_slug: &str, family: &str) -> String {
    let trimmed = prefix.trim_end_matches('-');
    format!("{trimmed}-{repo_slug}-{family}")
}

/// Compose the full index name for a `Kind` + repo. `repo_id = None`
/// (or empty) falls back to [`UNKNOWN_REPO_SLUG`] so the produced
/// name is always well-formed.
pub fn index_for(prefix: &str, kind: Kind, repo_id: Option<&str>) -> String {
    let slug = repo_id
        .map(slug_for_repo)
        .unwrap_or_else(|| UNKNOWN_REPO_SLUG.to_string());
    index_name(prefix, &slug, family_for(kind))
}

/// All known family suffixes in deterministic order. Used by tests that
/// need to iterate every family. Bootstrap is now lazy per-repo, so
/// this is no longer driven into a fixed `ensure_index` loop.
pub const FAMILIES: &[&str] = &["code", "docs", "decisions", "turns", "governance", "misc"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_for_artifact_uses_per_repo_docs() {
        assert_eq!(
            index_for("cortex-", Kind::Artifact, Some("Cortex")),
            "cortex-cortex-docs"
        );
        assert_eq!(
            index_for("cortex-", Kind::Artifact, Some("Tml")),
            "cortex-tml-docs"
        );
    }

    #[test]
    fn index_for_falls_back_to_unknown_slug() {
        assert_eq!(
            index_for("cortex-", Kind::ToolCall, None),
            "cortex-unknown-code"
        );
        assert_eq!(
            index_for("cortex-", Kind::ToolCall, Some("")),
            "cortex-unknown-code"
        );
    }

    #[test]
    fn prefix_without_trailing_dash_still_works() {
        assert_eq!(
            index_for("cortex", Kind::Turn, Some("Cortex")),
            "cortex-cortex-turns"
        );
    }
}
