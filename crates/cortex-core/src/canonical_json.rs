//! Canonical-JSON serializer used to compute stable [`content_hash`](crate::content_hash)
//! values across platforms.
//!
//! Rules (per spec 01 §Identity):
//! - Keys in objects are sorted lexicographically by their UTF-8 bytes.
//! - No insignificant whitespace.
//! - UTF-8 output.
//! - Numbers in shortest-roundtrip form via `serde_json`'s default formatter.
//! - Null, boolean, string, array, and object values are preserved verbatim.

use serde_json::Value;
use std::io::{self, Write};

/// Errors returned by [`canonicalize`].
#[derive(Debug, thiserror::Error)]
pub enum CanonicalJsonError {
    /// Underlying I/O failure while building the canonical string.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// JSON (de)serialization failure.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Canonicalize a JSON value into a stable byte representation.
///
/// Two inputs that are semantically equal produce byte-identical output,
/// regardless of key insertion order or non-significant whitespace.
pub fn canonicalize(value: &Value) -> Result<Vec<u8>, CanonicalJsonError> {
    let mut buf = Vec::with_capacity(256);
    write_value(&mut buf, value)?;
    Ok(buf)
}

fn write_value<W: Write>(w: &mut W, value: &Value) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => w.write_all(b"null")?,
        Value::Bool(true) => w.write_all(b"true")?,
        Value::Bool(false) => w.write_all(b"false")?,
        Value::Number(n) => {
            // Delegate to serde_json so numeric formatting matches the shortest
            // round-trip form (ints stay ints, floats keep minimal digits).
            let s = serde_json::to_string(n)?;
            w.write_all(s.as_bytes())?;
        }
        Value::String(s) => {
            let encoded = serde_json::to_string(s)?;
            w.write_all(encoded.as_bytes())?;
        }
        Value::Array(items) => {
            w.write_all(b"[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                write_value(w, item)?;
            }
            w.write_all(b"]")?;
        }
        Value::Object(map) => {
            // serde_json with "preserve_order" keeps insertion order; we sort here.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            w.write_all(b"{")?;
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                let encoded_key = serde_json::to_string(key)?;
                w.write_all(encoded_key.as_bytes())?;
                w.write_all(b":")?;
                write_value(w, &map[*key])?;
            }
            w.write_all(b"}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        // serde_json escapes non-ASCII by default; canonical form matches that.
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
}
