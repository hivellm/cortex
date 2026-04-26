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
/// use cortex_graph::identity::artifact_natural_key;
///
/// let key = artifact_natural_key("hivellm/cortex", "src/lib.rs", "abc123");
/// assert_eq!(key, "hivellm/cortex|src/lib.rs|abc123");
/// ```
pub fn artifact_natural_key(repo: &str, path: &str, content_hash: &str) -> String {
    format!("{repo}|{path}|{content_hash}")
}
