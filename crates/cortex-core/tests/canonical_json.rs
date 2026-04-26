//! Integration tests for `cortex_core::canonical_json`.

use cortex_core::canonicalize;
use serde_json::{json, Value};

#[test]
fn key_order_is_stable() {
    let a = json!({ "b": 1, "a": 2, "c": 3 });
    let b = json!({ "c": 3, "a": 2, "b": 1 });
    assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
}

#[test]
fn nested_objects_are_sorted_too() {
    let v = json!({
        "outer": { "z": 1, "a": 2 },
        "arr": [ { "b": 1, "a": 2 }, { "a": 2, "b": 1 } ]
    });
    let out = String::from_utf8(canonicalize(&v).unwrap()).unwrap();
    assert_eq!(
        out,
        r#"{"arr":[{"a":2,"b":1},{"a":2,"b":1}],"outer":{"a":2,"z":1}}"#
    );
}

#[test]
fn preserves_unicode() {
    let v = json!({ "pt": "alçada", "jp": "テスト" });
    let out = String::from_utf8(canonicalize(&v).unwrap()).unwrap();
    assert!(out.contains("alçada") || out.contains("al\\u00e7ada"));
}

#[test]
fn whitespace_is_insignificant() {
    let a: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
    let b: Value = serde_json::from_str("{  \"a\" : 1 , \"b\" : 2  }").unwrap();
    assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
}

#[test]
fn null_and_booleans() {
    assert_eq!(canonicalize(&json!(null)).unwrap(), b"null");
    assert_eq!(canonicalize(&json!(true)).unwrap(), b"true");
    assert_eq!(canonicalize(&json!(false)).unwrap(), b"false");
}
