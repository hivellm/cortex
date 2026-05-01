//! SQLite-backed `api_keys` table — phase3 §7.2.
//!
//! Stores Argon2id digests of every dashboard API key issued via
//! the `cortex-api admin issue-api-key` subcommand. Cleartext keys
//! are printed once at issue time; only the digest persists.
//!
//! Schema:
//!
//! ```sql
//! CREATE TABLE api_keys (
//!   id            TEXT PRIMARY KEY,           -- ULID
//!   scope         TEXT NOT NULL,              -- e.g. "dashboard"
//!   label         TEXT NOT NULL,
//!   hash          TEXT NOT NULL,              -- argon2id PHC string
//!   created_at    INTEGER NOT NULL,           -- epoch ms
//!   last_used_at  INTEGER,                    -- epoch ms; NULL until first use
//!   revoked_at    INTEGER                     -- epoch ms; NULL while active
//! );
//! ```
//!
//! Keys are 32 random bytes drawn from `OsRng`, encoded with
//! Crockford base32 (lower case, no padding) and prefixed
//! `cortex_dash_`. `verify` compares a candidate cleartext against
//! every active row by walking the small (expected single-digit)
//! key set; constant-time compare is used inside Argon2id so an
//! attacker can't time which row matched.

use std::path::Path;
use std::sync::{Arc, Mutex};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::Utc;
use data_encoding::BASE32_NOPAD;
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;
use ulid::Ulid;

/// Wire-format prefix every minted key carries. Lets the renderer
/// reject obviously-malformed pastes before round-tripping to the
/// daemon.
pub const KEY_PREFIX: &str = "cortex_dash_";

/// 32 raw bytes of OsRng entropy → 52 chars when base32-encoded
/// (no padding). The full wire-format key is therefore exactly
/// `cortex_dash_` (12 chars) + 52 chars = 64 chars.
pub const KEY_RAW_BYTES: usize = 32;

/// Errors surfaced by the `api_keys` module. Wrapping `rusqlite`
/// + `argon2` so callers can match on either layer when needed.
#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("argon2 error: {0}")]
    Argon2(String),
    #[error("encoding error: {0}")]
    Encoding(String),
    #[error("key has been revoked")]
    Revoked,
    #[error("key not found")]
    NotFound,
}

impl From<argon2::password_hash::Error> for ApiKeyError {
    fn from(e: argon2::password_hash::Error) -> Self {
        ApiKeyError::Argon2(e.to_string())
    }
}

/// Snapshot of one row in the `api_keys` table. `hash` is omitted
/// from this struct because callers should never see it — admin
/// list output, the middleware verify path, and tests all read
/// from the table directly through helpers in this module.
#[derive(Debug, Clone)]
pub struct ApiKeyRecord {
    pub id: String,
    pub scope: String,
    pub label: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

/// In-process handle around a single SQLite connection guarded by
/// a mutex. Cheap to clone — the inner `Arc<Mutex<Connection>>` is
/// shared across the daemon. Read + write paths are short so the
/// global lock does not become a contention hot-spot in practice.
#[derive(Clone)]
pub struct ApiKeyStore {
    conn: Arc<Mutex<Connection>>,
}

const SCHEMA_DDL: &str = "CREATE TABLE IF NOT EXISTS api_keys (
   id            TEXT PRIMARY KEY,
   scope         TEXT NOT NULL,
   label         TEXT NOT NULL,
   hash          TEXT NOT NULL,
   created_at    INTEGER NOT NULL,
   last_used_at  INTEGER,
   revoked_at    INTEGER
);
CREATE INDEX IF NOT EXISTS api_keys_scope_idx ON api_keys(scope);";

impl ApiKeyStore {
    /// Open or create the SQLite database at `path` and apply the
    /// `api_keys` schema migration. The parent directory must
    /// already exist; callers (typically `main.rs` boot path) are
    /// responsible for creating it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ApiKeyError> {
        let conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA_DDL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory store. Used by ITs and the smoke tests in
    /// this module — boots without touching the filesystem.
    pub fn open_in_memory() -> Result<Self, ApiKeyError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_DDL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Mint a new key. Generates 32 OsRng bytes, encodes them with
    /// Crockford base32, prefixes with `cortex_dash_`, persists the
    /// Argon2id digest, and returns the full cleartext key (the
    /// caller prints it once and lets the user copy it). Subsequent
    /// reads can never recover the cleartext.
    pub fn issue(&self, scope: &str, label: &str) -> Result<IssuedKey, ApiKeyError> {
        let mut raw = [0u8; KEY_RAW_BYTES];
        OsRng.fill_bytes(&mut raw);
        let cleartext = format!(
            "{}{}",
            KEY_PREFIX,
            BASE32_NOPAD.encode(&raw).to_lowercase(),
        );
        let salt = SaltString::generate(&mut OsRng);
        let argon = Argon2::default();
        let hash = argon
            .hash_password(cleartext.as_bytes(), &salt)?
            .to_string();
        let id = Ulid::new().to_string();
        let created_at_ms = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        conn.execute(
            "INSERT INTO api_keys (id, scope, label, hash, created_at, last_used_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
            params![&id, scope, label, &hash, created_at_ms],
        )?;
        Ok(IssuedKey {
            id,
            cleartext,
            scope: scope.to_string(),
            label: label.to_string(),
            created_at_ms,
        })
    }

