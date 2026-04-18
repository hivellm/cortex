//! SQLite-backed content-addressable blob store.
//!
//! Blobs are Zstd-compressed on write, refcounted on reference, vacuumed
//! when `refcount == 0 AND last_referenced < now - 30 days` (the vacuum
//! sweep is driven by a CLI; see spec 02 §Design).

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Errors returned by [`CasStore`].
#[derive(Debug, thiserror::Error)]
pub enum CasError {
    /// SQLite failure.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Compression / decompression failure.
    #[error("zstd: {0}")]
    Zstd(#[from] std::io::Error),
    /// Requested a blob that isn't in the store.
    #[error("blob not found: {0}")]
    NotFound(String),
    /// Caller supplied the wrong hash for verification.
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Hash declared by the caller.
        expected: String,
        /// Hash actually computed.
        actual: String,
    },
}

/// Embedded DDL for the CAS tables.
pub const CAS_SCHEMA_SQL: &str = include_str!("../schemas/sqlite/cas.sql");

/// Content type tags recognized by the CAS store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasContentType {
    /// UTF-8 text.
    Text,
    /// Unified diff.
    Diff,
    /// JSON document.
    Json,
    /// Opaque binary.
    Binary,
}

impl CasContentType {
    /// Canonical MIME-style string stored in the `content_type` column.
    pub fn as_str(self) -> &'static str {
        match self {
            CasContentType::Text => "text/plain",
            CasContentType::Diff => "text/x-diff",
            CasContentType::Json => "application/json",
            CasContentType::Binary => "application/octet-stream",
        }
    }

    /// Parse from the stored string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text/plain" => Some(CasContentType::Text),
            "text/x-diff" => Some(CasContentType::Diff),
            "application/json" => Some(CasContentType::Json),
            "application/octet-stream" => Some(CasContentType::Binary),
            _ => None,
        }
    }
}

/// Decoded CAS record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasBlob {
    /// Prefixed hash (`sha256:<hex>`).
    pub hash: String,
    /// Original (pre-compression) size.
    pub size: u64,
    /// Content type.
    pub content_type: CasContentType,
    /// Decompressed bytes.
    pub bytes: Vec<u8>,
}

/// SQLite-backed content-addressable store.
pub struct CasStore {
    conn: Connection,
}

impl CasStore {
    /// Open or create the CAS database at `path`.
    pub fn open(path: &Path) -> Result<Self, CasError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        conn.execute_batch(CAS_SCHEMA_SQL)?;
        Ok(CasStore { conn })
    }

    /// Open an in-memory CAS store (test-only).
    pub fn open_in_memory() -> Result<Self, CasError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        conn.execute_batch(CAS_SCHEMA_SQL)?;
        Ok(CasStore { conn })
    }

    fn configure(conn: &Connection) -> rusqlite::Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(())
    }

    /// Insert a blob, compressing with Zstd. Returns the prefixed SHA-256 hash.
    pub fn put(&self, bytes: &[u8], content_type: CasContentType) -> Result<String, CasError> {
        let hash = compute_hash(bytes);
        let compressed = zstd::encode_all(bytes, super::ArchiveLayout::COMPRESSION_LEVEL)?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO cas_blobs (hash, size, content_type, blob, refcount, first_seen, last_referenced)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)
             ON CONFLICT(hash) DO UPDATE SET last_referenced = excluded.last_referenced",
            params![
                hash,
                bytes.len() as i64,
                content_type.as_str(),
                compressed,
                now
            ],
        )?;
        Ok(hash)
    }

    /// Increment the refcount on a blob that already exists.
    pub fn retain(&self, hash: &str) -> Result<(), CasError> {
        let updated = self.conn.execute(
            "UPDATE cas_blobs SET refcount = refcount + 1, last_referenced = ?1 WHERE hash = ?2",
            params![Utc::now().to_rfc3339(), hash],
        )?;
        if updated == 0 {
            return Err(CasError::NotFound(hash.to_string()));
        }
        Ok(())
    }

    /// Decrement the refcount on a blob. Refcount is clamped at zero.
    pub fn release(&self, hash: &str) -> Result<(), CasError> {
        let updated = self.conn.execute(
            "UPDATE cas_blobs SET refcount = CASE WHEN refcount > 0 THEN refcount - 1 ELSE 0 END
             WHERE hash = ?1",
            params![hash],
        )?;
        if updated == 0 {
            return Err(CasError::NotFound(hash.to_string()));
        }
        Ok(())
    }

    /// Fetch a blob, decompressing on the way out. Updates `last_referenced`.
    pub fn get(&self, hash: &str) -> Result<CasBlob, CasError> {
        let row = self
            .conn
            .query_row(
                "SELECT size, content_type, blob FROM cas_blobs WHERE hash = ?1",
                params![hash],
                |r| {
                    let size: i64 = r.get(0)?;
                    let ct: String = r.get(1)?;
                    let blob: Vec<u8> = r.get(2)?;
                    Ok((size, ct, blob))
                },
            )
            .optional()?;
        let (size, ct, compressed) = row.ok_or_else(|| CasError::NotFound(hash.to_string()))?;
        let bytes = zstd::decode_all(compressed.as_slice())?;
        // Integrity: verify hash matches the decompressed payload.
        let actual = compute_hash(&bytes);
        if actual != hash {
            return Err(CasError::HashMismatch {
                expected: hash.to_string(),
                actual,
            });
        }
        // Bump last_referenced.
        self.conn.execute(
            "UPDATE cas_blobs SET last_referenced = ?1 WHERE hash = ?2",
            params![Utc::now().to_rfc3339(), hash],
        )?;
        Ok(CasBlob {
            hash: hash.to_string(),
            size: size as u64,
            content_type: CasContentType::parse(&ct)
                .unwrap_or(CasContentType::Binary),
            bytes,
        })
    }

    /// Check presence without retrieving bytes.
    pub fn contains(&self, hash: &str) -> Result<bool, CasError> {
        let c: i64 = self.conn.query_row(
            "SELECT count(*) FROM cas_blobs WHERE hash = ?1",
            params![hash],
            |r| r.get(0),
        )?;
        Ok(c > 0)
    }

    /// Delete blobs whose `refcount == 0` and `last_referenced < cutoff`.
    /// Returns the number of rows dropped.
    pub fn vacuum(&self, cutoff: DateTime<Utc>) -> Result<u64, CasError> {
        let dropped = self.conn.execute(
            "DELETE FROM cas_blobs WHERE refcount = 0 AND last_referenced < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(dropped as u64)
    }

    /// Current refcount; 0 for absent blobs.
    pub fn refcount(&self, hash: &str) -> Result<u64, CasError> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT refcount FROM cas_blobs WHERE hash = ?1",
                params![hash],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.map(|x| x.max(0) as u64).unwrap_or(0))
    }
}

