//! Deterministic dedup-key derivation.
//!
//! Cortex embeds chunks into a Vectorizer server whose `POST /insert_texts`
//! endpoint discards any client-supplied `id` and assigns its own UUID per
//! stored vector (tracked as server bug #4 in the knowledge base). Rather
//! than fake a primary id, the embedder treats the server's UUID as the
//! canonical chunk id and keeps a **client-side dedup key** in the vector's
//! metadata. The dedup key is a ULID derived from a SHA-256 digest of
//! `event_id || ':' || ordinal || ':' || chunk_content_hash`. The ULID's
//! 16 bytes are taken from the first 16 bytes of the digest, so the same
//! inputs always yield the same key — the orchestrator's pre-upsert
//! `list_stored_dedup_keys` scan then short-circuits re-runs.

use sha2::{Digest, Sha256};
use ulid::Ulid;

/// Derive a deterministic dedup key.
///
/// Given a parent event id, a 0-based ordinal within that event, and the
/// chunk's own content hash, return a ULID-encoded string that is stable
/// across re-runs. The return value is stored as `metadata.dedup_key` on
/// the upserted vector; the server-assigned UUID is the actual primary id.
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
