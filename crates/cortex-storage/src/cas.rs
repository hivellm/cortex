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

    /// Phase9c — list vacuum-eligible blobs (refcount == 0 AND
    /// last_referenced < cutoff). Returns at most `limit` rows in
    /// stable hash-order so a batched delete can checkpoint mid-run.
    pub fn select_vacuumable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<VacuumCandidate>, CasError> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, size FROM cas_blobs
              WHERE refcount = 0 AND last_referenced < ?1
              ORDER BY hash
              LIMIT ?2",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(params![cutoff.to_rfc3339(), limit as i64], |row| {
                Ok(VacuumCandidate {
                    hash: row.get::<_, String>(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                })
            })?
            .collect();
        Ok(rows?)
    }

    /// Phase9c — delete every row whose hash appears in `hashes`,
    /// inside one `BEGIN IMMEDIATE` transaction. Returns the number
    /// of rows actually deleted (may differ from `hashes.len()` when
    /// a concurrent ingestion path bumped the refcount mid-flight).
    pub fn delete_blobs(&mut self, hashes: &[String]) -> Result<u64, CasError> {
        if hashes.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut total: u64 = 0;
        {
            let mut stmt = tx.prepare(
                "DELETE FROM cas_blobs WHERE hash = ?1 AND refcount = 0",
            )?;
            for h in hashes {
                total += stmt.execute(params![h])? as u64;
            }
        }
        tx.commit()?;
        Ok(total)
    }

    /// Phase9c — total number of blobs in the store, used by the
    /// safeguard (`would_drop / total_blobs > 0.5` ⇒ refuse without
    /// `--force`).
    pub fn total_blob_count(&self) -> Result<u64, CasError> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM cas_blobs", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// Phase9c — sum of `size` (uncompressed bytes) across all
    /// blobs. Used by the report's `bytes_reclaimed` estimate.
    pub fn total_blob_bytes(&self) -> Result<u64, CasError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(size), 0) FROM cas_blobs",
                [],
                |r| r.get(0),
            )?;
        Ok(n.max(0) as u64)
    }

    /// Phase9c — borrow the underlying connection mutably so the
    /// runner can issue `VACUUM` against the same `CasStore`.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// One row returned by [`CasStore::select_vacuumable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacuumCandidate {
    /// Prefixed hash (`sha256:<hex>`).
    pub hash: String,
    /// Original (uncompressed) byte size — used by the runner to
    /// project `bytes_reclaimed` before the actual delete.
    pub size: u64,
}

impl CasStore {
    /// Borrow the underlying SQLite connection. Used by integration tests
    /// that need to assert raw row counts or compressed-blob byte sizes.
    pub fn conn(&self) -> &Connection {
        &self.conn
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

