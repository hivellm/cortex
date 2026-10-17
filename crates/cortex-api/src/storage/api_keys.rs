//! SQLite-backed `api_keys` table — phase3 §7.2 + phase21 §4.3.
//!
//! Stores Argon2id digests of every dashboard API key issued via
//! the `cortex-api admin issue-api-key` subcommand. Cleartext keys
//! are printed once at issue time; only digests persist.
//!
//! Schema (v3 — adds a durable lookup digest for high-entropy keys):
//!
//! ```sql
//! CREATE TABLE api_keys (
//!   id            TEXT PRIMARY KEY,           -- ULID
//!   scope         TEXT NOT NULL,              -- e.g. "dashboard"
//!   label         TEXT NOT NULL,
//!   hash          TEXT NOT NULL,              -- argon2id PHC string
//!   created_at    INTEGER NOT NULL,           -- epoch ms
//!   last_used_at  INTEGER,                    -- epoch ms; NULL until first use
//!   revoked_at    INTEGER,                    -- epoch ms; NULL while active
//!   role          TEXT,                       -- RBAC role label; NULL → default principal
//!   lookup_sha256 TEXT                        -- NULL for unmigrated legacy keys
//! );
//! ```
//!
//! Schema additions are nullable: `open`/`open_in_memory` inspect existing
//! columns and add missing `role` and `lookup_sha256` columns. Keys without
//! a role resolve to the configured default principal in `PrincipalStore`.
//!
//! Keys are 32 random bytes drawn from `OsRng`, encoded with
//! Crockford base32 (lower case, no padding) and prefixed
//! `cortex_dash_`. SHA-256 selects a new key's row before Argon2id verifies
//! it. This avoids hashing unrelated credentials on a cold request. The lookup
//! digest is safe for these random 256-bit tokens and never grants authority
//! without Argon2 verification. Legacy rows are scanned only on lookup miss,
//! then gain a lookup digest after successful verification.

use std::path::Path;
use std::sync::{Arc, Mutex};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::Utc;
use data_encoding::BASE32_NOPAD;
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
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
    /// RBAC role label. `None` means "no binding → use default principal".
    pub role: Option<String>,
}

/// In-process handle around a single SQLite connection guarded by
/// a mutex. Cheap to clone — the inner `Arc<Mutex<Connection>>` is
/// shared across the daemon. Password hashing runs outside this lock; only
/// row selection and the final authority transaction hold it.
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

fn apply_migrations(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    // Serialize inspection and ALTER across independent daemon connections.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA_DDL)?;
    let mut stmt = tx.prepare("PRAGMA table_info(api_keys)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    if !columns.iter().any(|column| column == "role") {
        tx.execute("ALTER TABLE api_keys ADD COLUMN role TEXT", [])?;
    }
    if !columns.iter().any(|column| column == "lookup_sha256") {
        tx.execute("ALTER TABLE api_keys ADD COLUMN lookup_sha256 TEXT", [])?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS api_keys_lookup_sha256_idx
        ON api_keys(lookup_sha256) WHERE lookup_sha256 IS NOT NULL;",
    )?;
    tx.commit()
}

fn lookup_digest(candidate: &str) -> String {
    format!("{:x}", Sha256::digest(candidate.as_bytes()))
}