    /// List every key in the table. Hashes are stripped before
    /// return so callers cannot accidentally print them.
    pub fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyError> {
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, scope, label, created_at, last_used_at, revoked_at
             FROM api_keys
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ApiKeyRecord {
                id: row.get(0)?,
                scope: row.get(1)?,
                label: row.get(2)?,
                created_at_ms: row.get(3)?,
                last_used_at_ms: row.get::<_, Option<i64>>(4)?,
                revoked_at_ms: row.get::<_, Option<i64>>(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Soft-revoke a key by id. The middleware checks `revoked_at`
    /// on every request; a revoked key 401s on the next call.
    /// Idempotent — re-revoking a revoked key is a no-op that
    /// preserves the original timestamp.
    pub fn revoke(&self, id: &str) -> Result<(), ApiKeyError> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        let updated = conn.execute(
            "UPDATE api_keys SET revoked_at = ?1
             WHERE id = ?2 AND revoked_at IS NULL",
            params![now, id],
        )?;
        if updated == 0 {
            // Distinguish "doesn't exist" from "already revoked";
            // the admin CLI surfaces both as success but logs the
            // distinction.
            let exists: Option<String> = conn
                .query_row(
                    "SELECT id FROM api_keys WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(ApiKeyError::NotFound);
            }
        }
        Ok(())
    }

    /// Verify a candidate cleartext key against every active row.
    /// Returns the matching record's id on success and bumps
    /// `last_used_at`. Returns `Err(NotFound)` when no row matches
    /// (the middleware translates this to 401). Argon2id verify
    /// is constant-time per row, but the search itself walks the
    /// table linearly — acceptable while the active key set fits
    /// in single digits, which is the dev-stack norm. When key
    /// counts grow we add a fingerprint column + lookup index.
    pub fn verify(&self, candidate: &str) -> Result<String, ApiKeyError> {
        if !candidate.starts_with(KEY_PREFIX) {
            return Err(ApiKeyError::NotFound);
        }
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, hash, revoked_at FROM api_keys",
        )?;
        let rows: Vec<(String, String, Option<i64>)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<i64>>(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for (id, hash, revoked_at) in rows {
            let parsed = match PasswordHash::new(&hash) {
                Ok(p) => p,
                Err(_) => continue, // skip malformed rows
            };
            if Argon2::default()
                .verify_password(candidate.as_bytes(), &parsed)
                .is_ok()
            {
                if revoked_at.is_some() {
                    return Err(ApiKeyError::Revoked);
                }
                let now = Utc::now().timestamp_millis();
                conn.execute(
                    "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                    params![now, &id],
                )?;
                return Ok(id);
            }
        }
        Err(ApiKeyError::NotFound)
    }
}

/// Return value of [`ApiKeyStore::issue`]. The cleartext key is
/// only available here — once this struct goes out of scope the
/// caller has lost the key forever. The admin CLI prints
/// [`Self::cleartext`] exactly once.
#[derive(Debug)]
pub struct IssuedKey {
    pub id: String,
    pub cleartext: String,
    pub scope: String,
    pub label: String,
    pub created_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_returns_cortex_dash_prefixed_key() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "test").unwrap();
        assert!(issued.cleartext.starts_with(KEY_PREFIX));
        // 12-char prefix + 52-char base32 body (32 bytes / 5 bits).
        assert_eq!(issued.cleartext.len(), KEY_PREFIX.len() + 52);
    }

    #[test]
    fn verify_returns_id_for_a_freshly_minted_key() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "vt").unwrap();
        let id = store.verify(&issued.cleartext).expect("verify");
        assert_eq!(id, issued.id);
    }

    #[test]
    fn verify_fails_for_unknown_key() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        store.issue("dashboard", "x").unwrap();
        let bogus = format!("{}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", KEY_PREFIX);
        assert!(matches!(store.verify(&bogus).unwrap_err(), ApiKeyError::NotFound));
    }

    #[test]
    fn verify_fails_for_missing_prefix() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let err = store.verify("not-a-key").unwrap_err();
        assert!(matches!(err, ApiKeyError::NotFound));
    }

    #[test]
    fn revoke_then_verify_returns_revoked_error() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "rv").unwrap();
        store.revoke(&issued.id).unwrap();
        let err = store.verify(&issued.cleartext).unwrap_err();
        assert!(matches!(err, ApiKeyError::Revoked));
    }

    #[test]
    fn list_excludes_hash_field() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        store.issue("dashboard", "label-a").unwrap();
        store.issue("dashboard", "label-b").unwrap();
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "label-a");
        assert_eq!(rows[1].label, "label-b");
    }

    #[test]
    fn last_used_at_is_set_after_first_verify() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "lu").unwrap();
        let before = store.list().unwrap();
        assert!(before[0].last_used_at_ms.is_none());
        store.verify(&issued.cleartext).unwrap();
        let after = store.list().unwrap();
        assert!(after[0].last_used_at_ms.is_some());
    }

    #[test]
    fn revoke_unknown_id_returns_not_found() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let err = store.revoke("never-existed").unwrap_err();
        assert!(matches!(err, ApiKeyError::NotFound));
    }
}
