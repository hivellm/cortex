//! SHA-256 hash over canonical-JSON payload bytes.
//!
//! `content_hash` is the secondary identity used by the classifier cache
//! (spec 05) and the embedder dedup (spec 06). Identical payloads from
//! different sources collide on purpose.

use crate::canonical_json::{canonicalize, CanonicalJsonError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

/// A prefixed SHA-256 hash (`sha256:<64-hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    /// Wrap an already-formatted `sha256:...` string. Returns `None` if malformed.
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(hex) = s.strip_prefix("sha256:") {
            if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(ContentHash(s.to_string()));
            }
        }
        None
    }

    /// Raw `sha256:<hex>` view.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Borrow the hex portion (64 chars).
    pub fn hex(&self) -> &str {
        &self.0[7..]
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compute the content hash over the canonical-JSON encoding of `payload`.
///
/// This is computed **pre-redaction** — adapters are expected to call this
/// before passing the payload to the redactor so the hash represents the
/// semantic content, not the post-redaction form.
pub fn content_hash(payload: &Value) -> Result<ContentHash, CanonicalJsonError> {
    let bytes = canonicalize(payload)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    Ok(ContentHash(out))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(ContentHash::parse("sha256:toolong_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").is_none());
        assert!(ContentHash::parse("sha256:xyz").is_none());
        assert!(ContentHash::parse("md5:abc").is_none());
    }

    #[test]
    fn known_vector() {
        // Empty object canonicalizes to `{}`; hash of "{}" is stable.
        let h = content_hash(&json!({})).unwrap();
        assert_eq!(
            h.as_str(),
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }
}