impl ApiKeyStore {
    /// Open or create the SQLite database at `path` and apply the
    /// `api_keys` schema migration. The parent directory must
    /// already exist; callers (typically `main.rs` boot path) are
    /// responsible for creating it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ApiKeyError> {
        let mut conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        apply_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory store. Used by ITs and the smoke tests in
    /// this module — boots without touching the filesystem.
    pub fn open_in_memory() -> Result<Self, ApiKeyError> {
        let mut conn = Connection::open_in_memory()?;
        apply_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Mint a new key with no role binding. See [`Self::issue_with_role`]
    /// to mint a key bound to an RBAC role at creation time.
    pub fn issue(&self, scope: &str, label: &str) -> Result<IssuedKey, ApiKeyError> {
        self.issue_with_role(scope, label, None)
    }

    /// Mint a new key bound to `role` (phase21 §4.3). The `role` label
    /// is stored verbatim; the `PrincipalStore::resolve` path resolves
    /// it to clearance + compartments at request time.
    pub fn issue_with_role(
        &self,
        scope: &str,
        label: &str,
        role: Option<&str>,
    ) -> Result<IssuedKey, ApiKeyError> {
        let mut raw = [0u8; KEY_RAW_BYTES];
        OsRng.fill_bytes(&mut raw);
        let cleartext = format!("{}{}", KEY_PREFIX, BASE32_NOPAD.encode(&raw).to_lowercase(),);
        let salt = SaltString::generate(&mut OsRng);
        let argon = Argon2::default();
        let hash = argon
            .hash_password(cleartext.as_bytes(), &salt)?
            .to_string();
        let id = Ulid::new().to_string();
        let created_at_ms = Utc::now().timestamp_millis();
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        conn.execute(
            "INSERT INTO api_keys (id, scope, label, hash, created_at, last_used_at, revoked_at, role, lookup_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?7)",
            params![&id, scope, label, &hash, created_at_ms, role, lookup_digest(&cleartext)],
        )?;
        Ok(IssuedKey {
            id,
            cleartext,
            scope: scope.to_string(),
            label: label.to_string(),
            created_at_ms,
            role: role.map(str::to_string),
        })
    }

    /// Assign (or replace) the RBAC role on an existing key.
    /// Pass `None` to clear the binding (the key reverts to the default principal).
    pub fn assign_role(&self, id: &str, role: Option<&str>) -> Result<(), ApiKeyError> {
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        let updated = conn.execute(
            "UPDATE api_keys SET role = ?1 WHERE id = ?2",
            params![role, id],
        )?;
        if updated == 0 {
            return Err(ApiKeyError::NotFound);
        }
        Ok(())
    }

    /// List every key in the table. Hashes are stripped before
    /// return so callers cannot accidentally print them.
    pub fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyError> {
        let conn = self.conn.lock().expect("api_keys mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, scope, label, created_at, last_used_at, revoked_at, role
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
                role: row.get::<_, Option<String>>(6)?,
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

    /// Select by durable lookup digest, then verify the candidate with Argon2.
    /// Returns the matching record's id on success and bumps
    /// `last_used_at`. Returns `Err(NotFound)` when no row matches
    /// (the middleware translates this to 401).
    ///
    /// To get the key's role alongside the id (for principal resolution
    /// in §4.4), use [`Self::verify_with_role`].
    pub fn verify(&self, candidate: &str) -> Result<String, ApiKeyError> {
        self.verify_with_role(candidate).map(|(id, _)| id)
    }

    /// Verify a candidate cleartext key and return `(key_id, role)`.
    ///
    /// The `role` is `None` when the key has no binding — the caller
    /// should resolve using the `PrincipalStore` default principal.
    /// Used by the auth middleware added in phase21 §4.4.
    /// Legacy keys without a lookup digest require an initial Argon2 scan;
    /// successful verification backfills their digest for subsequent calls.
    pub fn verify_with_role(
        &self,
        candidate: &str,
    ) -> Result<(String, Option<String>), ApiKeyError> {
        if !candidate.starts_with(KEY_PREFIX) {
            return Err(ApiKeyError::NotFound);
        }
        let digest = lookup_digest(candidate);
        let rows = {
            let conn = self.conn.lock().expect("api_keys mutex poisoned");
            let mut stmt =
                conn.prepare("SELECT id, hash FROM api_keys WHERE lookup_sha256 = ?1")?;
            let mut rows = stmt
                .query_map(params![&digest], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            // Do not mix legacy rows into the indexed path: even thousands of
            // old keys must not slow a newly-issued credential's first call.
            if rows.is_empty() {
                let mut legacy =
                    conn.prepare("SELECT id, hash FROM api_keys WHERE lookup_sha256 IS NULL")?;
                rows = legacy
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
            }
            rows
        };
        for (id, hash) in rows {
            let parsed = match PasswordHash::new(&hash) {
                Ok(p) => p,
                Err(_) => continue, // skip malformed rows
            };
            if Argon2::default()
                .verify_password(candidate.as_bytes(), &parsed)
                .is_ok()
            {
                // Hashing deliberately releases the connection lock. Re-read
                // current authority after it completes so role edits/revokes
                // during expensive verification apply to this request too.
                let mut conn = self.conn.lock().expect("api_keys mutex poisoned");
                let tx = conn.transaction()?;
                let current: Option<(String, Option<i64>, Option<String>)> = tx
                    .query_row(
                        "SELECT hash, revoked_at, role FROM api_keys WHERE id = ?1",
                        params![&id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let Some((current_hash, revoked_at, role)) = current else {
                    return Err(ApiKeyError::NotFound);
                };
                if current_hash != hash {
                    return Err(ApiKeyError::NotFound);
                }
                if revoked_at.is_some() {
                    return Err(ApiKeyError::Revoked);
                }
                let now = Utc::now().timestamp_millis();
                tx.execute(
                    "UPDATE api_keys SET last_used_at = ?1,
                     lookup_sha256 = COALESCE(lookup_sha256, ?3) WHERE id = ?2",
                    params![now, &id, &digest],
                )?;
                tx.commit()?;
                return Ok((id, role));
            }
        }
        Err(ApiKeyError::NotFound)
    }
}

/// Return value of [`ApiKeyStore::issue`] and [`ApiKeyStore::issue_with_role`].
/// The cleartext key is only available here — once this struct goes out of
/// scope the caller has lost the key forever. The admin CLI prints
/// [`Self::cleartext`] exactly once.
#[derive(Debug)]
pub struct IssuedKey {
    pub id: String,
    pub cleartext: String,
    pub scope: String,
    pub label: String,
    pub created_at_ms: i64,
    /// The RBAC role bound to this key at issuance (`None` = default principal).
    pub role: Option<String>,
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
        let bogus = format!(
            "{}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            KEY_PREFIX
        );
        assert!(matches!(
            store.verify(&bogus).unwrap_err(),
            ApiKeyError::NotFound
        ));
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

    // ---- phase21 §4.3: role binding tests ----

    #[test]
    fn issue_without_role_produces_none_role() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "norole").unwrap();
        assert!(issued.role.is_none());
        let rows = store.list().unwrap();
        assert!(rows[0].role.is_none());
    }

    #[test]
    fn issue_with_role_stores_the_role() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store
            .issue_with_role("dashboard", "analyst", Some("analyst"))
            .unwrap();
        assert_eq!(issued.role.as_deref(), Some("analyst"));
        let rows = store.list().unwrap();
        assert_eq!(rows[0].role.as_deref(), Some("analyst"));
    }

    #[test]
    fn verify_with_role_returns_role_alongside_id() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store
            .issue_with_role("dashboard", "analyst", Some("analyst"))
            .unwrap();
        let (id, role) = store.verify_with_role(&issued.cleartext).unwrap();
        assert_eq!(id, issued.id);
        assert_eq!(role.as_deref(), Some("analyst"));
    }

    #[test]
    fn verify_with_role_returns_none_role_for_unbound_key() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "unbound").unwrap();
        let (id, role) = store.verify_with_role(&issued.cleartext).unwrap();
        assert_eq!(id, issued.id);
        assert!(role.is_none());
    }

