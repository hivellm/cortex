//! SQLite metadata store.
//!
//! Contains operational rows that don't fit a graph or a vector store:
//! session lifecycle, repo registry, bootstrap job progress, classifier
//! spend, law registry mirror, trust scores, retention sweeps, API keys.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Errors returned by [`MetadataStore`].
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    /// SQLite driver error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// I/O error around the database file.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Generic internal error.
    #[error("internal: {0}")]
    Internal(String),
}

/// Embedded DDL applied by [`MetadataStore::open`] at every startup.
pub const SCHEMA_SQL: &str = include_str!("../schemas/sqlite/schema.sql");

/// Schema version recorded in the `meta` table.
pub const SCHEMA_VERSION: u32 = 1;

/// Thin wrapper around [`rusqlite::Connection`] that guarantees the schema
/// is present + versioned.
pub struct MetadataStore {
    conn: Connection,
}

impl MetadataStore {
    /// Open or create the database at `path`, applying migrations.
    pub fn open(path: &Path) -> Result<Self, MetadataError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(MetadataStore { conn })
    }

    /// Open an in-memory database (test-only convenience).
    pub fn open_in_memory() -> Result<Self, MetadataError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(MetadataStore { conn })
    }

    fn configure(conn: &Connection) -> rusqlite::Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(())
    }

    fn migrate(conn: &Connection) -> Result<(), MetadataError> {
        conn.execute_batch(SCHEMA_SQL)?;
        // Record or assert schema version.
        let existing: Option<u32> = conn
            .query_row("SELECT version FROM meta WHERE key = 'schema'", [], |r| r.get(0))
            .optional()?;
        match existing {
            Some(v) if v == SCHEMA_VERSION => Ok(()),
            Some(v) => Err(MetadataError::Internal(format!(
                "incompatible schema version: db={v}, expected={SCHEMA_VERSION}"
            ))),
            None => {
                conn.execute(
                    "INSERT INTO meta (key, version, updated_at) VALUES ('schema', ?1, ?2)",
                    params![SCHEMA_VERSION, Utc::now().to_rfc3339()],
                )?;
                Ok(())
            }
        }
    }

    /// Access the underlying connection for custom queries.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Access the underlying connection mutably (transactions, batch ops).
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    // ---------- session convenience helpers ----------

    /// Insert or update a session row.
    pub fn upsert_session(
        &self,
        session_id: &str,
        tool: &str,
        model: Option<&str>,
        repo: Option<&str>,
        user: Option<&str>,
        started_at: DateTime<Utc>,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO sessions (session_id, tool, model, repo, user, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               tool=excluded.tool,
               model=COALESCE(excluded.model, sessions.model),
               repo=COALESCE(excluded.repo, sessions.repo),
               user=COALESCE(excluded.user, sessions.user)",
            params![
                session_id,
                tool,
                model,
                repo,
                user,
                started_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// Mark a session ended.
    pub fn close_session(
        &self,
        session_id: &str,
        ended_at: DateTime<Utc>,
        event_count: u64,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1, event_count = ?2 WHERE session_id = ?3",
            params![ended_at.to_rfc3339(), event_count as i64, session_id],
        )?;
        Ok(())
    }

    /// Record a day of classifier spend; idempotent upsert.
    pub fn record_classifier_spend(
        &self,
        day: &str,
        calls: u64,
        tokens_in: u64,
        tokens_out: u64,
        est_usd_cents: u64,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO classifier_spend (day, calls, tokens_in, tokens_out, est_usd_cents)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(day) DO UPDATE SET
               calls = calls + excluded.calls,
               tokens_in = tokens_in + excluded.tokens_in,
               tokens_out = tokens_out + excluded.tokens_out,
               est_usd_cents = est_usd_cents + excluded.est_usd_cents",
            params![
                day,
                calls as i64,
                tokens_in as i64,
                tokens_out as i64,
                est_usd_cents as i64
            ],
        )?;
        Ok(())
    }

    /// Read back a day's classifier spend.
    pub fn classifier_spend(&self, day: &str) -> Result<Option<ClassifierSpend>, MetadataError> {
        let row = self
            .conn
            .query_row(
                "SELECT calls, tokens_in, tokens_out, est_usd_cents FROM classifier_spend WHERE day = ?1",
                params![day],
                |r| {
                    Ok(ClassifierSpend {
                        calls: r.get::<_, i64>(0)? as u64,
                        tokens_in: r.get::<_, i64>(1)? as u64,
                        tokens_out: r.get::<_, i64>(2)? as u64,
                        est_usd_cents: r.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

/// Row from the `classifier_spend` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierSpend {
    /// Number of classifier calls.
    pub calls: u64,
    /// Tokens-in accumulated.
    pub tokens_in: u64,
    /// Tokens-out accumulated.
    pub tokens_out: u64,
    /// Running spend estimate in US cents.
    pub est_usd_cents: u64,
}

