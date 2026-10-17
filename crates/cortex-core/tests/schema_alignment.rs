//! `vocab.rs` lists must match the `enum` lists of
//! `schemas/envelope.schema.json` (phase36 §1.1: `opencode` was in the
//! schema but missing from `TOOL_IDS`).

use cortex_core::vocab::{STREAM_IDS, TOOL_IDS};
use serde_json::Value;

fn schema_enum(property: &str) -> Vec<String> {
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/envelope.schema.json")).unwrap();
    schema["properties"][property]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("schema property `{property}` has no enum"))
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn vocab_matches_schema() {
    assert_eq!(TOOL_IDS, schema_enum("tool").as_slice(), "tool ids");
    assert_eq!(STREAM_IDS, schema_enum("stream").as_slice(), "stream ids");
}