    #[test]
    fn assign_role_updates_an_existing_key() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store.issue("dashboard", "upgrader").unwrap();
        assert!(issued.role.is_none());
        store.assign_role(&issued.id, Some("security_eng")).unwrap();
        let rows = store.list().unwrap();
        assert_eq!(rows[0].role.as_deref(), Some("security_eng"));
    }

    #[test]
    fn assign_role_none_clears_the_binding() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store
            .issue_with_role("dashboard", "clearme", Some("analyst"))
            .unwrap();
        store.assign_role(&issued.id, None).unwrap();
        let rows = store.list().unwrap();
        assert!(rows[0].role.is_none());
    }

    #[test]
    fn assign_role_not_found_returns_not_found() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let err = store
            .assign_role("00000000000000000000000000", Some("r"))
            .unwrap_err();
        assert!(matches!(err, ApiKeyError::NotFound));
    }

    #[test]
    fn verify_with_role_errors_propagate_correctly_when_revoked() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store
            .issue_with_role("dashboard", "torevoke", Some("analyst"))
            .unwrap();
        store.revoke(&issued.id).unwrap();
        let err = store.verify_with_role(&issued.cleartext).unwrap_err();
        assert!(matches!(err, ApiKeyError::Revoked));
    }

    #[test]
    fn durable_lookup_survives_restart_and_uses_fresh_authority_from_second_connection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api-keys.sqlite");
        let store = ApiKeyStore::open(&path).unwrap();
        let legacy_decoy = store.issue("api", "legacy-decoy").unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE api_keys SET lookup_sha256 = NULL WHERE id = ?1",
                params![&legacy_decoy.id],
            )
            .unwrap();
        store.issue("api", "new-decoy-one").unwrap();
        store.issue("api", "new-decoy-two").unwrap();
        let issued = store
            .issue_with_role("api", "target", Some("reader"))
            .unwrap();
        drop(store);

        // First verification after reopening is cold: there is no in-memory
        // verification cache, and unrelated legacy hashes must not be scanned.
        let restarted = ApiKeyStore::open(&path).unwrap();
        assert_eq!(
            restarted.verify_with_role(&issued.cleartext).unwrap(),
            (issued.id.clone(), Some("reader".into()))
        );
        let other = ApiKeyStore::open(&path).unwrap();
        other.assign_role(&issued.id, Some("analyst")).unwrap();
        assert_eq!(
            restarted.verify_with_role(&issued.cleartext).unwrap(),
            (issued.id.clone(), Some("analyst".into()))
        );
        other.assign_role(&issued.id, None).unwrap();
        assert_eq!(
            restarted.verify_with_role(&issued.cleartext).unwrap(),
            (issued.id.clone(), None)
        );
        other.revoke(&issued.id).unwrap();
        assert!(matches!(
            restarted.verify(&issued.cleartext),
            Err(ApiKeyError::Revoked)
        ));
    }

    #[test]
    fn old_schema_migrates_and_successful_legacy_verification_backfills_lookup() {
        let source = ApiKeyStore::open_in_memory().unwrap();
        let issued = source.issue("api", "legacy").unwrap();
        let hash: String = source
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT hash FROM api_keys WHERE id = ?1",
                params![&issued.id],
                |row| row.get(0),
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.sqlite");
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(SCHEMA_DDL).unwrap();
            old.execute(
                "INSERT INTO api_keys(id, scope, label, hash, created_at)
                VALUES (?1, 'api', 'legacy', ?2, 0)",
                params![&issued.id, hash],
            )
            .unwrap();
        }
        let upgraded = ApiKeyStore::open(&path).unwrap();
        let before: Option<String> = upgraded
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT lookup_sha256 FROM api_keys WHERE id = ?1",
                params![&issued.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(before.is_none());
        assert_eq!(upgraded.verify(&issued.cleartext).unwrap(), issued.id);
        let after: String = upgraded
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT lookup_sha256 FROM api_keys WHERE id = ?1",
                params![&issued.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, lookup_digest(&issued.cleartext));
        drop(upgraded);
        let restarted = ApiKeyStore::open(&path).unwrap();
        let other = ApiKeyStore::open(&path).unwrap();
        other.assign_role(&issued.id, Some("reader")).unwrap();
        assert_eq!(
            restarted.verify_with_role(&issued.cleartext).unwrap().1,
            Some("reader".into())
        );
        other.revoke(&issued.id).unwrap();
        assert!(matches!(
            restarted.verify(&issued.cleartext),
            Err(ApiKeyError::Revoked)
        ));
    }

    #[test]
    fn lookup_digest_alone_never_authenticates_a_wrong_credential() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let issued = store
            .issue_with_role("api", "privileged", Some("acl_admin"))
            .unwrap();
        let wrong = format!("{KEY_PREFIX}{}", "a".repeat(52));
        // Even corrupted lookup metadata selecting a privileged row cannot
        // replace the final Argon2 proof of the actual credential.
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE api_keys SET lookup_sha256 = ?1 WHERE id = ?2",
                params![lookup_digest(&wrong), &issued.id],
            )
            .unwrap();
        assert!(matches!(
            store.verify_with_role(&wrong),
            Err(ApiKeyError::NotFound)
        ));
        assert!(store.list().unwrap()[0].last_used_at_ms.is_none());
    }

    #[test]
    fn concurrent_legacy_schema_migrations_both_succeed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy-concurrent.sqlite");
        {
            let legacy = Connection::open(&path).unwrap();
            legacy.pragma_update(None, "journal_mode", "WAL").unwrap();
            legacy.execute_batch(SCHEMA_DDL).unwrap();
        }
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let store = ApiKeyStore::open(path)?;
                    store.list()
                })
            })
            .collect();
        for worker in workers {
            assert!(worker.join().unwrap().unwrap().is_empty());
        }
    }
}
