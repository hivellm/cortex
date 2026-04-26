//! Integration tests for `cortex_core::content_hash`.

use cortex_core::{content_hash, ContentHash};
use serde_json::json;

#[test]
fn stable_across_key_order() {
    let a = json!({ "b": 1, "a": 2 });
    let b = json!({ "a": 2, "b": 1 });
    assert_eq!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
}

#[test]
fn different_values_different_hash() {
    let a = json!({ "a": 1 });
    let b = json!({ "a": 2 });
    assert_ne!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
}

#[test]
fn prefix_and_length() {
    let h = content_hash(&json!({ "foo": "bar" })).unwrap();
    assert!(h.as_str().starts_with("sha256:"));
    assert_eq!(h.hex().len(), 64);
}

#[test]
fn parse_round_trip() {
    let h = content_hash(&json!(42)).unwrap();
    let parsed = ContentHash::parse(h.as_str()).unwrap();
    assert_eq!(parsed, h);
}

#[test]
fn parse_rejects_malformed() {
    assert!(ContentHash::parse(
        "sha256:toolong_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    )
    .is_none());
    assert!(ContentHash::parse("sha256:xyz").is_none());
    assert!(ContentHash::parse("md5:abc").is_none());
}

#[test]
fn known_vector() {
    let h = content_hash(&json!({})).unwrap();
    assert_eq!(
        h.as_str(),
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
}
