//! Deterministic dedup-key and stable vector-id derivation.
//!
//! Cortex embeds chunks into a Vectorizer server whose `POST /insert_texts`
//! endpoint upserts by the client-supplied `id` field. Two distinct ids are
//! maintained per chunk:
//!
//! * [`dedup_key`] — content-dependent (folds the chunk content hash). Stored
//!   as `metadata.dedup_key` on the upserted vector. Used by the
//!   `exists_by_dedup_key` pre-check to skip re-embedding when content is
//!   unchanged.
//!
//! * [`vector_id`] — content-independent (event id + ordinal only). This is
//!   the id sent to `insert_texts`. Because the Vectorizer upserts by id, a
//!   re-embed with changed content reuses the same id and REPLACES the prior
//!   vector in place, preventing orphaned vectors (phase26e §1.2).

use sha2::{Digest, Sha256};
use ulid::Ulid;

/// Derive a deterministic dedup key.
///
/// Given a parent event id, a 0-based ordinal within that event, and the
/// chunk's own content hash, return a ULID-encoded string that is stable
/// across re-runs. The return value is stored as `metadata.dedup_key` on
/// the upserted vector and used by the `exists_by_dedup_key` pre-check to
/// skip re-embedding when content is unchanged.
pub fn dedup_key(event_id: &str, ordinal: u32, chunk_content_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_bytes());
    hasher.update(b":");
    hasher.update(ordinal.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(chunk_content_hash.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Ulid::from_bytes(bytes).to_string()
}

/// Derive a deterministic, CONTENT-INDEPENDENT vector id.
///
/// Unlike [`dedup_key`], this folds ONLY the parent event id and the
/// chunk ordinal — never the content hash — so re-embedding the same
/// (event, ordinal) with changed text yields the SAME id. The Vectorizer
/// `insert_texts` endpoint upserts by id, so a stable id makes a re-embed
/// REPLACE the prior vector in place instead of orphaning it (phase26e §1.2).
pub fn vector_id(event_id: &str, ordinal: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_bytes());
    hasher.update(b":");
    hasher.update(ordinal.to_string().as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Ulid::from_bytes(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_id_is_deterministic() {
        assert_eq!(vector_id("event-abc", 0), vector_id("event-abc", 0));
        assert_eq!(vector_id("event-xyz", 7), vector_id("event-xyz", 7));
    }

    #[test]
    fn vector_id_differs_by_ordinal() {
        assert_ne!(vector_id("event-abc", 0), vector_id("event-abc", 1));
    }

    #[test]
    fn vector_id_is_content_independent() {
        // No content argument means two calls with the same (event, ordinal)
        // always return the same value regardless of chunk text.
        let id1 = vector_id("event-abc", 0);
        let id2 = vector_id("event-abc", 0);
        assert_eq!(id1, id2);
    }

    #[test]
    fn vector_id_differs_from_dedup_key() {
        let vid = vector_id("event-abc", 0);
        let dkey = dedup_key("event-abc", 0, "some-content-hash");
        assert_ne!(vid, dkey);
    }
}
