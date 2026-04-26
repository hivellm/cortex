//! Integration tests for `cortex_graph::identity`.

use cortex_graph::identity::artifact_natural_key;

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
