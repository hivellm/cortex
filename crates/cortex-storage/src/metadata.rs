//! SQLite metadata store.
//!
//! Contains operational rows that don't fit a graph or a vector store:
//! session lifecycle, repo registry, bootstrap job progress, classifier
//! spend, law registry mirror, trust scores, retention sweeps, API keys.

use chrono::{DateTime, Timelike, Utc};
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
        // Phase9a — idempotent column-add for `retention_sweeps.status`.
        // Existing pre-phase9a databases lack the column; the
        // `CREATE TABLE IF NOT EXISTS` above is a no-op for them, so
        // we probe `pragma_table_info` and ALTER if needed.
        let has_status: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('retention_sweeps') WHERE name = 'status'")?
            .query_row([], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if !has_status {
            conn.execute(
                "ALTER TABLE retention_sweeps ADD COLUMN status TEXT NOT NULL DEFAULT 'success'",
                [],
            )?;
        }
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

    // ---------- phase9a retention sweep helpers ----------

    /// Phase9a — start a retention sweep row in `status='running'`.
    /// Returns `Err(SweepLockBusy)` when another sweep is already
    /// running and its `started_at` is younger than the abandon
    /// grace window.
    pub fn start_retention_sweep(
        &self,
        sweep_id: &str,
        started_at: DateTime<Utc>,
        abandon_grace_secs: i64,
    ) -> Result<(), MetadataError> {
        // Sweep abandonment: a previous run that died mid-flight
        // leaves a `status='running'` row. If its `started_at` is
        // older than the grace window, mark it `abandoned` and
        // proceed with our own row.
        let grace_cutoff = started_at - chrono::Duration::seconds(abandon_grace_secs);
        self.conn.execute(
            "UPDATE retention_sweeps
                SET status = 'abandoned', finished_at = ?1
              WHERE status = 'running' AND started_at < ?2",
            params![started_at.to_rfc3339(), grace_cutoff.to_rfc3339()],
        )?;
        // Concurrency check: any remaining `running` row blocks us.
        let busy: bool = self
            .conn
            .prepare("SELECT 1 FROM retention_sweeps WHERE status = 'running' LIMIT 1")?
            .query_row([], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if busy {
            return Err(MetadataError::Internal(
                "another retention sweep is in progress".into(),
            ));
        }
        self.conn.execute(
            "INSERT INTO retention_sweeps (sweep_id, started_at, status)
             VALUES (?1, ?2, 'running')",
            params![sweep_id, started_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Phase9a — close a running sweep row with the final counters.
    /// `status` should be `"success"` for a clean run, `"failed"` for
    /// an error path that still completed bookkeeping.
    pub fn finish_retention_sweep(
        &self,
        sweep_id: &str,
        finished_at: DateTime<Utc>,
        records_demoted: u64,
        records_dropped: u64,
        tier_transitions_json: &str,
        status: &str,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "UPDATE retention_sweeps SET
                finished_at = ?1,
                records_demoted = ?2,
                records_dropped = ?3,
                tier_transitions_json = ?4,
                status = ?5
             WHERE sweep_id = ?6",
            params![
                finished_at.to_rfc3339(),
                records_demoted as i64,
                records_dropped as i64,
                tier_transitions_json,
                status,
                sweep_id,
            ],
        )?;
        Ok(())
    }

    /// Phase9a — list the most recent `limit` sweeps, newest first.
    /// Returns `(sweep_id, started_at, finished_at, records_demoted,
    /// records_dropped, status)` tuples for the dashboard's
    /// retention view (phase9i).
    pub fn list_recent_sweeps(
        &self,
        limit: usize,
    ) -> Result<Vec<RetentionSweepRow>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT sweep_id, started_at, finished_at, records_demoted,
                    records_dropped, status, tier_transitions_json
               FROM retention_sweeps
              ORDER BY started_at DESC
              LIMIT ?1",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(params![limit as i64], |row| {
                Ok(RetentionSweepRow {
                    sweep_id: row.get(0)?,
                    started_at: row.get(1)?,
                    finished_at: row.get(2)?,
                    records_demoted: row.get::<_, i64>(3)? as u64,
                    records_dropped: row.get::<_, i64>(4)? as u64,
                    status: row.get(5)?,
                    tier_transitions_json: row.get(6)?,
                })
            })?
            .collect();
        Ok(rows?)
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

    /// Record an hour of classifier spend; idempotent additive upsert.
    /// `hour` MUST be the canonical RFC3339 truncated-to-hour form
    /// (`YYYY-MM-DDTHH:00:00Z`); use [`hour_bucket_rfc3339`] to derive
    /// it from a wall-clock timestamp.
    pub fn record_classifier_spend_hourly(
        &self,
        hour: &str,
        calls: u64,
        tokens_in: u64,
        tokens_out: u64,
        est_usd_cents: u64,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO classifier_spend_hourly (hour, calls, tokens_in, tokens_out, est_usd_cents)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(hour) DO UPDATE SET
               calls = calls + excluded.calls,
               tokens_in = tokens_in + excluded.tokens_in,
               tokens_out = tokens_out + excluded.tokens_out,
               est_usd_cents = est_usd_cents + excluded.est_usd_cents",
            params![
                hour,
                calls as i64,
                tokens_in as i64,
                tokens_out as i64,
                est_usd_cents as i64
            ],
        )?;
        Ok(())
    }

    /// Read every hourly bucket whose key falls inside `[since_hour,
    /// until_hour]` (both inclusive). The result is sorted ascending
    /// by `hour` so the dashboard renderer can index oldest-first.
    pub fn classifier_spend_hourly_window(
        &self,
        since_hour: &str,
        until_hour: &str,
    ) -> Result<Vec<HourlySpendRow>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT hour, calls, tokens_in, tokens_out, est_usd_cents
               FROM classifier_spend_hourly
              WHERE hour >= ?1 AND hour <= ?2
              ORDER BY hour ASC",
        )?;
        let rows = stmt
            .query_map(params![since_hour, until_hour], |r| {
                Ok(HourlySpendRow {
                    hour: r.get::<_, String>(0)?,
                    spend: ClassifierSpend {
                        calls: r.get::<_, i64>(1)? as u64,
                        tokens_in: r.get::<_, i64>(2)? as u64,
                        tokens_out: r.get::<_, i64>(3)? as u64,
                        est_usd_cents: r.get::<_, i64>(4)? as u64,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// Truncate `ts` to the nearest UTC hour, returning the canonical
/// `YYYY-MM-DDTHH:00:00Z` key the `classifier_spend_hourly` table
/// uses. Centralised so the writer and the reader can't drift.
pub fn hour_bucket_rfc3339(ts: DateTime<Utc>) -> String {
    let truncated = ts
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(ts);
    truncated.format("%Y-%m-%dT%H:00:00Z").to_string()
}

/// Phase9a — one row from `retention_sweeps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionSweepRow {
    /// Sweep ULID.
    pub sweep_id: String,
    /// RFC-3339 start timestamp.
    pub started_at: String,
    /// RFC-3339 finish timestamp; `None` for in-flight rows.
    pub finished_at: Option<String>,
    /// Total records demoted across all tier pairs.
    pub records_demoted: u64,
    /// Records that failed re-encode and were dropped from the source
    /// without a destination write.
    pub records_dropped: u64,
    /// Phase9a status — `running` / `success` / `failed` / `abandoned`.
    pub status: String,
    /// JSON-encoded per-pair transition counts (`{"turn:fp32->pq": 12,
    /// "tool_call:pq->binary": 0, ...}`).
    pub tier_transitions_json: Option<String>,
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

/// One row of the hourly classifier spend table — the canonical
/// `hour` bucket key plus the rolled-up [`ClassifierSpend`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlySpendRow {
    /// `YYYY-MM-DDTHH:00:00Z` UTC hour bucket.
    pub hour: String,
    /// Accumulated calls / tokens / cents.
    pub spend: ClassifierSpend,
}



#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn start_retention_sweep_inserts_running_row() {
        let store = MetadataStore::open_in_memory().unwrap();
        store.start_retention_sweep("01ABCD", now(), 3600).unwrap();
        let rows = store.list_recent_sweeps(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sweep_id, "01ABCD");
        assert_eq!(rows[0].status, "running");
        assert!(rows[0].finished_at.is_none());
    }

    #[test]
    fn second_concurrent_sweep_returns_busy_error() {
        let store = MetadataStore::open_in_memory().unwrap();
        store.start_retention_sweep("01FIRST", now(), 3600).unwrap();
        let err = store
            .start_retention_sweep("01SECOND", now(), 3600)
            .unwrap_err();
        assert!(format!("{err}").contains("in progress"));
    }

    #[test]
    fn abandoned_orphan_unblocks_next_sweep() {
        let store = MetadataStore::open_in_memory().unwrap();
        // Insert an orphan whose started_at is 2 h in the past.
        let orphan_started = now() - chrono::Duration::hours(2);
        store
            .start_retention_sweep("01ORPHAN", orphan_started, 3600)
            .unwrap();
        // Subsequent sweep at `now()` finds the orphan older than
        // the 1-h grace window and proceeds.
        store.start_retention_sweep("01NEW", now(), 3600).unwrap();
        let rows = store.list_recent_sweeps(10).unwrap();
        let orphan = rows
            .iter()
            .find(|r| r.sweep_id == "01ORPHAN")
            .expect("orphan row");
        assert_eq!(orphan.status, "abandoned");
        assert!(orphan.finished_at.is_some());
        let new_row = rows
            .iter()
            .find(|r| r.sweep_id == "01NEW")
            .expect("new row");
        assert_eq!(new_row.status, "running");
    }

    #[test]
    fn finish_retention_sweep_updates_counters_and_status() {
        let store = MetadataStore::open_in_memory().unwrap();
        store.start_retention_sweep("01DONE", now(), 3600).unwrap();
        let finished = now() + chrono::Duration::seconds(30);
        store
            .finish_retention_sweep(
                "01DONE",
                finished,
                42,
                3,
                r#"{"turn:fp32->pq":42}"#,
                "success",
            )
            .unwrap();
        let rows = store.list_recent_sweeps(10).unwrap();
        let row = rows
            .iter()
            .find(|r| r.sweep_id == "01DONE")
            .expect("done row");
        assert_eq!(row.status, "success");
        assert_eq!(row.records_demoted, 42);
        assert_eq!(row.records_dropped, 3);
        assert_eq!(
            row.tier_transitions_json.as_deref(),
            Some(r#"{"turn:fp32->pq":42}"#)
        );
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn list_recent_sweeps_orders_newest_first() {
        let store = MetadataStore::open_in_memory().unwrap();
        // Insert + finish three rows out of chronological order.
        let ts1 = DateTime::parse_from_rfc3339("2026-04-27T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts2 = DateTime::parse_from_rfc3339("2026-04-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts3 = DateTime::parse_from_rfc3339("2026-04-28T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.start_retention_sweep("01OLD", ts1, 3600).unwrap();
        store
            .finish_retention_sweep("01OLD", ts1, 0, 0, "{}", "success")
            .unwrap();
        store.start_retention_sweep("01NEW", ts2, 3600).unwrap();
        store
            .finish_retention_sweep("01NEW", ts2, 0, 0, "{}", "success")
            .unwrap();
        store.start_retention_sweep("01MID", ts3, 3600).unwrap();
        store
            .finish_retention_sweep("01MID", ts3, 0, 0, "{}", "success")
            .unwrap();
        let rows = store.list_recent_sweeps(3).unwrap();
        assert_eq!(rows[0].sweep_id, "01NEW");
        assert_eq!(rows[1].sweep_id, "01MID");
        assert_eq!(rows[2].sweep_id, "01OLD");
    }

    #[test]
    fn list_recent_sweeps_honours_limit() {
        let store = MetadataStore::open_in_memory().unwrap();
        for i in 0..5 {
            let id = format!("01{i:024}");
            store.start_retention_sweep(&id, now(), 0).unwrap();
            store
                .finish_retention_sweep(&id, now(), 0, 0, "{}", "success")
                .unwrap();
        }
        assert_eq!(store.list_recent_sweeps(2).unwrap().len(), 2);
        assert_eq!(store.list_recent_sweeps(100).unwrap().len(), 5);
    }
}
