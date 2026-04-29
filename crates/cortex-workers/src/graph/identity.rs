//! Stable-identity helpers for graph nodes whose key is composite.
//!
//! Per spec 07 §Stable identity, `Artifact` is the one composite case in
//! the schema: its natural key is `repo|path|content_hash`. Nexus has a
//! unique constraint on `Artifact.natural_key`. All other node labels use
//! a single string key and don't need a helper here.

/// Build the canonical `Artifact.natural_key` from `(repo, path, content_hash)`.
///
/// The triple is concatenated with the pipe character `|`, which is
/// rejected by the redactor for any of the three components — so the
/// produced key is unambiguous.
///
/// # Examples
///
/// ```
/// use cortex_workers::graph::identity::artifact_natural_key;
///
/// let key = artifact_natural_key("hivellm/cortex", "src/lib.rs", "abc123");
/// assert_eq!(key, "hivellm/cortex|src/lib.rs|abc123");
/// ```
pub fn artifact_natural_key(repo: &str, path: &str, content_hash: &str) -> String {
    format!("{repo}|{path}|{content_hash}")
}

/// Build the canonical `Symbol.natural_key` from `(repo, language, qualified_name)`.
///
/// Phase4c §1.2 — `qualified_name` is the most specific identifier the
/// chunker can produce (e.g. `crate::module::Type` for Rust, the
/// definition name verbatim otherwise). When the language has no
/// namespace concept (or the chunker only surfaced the bare name),
/// the caller MUST fold the artifact path into `qualified_name` so
/// two `parse()` functions in different files hash to distinct
/// Symbols. The triple is concatenated with `|`, mirroring
/// [`artifact_natural_key`] so the same redactor invariant applies
/// (the pipe character is rejected on each component upstream).
///
/// # Examples
///
/// ```
/// use cortex_workers::graph::identity::symbol_natural_key;
///
/// let key = symbol_natural_key("Cortex", "rust", "PreThinkingTool");
/// assert_eq!(key, "Cortex|rust|PreThinkingTool");
/// ```
pub fn symbol_natural_key(repo: &str, language: &str, qualified_name: &str) -> String {
    format!("{repo}|{language}|{qualified_name}")
}
