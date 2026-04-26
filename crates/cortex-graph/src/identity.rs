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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_key_is_pipe_joined() {
        assert_eq!(
            artifact_natural_key("repo", "path", "hash"),
            "repo|path|hash"
        );
    }

    #[test]
    fn artifact_key_handles_nested_paths() {
        assert_eq!(
            artifact_natural_key("hivellm/cortex", "crates/cortex-graph/src/lib.rs", "deadbeef"),
            "hivellm/cortex|crates/cortex-graph/src/lib.rs|deadbeef"
        );
    }

    #[test]
    fn artifact_key_is_deterministic() {
        let a = artifact_natural_key("r", "p", "h");
        let b = artifact_natural_key("r", "p", "h");
        assert_eq!(a, b);
    }
}