fn compute_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_round_trip() {
        let store = CasStore::open_in_memory().unwrap();
        let body = b"hello, cortex";
        let hash = store.put(body, CasContentType::Text).unwrap();
        let blob = store.get(&hash).unwrap();
        assert_eq!(blob.bytes, body);
        assert_eq!(blob.size, body.len() as u64);
        assert_eq!(blob.content_type, CasContentType::Text);
    }

    #[test]
    fn put_is_idempotent_on_hash() {
        let store = CasStore::open_in_memory().unwrap();
        let body = b"same body";
        let h1 = store.put(body, CasContentType::Text).unwrap();
        let h2 = store.put(body, CasContentType::Text).unwrap();
        assert_eq!(h1, h2);
        let count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM cas_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn refcount_lifecycle() {
        let store = CasStore::open_in_memory().unwrap();
        let h = store.put(b"refd", CasContentType::Binary).unwrap();
        assert_eq!(store.refcount(&h).unwrap(), 0);
        store.retain(&h).unwrap();
        store.retain(&h).unwrap();
        assert_eq!(store.refcount(&h).unwrap(), 2);
        store.release(&h).unwrap();
        assert_eq!(store.refcount(&h).unwrap(), 1);
        store.release(&h).unwrap();
        store.release(&h).unwrap(); // clamped
        assert_eq!(store.refcount(&h).unwrap(), 0);
    }

    #[test]
    fn vacuum_drops_only_expired_unreferenced() {
        let store = CasStore::open_in_memory().unwrap();
        let h = store.put(b"old", CasContentType::Binary).unwrap();
        // Move the timestamp into the past manually.
        store
            .conn
            .execute(
                "UPDATE cas_blobs SET last_referenced = ?1 WHERE hash = ?2",
                params!["2000-01-01T00:00:00+00:00", h],
            )
            .unwrap();
        let dropped = store.vacuum(Utc::now()).unwrap();
        assert_eq!(dropped, 1);
        assert!(!store.contains(&h).unwrap());
    }

    #[test]
    fn referenced_blob_survives_vacuum() {
        let store = CasStore::open_in_memory().unwrap();
        let h = store.put(b"keep", CasContentType::Binary).unwrap();
        store.retain(&h).unwrap();
        let dropped = store.vacuum(Utc::now()).unwrap();
        assert_eq!(dropped, 0);
        assert!(store.contains(&h).unwrap());
    }

    #[test]
    fn get_missing_is_not_found() {
        let store = CasStore::open_in_memory().unwrap();
        let err = store.get("sha256:deadbeef").unwrap_err();
        matches!(err, CasError::NotFound(_));
    }

    #[test]
    fn compression_actually_compresses_repetitive_input() {
        let store = CasStore::open_in_memory().unwrap();
        let body = vec![b'x'; 4096];
        let h = store.put(&body, CasContentType::Binary).unwrap();
        let stored: Vec<u8> = store
            .conn
            .query_row(
                "SELECT blob FROM cas_blobs WHERE hash = ?1",
                params![h],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored.len() < body.len() / 4);
    }
}
