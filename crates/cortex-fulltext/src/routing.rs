//! Per-kind index routing.
//!
//! Spec 08 §Indexes lays out the family-to-index mapping. The `family`
//! suffix gets prepended with the configured `index_prefix` to form the
//! actual Meilisearch index name (default prefix is `cortex-`, matching
//! the embedder collection convention).

use cortex_core::events::Kind;

/// Family suffix for the index a `Kind` routes to. Spec 08 §Indexes:
/// `tool_call` ⇒ `cortex-code`, `decision` ⇒ `cortex-decisions`,
/// `turn` ⇒ `cortex-turns`, `law_violation` ⇒ `cortex-governance`,
/// `artifact` (default) ⇒ `cortex-docs`, anything else ⇒ `cortex-misc`.
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

/// Compose the full index name from a prefix + family.
pub fn index_name(prefix: &str, family: &str) -> String {
    format!("{prefix}{family}")
}

/// Compose the full index name for a `Kind` directly. Convenience
/// wrapper around [`family_for`] + [`index_name`].
pub fn index_for(prefix: &str, kind: Kind) -> String {
    index_name(prefix, family_for(kind))
}

/// All known family suffixes in deterministic order. Used by
/// `MeiliClient::ensure_index` at startup so every index is bootstrapped
/// before the first event lands, and by acceptance tests that need to
/// iterate every Phase-1 family.
pub const FAMILIES: &[&str] = &["code", "docs", "decisions", "turns", "governance", "misc"];
