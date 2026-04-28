//! Integration tests for `cortex_graph::identity`.

use cortex_graph::identity::{artifact_natural_key, symbol_natural_key};

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

#[test]
fn symbol_key_is_pipe_joined_repo_language_qname() {
    assert_eq!(
        symbol_natural_key("Cortex", "rust", "PreThinkingTool"),
        "Cortex|rust|PreThinkingTool"
    );
}

#[test]
fn symbol_key_distinguishes_same_name_in_different_languages() {
    let rust = symbol_natural_key("Cortex", "rust", "parse");
    let python = symbol_natural_key("Cortex", "python", "parse");
    assert_ne!(rust, python, "language must change the natural key");
}

#[test]
fn symbol_key_is_deterministic_across_calls() {
    let a = symbol_natural_key("r", "rust", "Foo");
    let b = symbol_natural_key("r", "rust", "Foo");
    assert_eq!(a, b);
}
