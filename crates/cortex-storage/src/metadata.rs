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
        // Phase13d — assert the event_identity table exists. ADR-012
        // owns the schema in `crate::identity`; calling the helper
        // here at MetadataStore open time means every consumer that
        // grabs `metadata.conn()` to build an `IdentityIndex` sees
        // the migrated table without coordinating its own migration.
        crate::identity::apply_phase13d_schema(conn)
            .map_err(|e| MetadataError::Internal(format!("phase13d schema: {e}")))?;
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
            .query_row("SELECT version FROM meta WHERE key = 'schema'", [], |r| {
                r.get(0)
            })
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
    ///
    /// phase10i — when an upsert from a hook that didn't capture
    /// the tool name lands (the IPC peer reported `""`), the
    /// existing `tool` value is preserved instead of being
    /// overwritten with NULL. The 2026-04-29 audit caught 574
    /// sessions in the `tool IS NULL` state because the
    /// pre-phase10i upsert wrote `tool=excluded.tool`
    /// unconditionally, and `Stop` / lifecycle hooks frequently
    /// passed an empty `tool` string. `model` / `repo` / `user`
    /// already preserved their existing value via `COALESCE`;
    /// `tool` now matches.
    pub fn upsert_session(
        &self,
        session_id: &str,
        tool: &str,
        model: Option<&str>,
        repo: Option<&str>,
        user: Option<&str>,
        started_at: DateTime<Utc>,
    ) -> Result<(), MetadataError> {
        // The schema declares `sessions.tool TEXT NOT NULL`, so
        // first-time writes preserve the literal even when it's
        // an empty string. The COALESCE / NULLIF dance fires
        // on conflict only: a subsequent write that passes
        // empty falls back to the existing value rather than
        // overwriting it with `''`.
        self.conn.execute(
            "INSERT INTO sessions (session_id, tool, model, repo, user, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
               tool=COALESCE(NULLIF(excluded.tool, ''), sessions.tool),
               model=COALESCE(excluded.model, sessions.model),
               repo=COALESCE(excluded.repo, sessions.repo),
               user=COALESCE(excluded.user, sessions.user)",
            params![session_id, tool, model, repo, user, started_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// phase10i — explicit setter for `sessions.tool`. Used by
    /// the `cortex-ops sessions backfill-tool` migration to
    /// stamp a tool on rows the pre-phase10i upsert NULL-ed out.
    /// Returns the rowcount touched so the caller can build a
    /// dry-run report. `tool` is canonicalised to its trimmed,
    /// non-empty form; passing `""` is a no-op.
    pub fn set_session_tool(&self, session_id: &str, tool: &str) -> Result<u64, MetadataError> {
        let trimmed = tool.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }
        let touched = self.conn.execute(
            "UPDATE sessions SET tool = ?1 WHERE session_id = ?2",
            params![trimmed, session_id],
        )?;
        Ok(touched as u64)
    }

    /// phase10i — list `(session_id, started_at)` for sessions
    /// whose tool is currently NULL or empty. The backfill CLI
    /// calls this once and then iterates to stamp each row.
    pub fn sessions_missing_tool(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String)>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, started_at FROM sessions
             WHERE tool IS NULL OR tool = ''
             ORDER BY started_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
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

    /// phase11v §6.2 — record a single completed sweep row in one
    /// statement. Bypasses the `start_retention_sweep` /
    /// `finish_retention_sweep` two-step that the legacy
    /// `retention_sweep` and `rollup` handlers use because the
    /// per-sweep lock those provide is already held by the
    /// `cortex-workers::retention::scheduler` per-job semaphore at a
    /// higher layer. New sweep callers (every `cortex-ops <sweep>`
    /// handler that did not previously bookkeep) call this helper at
    /// the end of their successful or failed run so the
    /// `retention_sweeps` table — and the dashboard's
    /// "Bytes reclaimed last 30 d" panel — finally reflect every
    /// invocation, not just the two legacy ones.
    #[allow(clippy::too_many_arguments)]
    pub fn note_completed_retention_sweep(
        &self,
        sweep_id: &str,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
        records_demoted: u64,
        records_dropped: u64,
        tier_transitions_json: &str,
        status: &str,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO retention_sweeps
                 (sweep_id, started_at, finished_at, records_demoted,
                  records_dropped, tier_transitions_json, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                sweep_id,
                started_at.to_rfc3339(),
                finished_at.to_rfc3339(),
                records_demoted as i64,
                records_dropped as i64,
                tier_transitions_json,
                status,
            ],
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

    /// Phase13b — append one row to `producer_checkpoints`. ADR-010
    /// requires the table be append-only so a kill-resume cycle
    /// never loses the prior cursor.
    pub fn record_producer_checkpoint(
        &self,
        producer_name: &str,
        scope: &str,
        last_event_id: &str,
        last_occurred_at: DateTime<Utc>,
        accumulated_at: DateTime<Utc>,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO producer_checkpoints
                (producer_name, scope, last_event_id, last_occurred_at, accumulated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                producer_name,
                scope,
                last_event_id,
                last_occurred_at.to_rfc3339(),
                accumulated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Phase13b — return the most recent producer-checkpoint row
    /// for the supplied `(producer_name, scope)`. The producer
    /// reads this on resume. `None` when no checkpoint has been
    /// written yet (fresh-corpus walk).
    pub fn latest_producer_checkpoint(
        &self,
        producer_name: &str,
        scope: &str,
    ) -> Result<Option<ProducerCheckpointRow>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT producer_name, scope, last_event_id, last_occurred_at, accumulated_at
               FROM producer_checkpoints
              WHERE producer_name = ?1 AND scope = ?2
              ORDER BY accumulated_at DESC
              LIMIT 1",
        )?;
        let mut rows = stmt.query(params![producer_name, scope])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ProducerCheckpointRow {
                producer_name: row.get(0)?,
                scope: row.get(1)?,
                last_event_id: row.get(2)?,
                last_occurred_at: row.get(3)?,
                accumulated_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Phase13b — list every checkpoint row for one producer
    /// across all scopes, newest first. Used by the dashboard's
    /// per-producer audit view + the resume-after-kill IT to
    /// verify the accumulation contract.
    pub fn list_producer_checkpoints_for(
        &self,
        producer_name: &str,
        limit: usize,
    ) -> Result<Vec<ProducerCheckpointRow>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT producer_name, scope, last_event_id, last_occurred_at, accumulated_at
               FROM producer_checkpoints
              WHERE producer_name = ?1
              ORDER BY accumulated_at DESC
              LIMIT ?2",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map(params![producer_name, limit as i64], |row| {
                Ok(ProducerCheckpointRow {
                    producer_name: row.get(0)?,
                    scope: row.get(1)?,
                    last_event_id: row.get(2)?,
                    last_occurred_at: row.get(3)?,
                    accumulated_at: row.get(4)?,
                })
            })?
            .collect();
        Ok(rows?)
    }

    /// Phase13f §3.4 — return the most recent checkpoint row for
    /// every `(producer_name, scope)` pair, sorted by
    /// `producer_name`, `scope`. The dashboard's
    /// `/v1/dashboard/producers` endpoint reads this to render one
    /// row per producer / scope pair without per-row queries.
    pub fn latest_producer_checkpoint_per_scope(
        &self,
    ) -> Result<Vec<ProducerCheckpointRow>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT producer_name, scope, last_event_id, last_occurred_at, accumulated_at
               FROM producer_checkpoints AS pc
              WHERE accumulated_at = (
                  SELECT MAX(accumulated_at)
                    FROM producer_checkpoints
                   WHERE producer_name = pc.producer_name
                     AND scope = pc.scope
              )
              ORDER BY producer_name ASC, scope ASC",
        )?;
        let rows: Result<Vec<_>, _> = stmt
            .query_map([], |row| {
                Ok(ProducerCheckpointRow {
                    producer_name: row.get(0)?,
                    scope: row.get(1)?,
                    last_event_id: row.get(2)?,
                    last_occurred_at: row.get(3)?,
                    accumulated_at: row.get(4)?,
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

    // ---------- phase9k cron scheduler helpers ----------

    /// Insert a cron-job row if `name` is not already present.
    /// Returns `true` when a new row was inserted, `false` when the
    /// row already existed (used by the daemon to seed defaults
    /// idempotently).
    pub fn upsert_cron_job_if_absent(
        &self,
        name: &str,
        schedule: &str,
        command: &str,
        enabled: bool,
        next_run_at: &str,
    ) -> Result<bool, MetadataError> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO cron_jobs (name, schedule, command, enabled, next_run_at)
                  VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                name,
                schedule,
                command,
                if enabled { 1_i64 } else { 0_i64 },
                next_run_at
            ],
        )?;
        Ok(n > 0)
    }

    /// phase11v §3.1 — reconcile a default-driven drift on an
    /// existing row. Updates `enabled` and `command` to the values
    /// the latest seed call carries, leaves every other column
    /// untouched (the operator-tuned `schedule`, every run-history
    /// column, `failure_streak`, `last_warning_at`).
    pub fn update_cron_job_default_state(
        &self,
        name: &str,
        enabled: bool,
        command: &str,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "UPDATE cron_jobs SET enabled = ?1, command = ?2 WHERE name = ?3",
            params![if enabled { 1_i64 } else { 0_i64 }, command, name],
        )?;
        Ok(())
    }

    /// List every cron job, sorted ascending by `name`.
    pub fn list_cron_jobs(&self) -> Result<Vec<CronJob>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, schedule, command, enabled, last_run_at,
                    last_status, next_run_at, last_error, last_stdout,
                    last_stderr, failure_streak, last_warning_at
               FROM cron_jobs ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], cron_job_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Look up one cron job by name.
    pub fn get_cron_job(&self, name: &str) -> Result<Option<CronJob>, MetadataError> {
        let row = self
            .conn
            .query_row(
                "SELECT name, schedule, command, enabled, last_run_at,
                        last_status, next_run_at, last_error, last_stdout,
                        last_stderr, failure_streak, last_warning_at
                   FROM cron_jobs WHERE name = ?1",
                params![name],
                cron_job_from_row,
            )
            .optional()?;
        Ok(row)
    }

    /// Toggle the `enabled` flag. Returns the row count affected
    /// (0 when `name` is unknown).
    pub fn set_cron_job_enabled(&self, name: &str, enabled: bool) -> Result<usize, MetadataError> {
        let n = self.conn.execute(
            "UPDATE cron_jobs SET enabled = ?1 WHERE name = ?2",
            params![if enabled { 1_i64 } else { 0_i64 }, name],
        )?;
        Ok(n)
    }

    /// Update `schedule` and `next_run_at` for a job. Returns the
    /// row count affected.
    pub fn set_cron_job_schedule(
        &self,
        name: &str,
        schedule: &str,
        next_run_at: &str,
    ) -> Result<usize, MetadataError> {
        let n = self.conn.execute(
            "UPDATE cron_jobs SET schedule = ?1, next_run_at = ?2 WHERE name = ?3",
            params![schedule, next_run_at, name],
        )?;
        Ok(n)
    }

    /// Records the outcome of a run. `stdout_tail` and `stderr_tail`
    /// should already be capped at 64 KB by the caller.
    /// `failure_streak` is incremented on `failed` and reset to 0
    /// on `success`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_cron_run(
        &self,
        name: &str,
        last_run_at: DateTime<Utc>,
        status: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
        last_error: Option<&str>,
        next_run_at: &str,
    ) -> Result<u32, MetadataError> {
        // Phase12f §1 — single atomic UPDATE with CASE so concurrent
        // writers cannot race on a read-modify-write of
        // `failure_streak`. Two cron supervisors hitting the same job
        // (multi-replica deploy or restart-overlap) used to read the
        // same streak, both compute streak+1, both UPDATE — net
        // effect: counter advances by 1 instead of 2. Collapsing to
        // one UPDATE delegates serialisation to SQLite's write lock
        // (WAL mode with the workspace's `busy_timeout`); RETURNING
        // hands back the post-update value in the same statement so
        // callers see the authoritative streak.
        let new_streak: i64 = self.conn.query_row(
            "UPDATE cron_jobs SET
                last_run_at    = ?1,
                last_status    = ?2,
                last_stdout    = ?3,
                last_stderr    = ?4,
                last_error     = ?5,
                next_run_at    = ?6,
                failure_streak = CASE WHEN ?2 = 'failed'
                                      THEN failure_streak + 1
                                      ELSE 0 END
              WHERE name = ?7
              RETURNING failure_streak",
            params![
                last_run_at.to_rfc3339(),
                status,
                stdout_tail,
                stderr_tail,
                last_error,
                next_run_at,
                name,
            ],
            |r| r.get(0),
        )?;
        Ok(new_streak.max(0) as u32)
    }

    /// Stamp `last_warning_at` so the repeated-failure event is
    /// deduped within a 24-hour window.
    pub fn touch_cron_warning(
        &self,
        name: &str,
        ts: DateTime<Utc>,
    ) -> Result<usize, MetadataError> {
        let n = self.conn.execute(
            "UPDATE cron_jobs SET last_warning_at = ?1 WHERE name = ?2",
            params![ts.to_rfc3339(), name],
        )?;
        Ok(n)
    }

    /// Return every job whose `enabled=1 AND next_run_at <= now`.
    pub fn select_due_cron_jobs(&self, now: DateTime<Utc>) -> Result<Vec<CronJob>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, schedule, command, enabled, last_run_at,
                    last_status, next_run_at, last_error, last_stdout,
                    last_stderr, failure_streak, last_warning_at
               FROM cron_jobs
              WHERE enabled = 1
                AND next_run_at IS NOT NULL
                AND next_run_at <= ?1
              ORDER BY next_run_at ASC, name ASC",
        )?;
        let rows = stmt
            .query_map(params![now.to_rfc3339()], cron_job_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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

    // ---------- phase11s consumer-offset helpers ----------

    /// Phase11s §2.3 — look up the persisted offset for `(consumer_id,
    /// stream)`. Returns `None` when the consumer has never advanced
    /// (boot defaults to offset 0 in that case so the worker walks
    /// the whole stream from the beginning).
    pub fn consumer_offset_lookup(
        &self,
        consumer_id: &str,
        stream: &str,
    ) -> Result<Option<ConsumerOffsetRow>, MetadataError> {
        let row = self
            .conn
            .query_row(
                "SELECT last_offset, last_event_id, updated_at FROM consumer_offsets
                 WHERE consumer_id = ?1 AND stream = ?2",
                params![consumer_id, stream],
                |r| {
                    Ok(ConsumerOffsetRow {
                        last_offset: r.get::<_, i64>(0)?.max(0) as u64,
                        last_event_id: r.get::<_, Option<String>>(1)?,
                        updated_at: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Phase11s §2.3 — upsert the persisted offset for `(consumer_id,
    /// stream)`. Idempotent. Callers invoke this on every successful
    /// batch flush so a worker restart resumes from the next event
    /// after the last persisted offset.
    pub fn consumer_offset_upsert(
        &self,
        consumer_id: &str,
        stream: &str,
        last_offset: u64,
        last_event_id: Option<&str>,
    ) -> Result<(), MetadataError> {
        let updated_at = Utc::now().to_rfc3339();
        // Cap at i64::MAX — SQLite INTEGER is 64-bit signed; offsets
        // beyond 2^63 would corrupt the row, but Synap streams roll
        // over long before that.
        let stored_offset = last_offset.min(i64::MAX as u64) as i64;
        self.conn.execute(
            "INSERT INTO consumer_offsets (consumer_id, stream, last_offset, last_event_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(consumer_id, stream) DO UPDATE SET
               last_offset    = MAX(consumer_offsets.last_offset, excluded.last_offset),
               last_event_id  = excluded.last_event_id,
               updated_at     = excluded.updated_at",
            params![consumer_id, stream, stored_offset, last_event_id, updated_at],
        )?;
        Ok(())
    }

    /// Phase11s §2.4 — force the persisted offset back to `target_offset`
    /// for the given `(consumer_id, stream)` pair. Used by the
    /// `cortex-ops graph replay --since=<event_id>` subcommand to
    /// rewind a consumer cursor when an operator knows a window
    /// was lost. Unlike [`Self::consumer_offset_upsert`] this does NOT
    /// take the MAX of stored vs. supplied — replay must be able to
    /// move BACKWARDS in the stream.
    pub fn consumer_offset_set(
        &self,
        consumer_id: &str,
        stream: &str,
        target_offset: u64,
    ) -> Result<(), MetadataError> {
        let updated_at = Utc::now().to_rfc3339();
        let stored = target_offset.min(i64::MAX as u64) as i64;
        self.conn.execute(
            "INSERT INTO consumer_offsets (consumer_id, stream, last_offset, last_event_id, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4)
             ON CONFLICT(consumer_id, stream) DO UPDATE SET
               last_offset    = excluded.last_offset,
               last_event_id  = NULL,
               updated_at     = excluded.updated_at",
            params![consumer_id, stream, stored, updated_at],
        )?;
        Ok(())
    }

    // ---------- phase10c bootstrap-dedup helpers ----------

    /// Phase10c — look up an existing `bootstrap_seen` row for
    /// `(repo, path)`. Returns `None` when the file has never been
    /// emitted, otherwise the previously-stored `(content_hash,
    /// last_run_id)` tuple so the walker can branch on whether the
    /// content changed since the last run.
    pub fn bootstrap_seen_lookup(
        &self,
        repo: &str,
        path: &str,
    ) -> Result<Option<BootstrapSeenRow>, MetadataError> {
        let row = self
            .conn
            .query_row(
                "SELECT content_hash, last_run_id, last_emitted_at FROM bootstrap_seen
                 WHERE repo = ?1 AND path = ?2",
                params![repo, path],
                |r| {
                    Ok(BootstrapSeenRow {
                        content_hash: r.get(0)?,
                        last_run_id: r.get::<_, Option<String>>(1)?,
                        last_emitted_at: r.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Phase10c — upsert the dedup ledger for `(repo, path)`. The
    /// caller passes the redacted-body content hash + the current
    /// run id; on conflict the row is replaced. Idempotent.
    pub fn bootstrap_seen_upsert(
        &self,
        repo: &str,
        path: &str,
        content_hash: &str,
        run_id: Option<&str>,
        emitted_at: DateTime<Utc>,
    ) -> Result<(), MetadataError> {
        self.conn.execute(
            "INSERT INTO bootstrap_seen (repo, path, content_hash, last_run_id, last_emitted_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo, path) DO UPDATE SET
               content_hash    = excluded.content_hash,
               last_run_id     = excluded.last_run_id,
               last_emitted_at = excluded.last_emitted_at",
            params![repo, path, content_hash, run_id, emitted_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Phase10c — count the ledger rows scoped to `repo` (or all
    /// rows when `repo` is `None`). The walker's pre-flight warning
    /// branches on this — when the ledger is empty AND the live
    /// lane carries far more rows than the disk has files, a
    /// `cortex-ops bootstrap dedup` run is the suggested remedy.
    pub fn bootstrap_seen_count(&self, repo: Option<&str>) -> Result<u64, MetadataError> {
        let count: i64 = match repo {
            Some(r) => self.conn.query_row(
                "SELECT COUNT(*) FROM bootstrap_seen WHERE repo = ?1",
                params![r],
                |row| row.get(0),
            )?,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM bootstrap_seen", [], |row| row.get(0))?,
        };
        Ok(count.max(0) as u64)
    }

    /// Phase10c — group `bootstrap_seen` rows by `content_hash`
    /// within `repo` and return every group of size ≥ 2. Used by
    /// the dedup CLI to surface files that historically emitted the
    /// same redacted body under different paths (a less-common but
    /// real corruption mode).
    pub fn bootstrap_seen_duplicates_by_hash(
        &self,
        repo: &str,
    ) -> Result<Vec<BootstrapSeenDuplicate>, MetadataError> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, GROUP_CONCAT(path, '\u{1f}'), COUNT(*) AS n
             FROM bootstrap_seen
             WHERE repo = ?1
             GROUP BY content_hash
             HAVING n >= 2
             ORDER BY n DESC, content_hash ASC",
        )?;
        let rows = stmt.query_map(params![repo], |r| {
            let hash: String = r.get(0)?;
            let paths_blob: String = r.get(1)?;
            let count: i64 = r.get(2)?;
            Ok(BootstrapSeenDuplicate {
                content_hash: hash,
                paths: paths_blob.split('\u{1f}').map(String::from).collect(),
                count: count.max(0) as u64,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

/// Phase10c — assert the bootstrap dedup ledger exists on `conn`.
/// The bundled `schema.sql` creates the table at every
/// [`MetadataStore::open`]; this helper exists so callers that
/// hold a borrowed connection can be explicit about the
/// dependency. Idempotent (`CREATE TABLE IF NOT EXISTS`).
pub fn apply_phase10c_schema(conn: &Connection) -> Result<(), MetadataError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bootstrap_seen (
            repo            TEXT NOT NULL,
            path            TEXT NOT NULL,
            content_hash    TEXT NOT NULL,
            last_run_id     TEXT,
            last_emitted_at TEXT NOT NULL,
            PRIMARY KEY (repo, path)
        );
        CREATE INDEX IF NOT EXISTS bootstrap_seen_hash ON bootstrap_seen (repo, content_hash);",
    )?;
    Ok(())
}

/// Phase13b — assert the producer-checkpoint table exists on
/// `conn`. New in ADR-010. Append-only: every emit batch from
/// every producer writes one row, keyed by
/// `(producer_name, scope, accumulated_at)` so two invocations
/// never collide. `latest_producer_checkpoint(name, scope)`
/// returns the row with the maximum `accumulated_at` to resume
/// after kill.
pub fn apply_phase13b_schema(conn: &Connection) -> Result<(), MetadataError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS producer_checkpoints (
            producer_name    TEXT NOT NULL,
            scope            TEXT NOT NULL,
            last_event_id    TEXT NOT NULL,
            last_occurred_at TEXT NOT NULL,
            accumulated_at   TEXT NOT NULL,
            PRIMARY KEY (producer_name, scope, accumulated_at)
        );
        CREATE INDEX IF NOT EXISTS producer_checkpoints_latest
            ON producer_checkpoints (producer_name, scope, accumulated_at DESC);",
    )?;
    Ok(())
}

/// Phase9k — assert the cron scheduler registry exists on `conn`.
/// Mirrors [`apply_phase9g_schema`]: the bundled `schema.sql`
/// already creates the table at every open, but the cortex-ops
/// daemon calls this helper at startup so the dependency is
/// explicit and the daemon doesn't have to re-`MetadataStore::open`
/// just to land the column-adds for older DBs.
pub fn apply_phase9k_schema(conn: &Connection) -> Result<(), MetadataError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cron_jobs (
            name           TEXT PRIMARY KEY,
            schedule       TEXT NOT NULL,
            command        TEXT NOT NULL,
            enabled        INTEGER NOT NULL DEFAULT 1,
            last_run_at    TEXT,
            last_status    TEXT,
            next_run_at    TEXT,
            last_error     TEXT,
            last_stdout    TEXT,
            last_stderr    TEXT,
            failure_streak INTEGER NOT NULL DEFAULT 0,
            last_warning_at TEXT
        );
        CREATE INDEX IF NOT EXISTS cron_jobs_due ON cron_jobs (enabled, next_run_at);",
    )?;
    Ok(())
}

/// Phase9g — assert the metadata reaper's rollup tables exist on
/// `conn`. The schema bundled with [`MetadataStore`] already creates
/// them at every open via [`SCHEMA_SQL`]; this helper is exposed so
/// callers that hold a borrowed connection (e.g. the reaper running
/// inside a `BEGIN IMMEDIATE` transaction on the same DB) can be
/// explicit about the dependency. Idempotent — uses
/// `CREATE TABLE IF NOT EXISTS`.
pub fn apply_phase9g_schema(conn: &Connection) -> Result<(), MetadataError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bootstrap_jobs_daily (
            day          TEXT NOT NULL,
            repo_path    TEXT NOT NULL,
            runs         INTEGER NOT NULL DEFAULT 0,
            total_files  INTEGER NOT NULL DEFAULT 0,
            total_chunks INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (day, repo_path)
        );
        CREATE TABLE IF NOT EXISTS sessions_monthly (
            year_month        TEXT NOT NULL,
            tool              TEXT NOT NULL,
            repo              TEXT NOT NULL,
            count             INTEGER NOT NULL DEFAULT 0,
            total_event_count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (year_month, tool, repo)
        );
        CREATE TABLE IF NOT EXISTS classifier_spend_monthly (
            year_month     TEXT PRIMARY KEY,
            calls          INTEGER NOT NULL DEFAULT 0,
            tokens_in      INTEGER NOT NULL DEFAULT 0,
            tokens_out     INTEGER NOT NULL DEFAULT 0,
            est_usd_cents  INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(())
}

/// Phase9g — daily bootstrap-job series suitable for the dashboard.
/// Output is sorted oldest-first by `day`. Each entry sums the raw
/// `bootstrap_jobs` rows still living in the store with the
/// already-rolled `bootstrap_jobs_daily` rows so a chart that spans
/// the rollup boundary stays continuous after the reaper runs.
///
/// `since` and `until` are RFC-3339 timestamps; the helper truncates
/// them to `YYYY-MM-DD` so the day-level grouping is stable across
/// invocations.
pub fn union_read_bootstrap_jobs(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Vec<BootstrapDailyRow>, MetadataError> {
    let since_day = since.format("%Y-%m-%d").to_string();
    let until_day = until.format("%Y-%m-%d").to_string();
    let mut acc: std::collections::BTreeMap<(String, String), BootstrapDailyRow> =
        std::collections::BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT substr(finished_at, 1, 10) AS day,
                    repo_path,
                    COUNT(*),
                    COALESCE(SUM(files_processed), 0),
                    COALESCE(SUM(chunks_emitted), 0)
               FROM bootstrap_jobs
              WHERE status = 'success'
                AND substr(finished_at, 1, 10) >= ?1
                AND substr(finished_at, 1, 10) <= ?2
              GROUP BY day, repo_path",
        )?;
        let rows = stmt.query_map(params![since_day, until_day], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
                r.get::<_, i64>(4)? as u64,
            ))
        })?;
        for row in rows {
            let (day, repo_path, runs, total_files, total_chunks) = row?;
            acc.insert(
                (day.clone(), repo_path.clone()),
                BootstrapDailyRow {
                    day,
                    repo_path,
                    runs,
                    total_files,
                    total_chunks,
                },
            );
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT day, repo_path, runs, total_files, total_chunks
               FROM bootstrap_jobs_daily
              WHERE day >= ?1 AND day <= ?2",
        )?;
        let rows = stmt.query_map(params![since_day, until_day], |r| {
            Ok(BootstrapDailyRow {
                day: r.get(0)?,
                repo_path: r.get(1)?,
                runs: r.get::<_, i64>(2)? as u64,
                total_files: r.get::<_, i64>(3)? as u64,
                total_chunks: r.get::<_, i64>(4)? as u64,
            })
        })?;
        for row in rows {
            let row = row?;
            let key = (row.day.clone(), row.repo_path.clone());
            match acc.get_mut(&key) {
                Some(existing) => {
                    existing.runs = existing.runs.saturating_add(row.runs);
                    existing.total_files = existing.total_files.saturating_add(row.total_files);
                    existing.total_chunks = existing.total_chunks.saturating_add(row.total_chunks);
                }
                None => {
                    acc.insert(key, row);
                }
            }
        }
    }
    Ok(acc.into_values().collect())
}

/// Phase9g — monthly session series unioned across raw `sessions`
/// rows and the rolled `sessions_monthly` table. `since`/`until`
/// are truncated to `YYYY-MM`.
pub fn union_read_sessions(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Vec<SessionsMonthlyRow>, MetadataError> {
    let since_month = since.format("%Y-%m").to_string();
    let until_month = until.format("%Y-%m").to_string();
    let mut acc: std::collections::BTreeMap<(String, String, String), SessionsMonthlyRow> =
        std::collections::BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT substr(started_at, 1, 7) AS year_month,
                    tool,
                    COALESCE(repo, ''),
                    COUNT(*),
                    COALESCE(SUM(event_count), 0)
               FROM sessions
              WHERE substr(started_at, 1, 7) >= ?1
                AND substr(started_at, 1, 7) <= ?2
              GROUP BY year_month, tool, COALESCE(repo, '')",
        )?;
        let rows = stmt.query_map(params![since_month, until_month], |r| {
            Ok(SessionsMonthlyRow {
                year_month: r.get(0)?,
                tool: r.get(1)?,
                repo: r.get(2)?,
                count: r.get::<_, i64>(3)? as u64,
                total_event_count: r.get::<_, i64>(4)? as u64,
            })
        })?;
        for row in rows {
            let row = row?;
            acc.insert(
                (row.year_month.clone(), row.tool.clone(), row.repo.clone()),
                row,
            );
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT year_month, tool, repo, count, total_event_count
               FROM sessions_monthly
              WHERE year_month >= ?1 AND year_month <= ?2",
        )?;
        let rows = stmt.query_map(params![since_month, until_month], |r| {
            Ok(SessionsMonthlyRow {
                year_month: r.get(0)?,
                tool: r.get(1)?,
                repo: r.get(2)?,
                count: r.get::<_, i64>(3)? as u64,
                total_event_count: r.get::<_, i64>(4)? as u64,
            })
        })?;
        for row in rows {
            let row = row?;
            let key = (row.year_month.clone(), row.tool.clone(), row.repo.clone());
            match acc.get_mut(&key) {
                Some(existing) => {
                    existing.count = existing.count.saturating_add(row.count);
                    existing.total_event_count = existing
                        .total_event_count
                        .saturating_add(row.total_event_count);
                }
                None => {
                    acc.insert(key, row);
                }
            }
        }
    }
    Ok(acc.into_values().collect())
}

/// Phase9g — monthly classifier-spend series unioned across the raw
/// daily `classifier_spend` rows and the rolled
/// `classifier_spend_monthly` table.
pub fn union_read_classifier_spend(
    conn: &Connection,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Vec<ClassifierSpendMonthlyRow>, MetadataError> {
    let since_month = since.format("%Y-%m").to_string();
    let until_month = until.format("%Y-%m").to_string();
    let mut acc: std::collections::BTreeMap<String, ClassifierSpendMonthlyRow> =
        std::collections::BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT substr(day, 1, 7) AS year_month,
                    COALESCE(SUM(calls), 0),
                    COALESCE(SUM(tokens_in), 0),
                    COALESCE(SUM(tokens_out), 0),
                    COALESCE(SUM(est_usd_cents), 0)
               FROM classifier_spend
              WHERE substr(day, 1, 7) >= ?1
                AND substr(day, 1, 7) <= ?2
              GROUP BY year_month",
        )?;
        let rows = stmt.query_map(params![since_month, until_month], |r| {
            Ok(ClassifierSpendMonthlyRow {
                year_month: r.get(0)?,
                calls: r.get::<_, i64>(1)? as u64,
                tokens_in: r.get::<_, i64>(2)? as u64,
                tokens_out: r.get::<_, i64>(3)? as u64,
                est_usd_cents: r.get::<_, i64>(4)? as u64,
            })
        })?;
        for row in rows {
            let row = row?;
            acc.insert(row.year_month.clone(), row);
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT year_month, calls, tokens_in, tokens_out, est_usd_cents
               FROM classifier_spend_monthly
              WHERE year_month >= ?1 AND year_month <= ?2",
        )?;
        let rows = stmt.query_map(params![since_month, until_month], |r| {
            Ok(ClassifierSpendMonthlyRow {
                year_month: r.get(0)?,
                calls: r.get::<_, i64>(1)? as u64,
                tokens_in: r.get::<_, i64>(2)? as u64,
                tokens_out: r.get::<_, i64>(3)? as u64,
                est_usd_cents: r.get::<_, i64>(4)? as u64,
            })
        })?;
        for row in rows {
            let row = row?;
            match acc.get_mut(&row.year_month) {
                Some(existing) => {
                    existing.calls = existing.calls.saturating_add(row.calls);
                    existing.tokens_in = existing.tokens_in.saturating_add(row.tokens_in);
                    existing.tokens_out = existing.tokens_out.saturating_add(row.tokens_out);
                    existing.est_usd_cents =
                        existing.est_usd_cents.saturating_add(row.est_usd_cents);
                }
                None => {
                    acc.insert(row.year_month.clone(), row);
                }
            }
        }
    }
    Ok(acc.into_values().collect())
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

/// Phase11s §2.3 — one row from `consumer_offsets`. The graph /
/// fulltext / embedder workers read this on boot to resume from
/// the last successfully-processed event instead of defaulting to
/// "latest" (which silently dropped 12 of 20 sibling repos during
/// the 2026-05-03 reindex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerOffsetRow {
    /// Last successfully-processed Synap stream offset (monotonically
    /// increasing per `(consumer_id, stream)` pair).
    pub last_offset: u64,
    /// Last successfully-processed envelope `event_id`, when known.
    /// Stamped on every advance so a future `consume_after_event_id`
    /// API can resume by id rather than offset.
    pub last_event_id: Option<String>,
    /// RFC-3339 timestamp of the last advance.
    pub updated_at: String,
}

/// Phase10c — one row from `bootstrap_seen`. The walker reads
/// this to decide whether a re-emit is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapSeenRow {
    /// `sha256:<hex>` over the redacted file body.
    pub content_hash: String,
    /// Last `run_id` (ULID) that emitted this row, when known.
    pub last_run_id: Option<String>,
    /// RFC-3339 timestamp of the last emit.
    pub last_emitted_at: String,
}

/// Phase10c — one duplicate group surfaced by the dedup CLI:
/// every `bootstrap_seen` row that shares a `content_hash` within
/// the same repo. Group size ≥ 2 by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapSeenDuplicate {
    /// Shared `sha256:<hex>` content hash.
    pub content_hash: String,
    /// Distinct paths whose redacted body hashed to the same
    /// value. Sorted (SQLite `GROUP_CONCAT` returns them in row
    /// order, which we then split on the unit-separator
    /// delimiter).
    pub paths: Vec<String>,
    /// Number of paths in the group (always equal to
    /// `paths.len()`).
    pub count: u64,
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

/// Phase13b — one row from `producer_checkpoints`. New under
/// ADR-010. Append-only: every emit batch from every producer
/// writes one row; the latest row per `(producer_name, scope)`
/// is what the producer reads on resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerCheckpointRow {
    /// Canonical producer name (`bootstrap`, `claude_archive`,
    /// `topic_cards`, `consolidator`, plus future adapters).
    pub producer_name: String,
    /// Per-(producer, sub-source) scope. Bootstrap uses the
    /// repo slug; consolidator uses the grain
    /// (`session`/`topic`/`decision`); claude-archive uses the
    /// project path; topic-cards uses the topic slug.
    pub scope: String,
    /// `event_id` of the last envelope this batch emitted.
    pub last_event_id: String,
    /// RFC-3339 `occurred_at` of the last envelope.
    pub last_occurred_at: String,
    /// RFC-3339 timestamp the row landed.
    pub accumulated_at: String,
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

/// Phase9k — one row of the cron_jobs registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJob {
    /// Stable job identifier (`retention.sweep`, …).
    pub name: String,
    /// 5-field cron expression. UTC.
    pub schedule: String,
    /// Shell command line the scheduler spawns.
    pub command: String,
    /// `true` when the scheduler will pick the row up.
    pub enabled: bool,
    /// RFC-3339 timestamp of the last run start.
    pub last_run_at: Option<String>,
    /// Last run status: `success` / `failed` / `running`.
    pub last_status: Option<String>,
    /// RFC-3339 timestamp of the next scheduled run.
    pub next_run_at: Option<String>,
    /// First line of stderr (or other diagnostic) when the last run
    /// failed. Capped at 256 chars.
    pub last_error: Option<String>,
    /// Tail of the most recent stdout (capped at 64 KB by the
    /// scheduler).
    pub last_stdout: Option<String>,
    /// Tail of the most recent stderr (capped at 64 KB).
    pub last_stderr: Option<String>,
    /// Number of consecutive failures. Reset to 0 on success.
    pub failure_streak: u32,
    /// RFC-3339 timestamp the last `schedule.repeated_failure`
    /// warning fired — used to dedup within a 24 h window.
    pub last_warning_at: Option<String>,
}

fn cron_job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    Ok(CronJob {
        name: row.get(0)?,
        schedule: row.get(1)?,
        command: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        last_run_at: row.get(4)?,
        last_status: row.get(5)?,
        next_run_at: row.get(6)?,
        last_error: row.get(7)?,
        last_stdout: row.get(8)?,
        last_stderr: row.get(9)?,
        failure_streak: row.get::<_, i64>(10)?.max(0) as u32,
        last_warning_at: row.get(11)?,
    })
}

/// Phase9g — one row of the bootstrap series (raw + rolled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapDailyRow {
    /// `YYYY-MM-DD` UTC day bucket.
    pub day: String,
    /// Repo this row aggregates.
    pub repo_path: String,
    /// Number of successful runs in the bucket.
    pub runs: u64,
    /// Sum of `files_processed` across the bucket.
    pub total_files: u64,
    /// Sum of `chunks_emitted` across the bucket.
    pub total_chunks: u64,
}

/// Phase9g — one row of the sessions series (raw + rolled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionsMonthlyRow {
    /// `YYYY-MM` UTC month bucket.
    pub year_month: String,
    /// Tool name (`claude-code`, `codex`, …).
    pub tool: String,
    /// Repo path; empty string when the source row left it NULL.
    pub repo: String,
    /// Number of sessions in the bucket.
    pub count: u64,
    /// Sum of `event_count` across the bucket.
    pub total_event_count: u64,
}

/// Phase9g — one row of the classifier-spend series (raw + rolled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierSpendMonthlyRow {
    /// `YYYY-MM` UTC month bucket.
    pub year_month: String,
    /// Sum of classifier calls.
    pub calls: u64,
    /// Sum of input tokens.
    pub tokens_in: u64,
    /// Sum of output tokens.
    pub tokens_out: u64,
    /// Sum of estimated spend in US cents.
    pub est_usd_cents: u64,
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

    // ----- Phase13b §2.5 — producer_checkpoints helpers ---------

    #[test]
    fn producer_checkpoint_returns_none_when_table_empty() {
        let store = MetadataStore::open_in_memory().unwrap();
        let latest = store
            .latest_producer_checkpoint("bootstrap", "cortex")
            .unwrap();
        assert!(latest.is_none());
    }

    #[test]
    fn producer_checkpoint_round_trips_a_single_write() {
        let store = MetadataStore::open_in_memory().unwrap();
        let occurred = now();
        let accumulated = occurred + chrono::Duration::seconds(1);
        store
            .record_producer_checkpoint("bootstrap", "cortex", "01ENV", occurred, accumulated)
            .unwrap();
        let latest = store
            .latest_producer_checkpoint("bootstrap", "cortex")
            .unwrap()
            .expect("row");
        assert_eq!(latest.producer_name, "bootstrap");
        assert_eq!(latest.scope, "cortex");
        assert_eq!(latest.last_event_id, "01ENV");
    }

    #[test]
    fn producer_checkpoint_returns_newest_row_when_multiple_per_scope() {
        let store = MetadataStore::open_in_memory().unwrap();
        let base = DateTime::parse_from_rfc3339("2026-05-19T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (i, id) in ["01OLD", "01MID", "01NEW"].iter().enumerate() {
            let accumulated = base + chrono::Duration::seconds(i as i64);
            store
                .record_producer_checkpoint("bootstrap", "cortex", id, base, accumulated)
                .unwrap();
        }
        let latest = store
            .latest_producer_checkpoint("bootstrap", "cortex")
            .unwrap()
            .expect("row");
        assert_eq!(latest.last_event_id, "01NEW");
    }

    #[test]
    fn producer_checkpoint_lists_all_rows_per_producer_across_scopes() {
        let store = MetadataStore::open_in_memory().unwrap();
        let base = DateTime::parse_from_rfc3339("2026-05-19T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (i, scope) in ["repo_a", "repo_b", "repo_c"].iter().enumerate() {
            let accumulated = base + chrono::Duration::seconds(i as i64);
            store
                .record_producer_checkpoint("bootstrap", scope, "01ENV", base, accumulated)
                .unwrap();
        }
        let rows = store
            .list_producer_checkpoints_for("bootstrap", 50)
            .unwrap();
        assert_eq!(rows.len(), 3);
        // Newest first per the ORDER BY.
        assert_eq!(rows[0].scope, "repo_c");
        assert_eq!(rows[2].scope, "repo_a");
    }

    #[test]
    fn producer_checkpoint_scope_discriminates_between_sources() {
        let store = MetadataStore::open_in_memory().unwrap();
        let base = now();
        store
            .record_producer_checkpoint("bootstrap", "cortex", "01CORTEX", base, base)
            .unwrap();
        store
            .record_producer_checkpoint(
                "bootstrap",
                "vectorizer",
                "01VEC",
                base,
                base + chrono::Duration::seconds(1),
            )
            .unwrap();
        let cortex = store
            .latest_producer_checkpoint("bootstrap", "cortex")
            .unwrap()
            .expect("cortex row");
        let vec = store
            .latest_producer_checkpoint("bootstrap", "vectorizer")
            .unwrap()
            .expect("vec row");
        assert_eq!(cortex.last_event_id, "01CORTEX");
        assert_eq!(vec.last_event_id, "01VEC");
    }

    #[test]
    fn latest_producer_checkpoint_per_scope_returns_one_row_per_pair_sorted() {
        let store = MetadataStore::open_in_memory().unwrap();
        let base = now();
        // bootstrap / cortex: two writes — only the newer survives.
        store
            .record_producer_checkpoint("bootstrap", "cortex", "01A", base, base)
            .unwrap();
        store
            .record_producer_checkpoint(
                "bootstrap",
                "cortex",
                "01B",
                base,
                base + chrono::Duration::seconds(2),
            )
            .unwrap();
        // bootstrap / vectorizer: one write.
        store
            .record_producer_checkpoint(
                "bootstrap",
                "vectorizer",
                "01V",
                base,
                base + chrono::Duration::seconds(1),
            )
            .unwrap();
        // claude_archive / proj1: one write.
        store
            .record_producer_checkpoint(
                "claude_archive",
                "proj1",
                "01P",
                base,
                base + chrono::Duration::seconds(3),
            )
            .unwrap();

        let rows = store.latest_producer_checkpoint_per_scope().unwrap();
        assert_eq!(rows.len(), 3, "one row per (producer, scope) pair");
        // Sort order: producer_name ASC, then scope ASC.
        assert_eq!(rows[0].producer_name, "bootstrap");
        assert_eq!(rows[0].scope, "cortex");
        assert_eq!(rows[0].last_event_id, "01B", "newest wins per scope");
        assert_eq!(rows[1].producer_name, "bootstrap");
        assert_eq!(rows[1].scope, "vectorizer");
        assert_eq!(rows[1].last_event_id, "01V");
        assert_eq!(rows[2].producer_name, "claude_archive");
        assert_eq!(rows[2].scope, "proj1");
        assert_eq!(rows[2].last_event_id, "01P");
    }

    #[test]
    fn latest_producer_checkpoint_per_scope_returns_empty_when_table_empty() {
        let store = MetadataStore::open_in_memory().unwrap();
        let rows = store.latest_producer_checkpoint_per_scope().unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn union_read_returns_identical_totals_before_and_after_rollup() {
        // Phase9g 5.2 — sanity-check the read-side union: the
        // combined raw + rolled view MUST yield the same daily /
        // monthly totals before and after a reaper run.
        let store = MetadataStore::open_in_memory().unwrap();
        // Seed a repo + 5 success bootstrap rows landing 35 days
        // ago (rollup-eligible) plus 2 success rows landing 5 days
        // ago (still raw).
        store
            .conn()
            .execute(
                "INSERT INTO repos (path, name) VALUES (?1, ?2)",
                params!["/repo/x", "/repo/x"],
            )
            .unwrap();
        let aged = now() - chrono::Duration::days(35);
        let fresh = now() - chrono::Duration::days(5);
        for i in 0..5 {
            store
                .conn()
                .execute(
                    "INSERT INTO bootstrap_jobs
                        (job_id, repo_path, started_at, finished_at,
                         files_processed, chunks_emitted, status)
                     VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'success')",
                    params![
                        format!("01A{i}"),
                        "/repo/x",
                        aged.to_rfc3339(),
                        10_i64,
                        100_i64
                    ],
                )
                .unwrap();
        }
        for i in 0..2 {
            store
                .conn()
                .execute(
                    "INSERT INTO bootstrap_jobs
                        (job_id, repo_path, started_at, finished_at,
                         files_processed, chunks_emitted, status)
                     VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'success')",
                    params![
                        format!("01B{i}"),
                        "/repo/x",
                        fresh.to_rfc3339(),
                        1_i64,
                        7_i64
                    ],
                )
                .unwrap();
        }
        let since = now() - chrono::Duration::days(60);
        let until = now();
        // Pre-rollup totals.
        let pre = union_read_bootstrap_jobs(store.conn(), since, until).unwrap();
        let pre_runs: u64 = pre.iter().map(|r| r.runs).sum();
        let pre_files: u64 = pre.iter().map(|r| r.total_files).sum();
        let pre_chunks: u64 = pre.iter().map(|r| r.total_chunks).sum();
        assert_eq!(pre_runs, 7);
        assert_eq!(pre_files, 5 * 10 + 2);
        assert_eq!(pre_chunks, 5 * 100 + 2 * 7);
        // Apply rollup directly via the same SQL the reaper uses.
        let cutoff = (now() - chrono::Duration::days(30)).to_rfc3339();
        store
            .conn()
            .execute(
                "INSERT INTO bootstrap_jobs_daily (day, repo_path, runs, total_files, total_chunks)
                      SELECT substr(finished_at, 1, 10), repo_path, COUNT(*),
                             COALESCE(SUM(files_processed), 0),
                             COALESCE(SUM(chunks_emitted), 0)
                        FROM bootstrap_jobs
                       WHERE status = 'success' AND finished_at < ?1
                       GROUP BY substr(finished_at, 1, 10), repo_path",
                params![cutoff],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "DELETE FROM bootstrap_jobs WHERE status='success' AND finished_at < ?1",
                params![cutoff],
            )
            .unwrap();
        // Post-rollup totals.
        let post = union_read_bootstrap_jobs(store.conn(), since, until).unwrap();
        let post_runs: u64 = post.iter().map(|r| r.runs).sum();
        let post_files: u64 = post.iter().map(|r| r.total_files).sum();
        let post_chunks: u64 = post.iter().map(|r| r.total_chunks).sum();
        assert_eq!(pre_runs, post_runs);
        assert_eq!(pre_files, post_files);
        assert_eq!(pre_chunks, post_chunks);
    }

    // ---------- phase10c bootstrap dedup ----------

    #[test]
    fn bootstrap_seen_lookup_returns_none_for_unknown_path() {
        let store = MetadataStore::open_in_memory().unwrap();
        let row = store.bootstrap_seen_lookup("Cortex", "src/lib.rs").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn bootstrap_seen_upsert_then_lookup_round_trips_hash() {
        let store = MetadataStore::open_in_memory().unwrap();
        let when = now();
        store
            .bootstrap_seen_upsert("Cortex", "src/lib.rs", "sha256:abc", Some("01RUN"), when)
            .unwrap();
        let row = store
            .bootstrap_seen_lookup("Cortex", "src/lib.rs")
            .unwrap()
            .expect("row present after upsert");
        assert_eq!(row.content_hash, "sha256:abc");
        assert_eq!(row.last_run_id.as_deref(), Some("01RUN"));
    }

    #[test]
    fn bootstrap_seen_upsert_replaces_existing_row_for_same_path() {
        let store = MetadataStore::open_in_memory().unwrap();
        let when = now();
        store
            .bootstrap_seen_upsert("Cortex", "src/lib.rs", "sha256:v1", Some("01R1"), when)
            .unwrap();
        // Same (repo, path) with a different hash -> replace.
        store
            .bootstrap_seen_upsert(
                "Cortex",
                "src/lib.rs",
                "sha256:v2",
                Some("01R2"),
                when + chrono::Duration::seconds(60),
            )
            .unwrap();
        let row = store
            .bootstrap_seen_lookup("Cortex", "src/lib.rs")
            .unwrap()
            .unwrap();
        assert_eq!(row.content_hash, "sha256:v2");
        assert_eq!(row.last_run_id.as_deref(), Some("01R2"));
        // Count is still 1 — same primary key means upsert, not insert.
        assert_eq!(store.bootstrap_seen_count(Some("Cortex")).unwrap(), 1);
    }

    #[test]
    fn bootstrap_seen_count_scopes_to_repo_when_set() {
        let store = MetadataStore::open_in_memory().unwrap();
        let when = now();
        store
            .bootstrap_seen_upsert("A", "f1", "sha256:1", None, when)
            .unwrap();
        store
            .bootstrap_seen_upsert("A", "f2", "sha256:2", None, when)
            .unwrap();
        store
            .bootstrap_seen_upsert("B", "f1", "sha256:1", None, when)
            .unwrap();
        assert_eq!(store.bootstrap_seen_count(Some("A")).unwrap(), 2);
        assert_eq!(store.bootstrap_seen_count(Some("B")).unwrap(), 1);
        assert_eq!(store.bootstrap_seen_count(None).unwrap(), 3);
    }

    #[test]
    fn bootstrap_seen_duplicates_by_hash_groups_paths() {
        let store = MetadataStore::open_in_memory().unwrap();
        let when = now();
        // Two paths sharing the same hash → one duplicate group.
        store
            .bootstrap_seen_upsert("R", "a.md", "sha256:dup", None, when)
            .unwrap();
        store
            .bootstrap_seen_upsert("R", "b.md", "sha256:dup", None, when)
            .unwrap();
        // Distinct hash → not in the dup output.
        store
            .bootstrap_seen_upsert("R", "c.md", "sha256:unique", None, when)
            .unwrap();
        let dups = store.bootstrap_seen_duplicates_by_hash("R").unwrap();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].content_hash, "sha256:dup");
        assert_eq!(dups[0].count, 2);
        assert!(dups[0].paths.contains(&"a.md".to_string()));
        assert!(dups[0].paths.contains(&"b.md".to_string()));
    }

    // ---------- phase10i — session tool preservation ----------

    fn read_session_tool(store: &MetadataStore, session_id: &str) -> Option<String> {
        store
            .conn
            .query_row(
                "SELECT tool FROM sessions WHERE session_id = ?1",
                params![session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .unwrap()
            .flatten()
    }

    #[test]
    fn upsert_session_preserves_tool_when_subsequent_write_passes_empty() {
        // phase10i — the audit caught 574 sessions with `tool IS
        // NULL` because the pre-phase10i upsert wrote
        // `tool=excluded.tool` unconditionally; lifecycle hooks
        // that didn't capture the tool name (Stop / Notification)
        // overwrote the session-start value with ''. The new
        // upsert's `COALESCE(NULLIF(excluded.tool, ''), sessions.tool)`
        // preserves the original.
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .upsert_session("01SESSA", "claude-code", None, None, None, now())
            .unwrap();
        assert_eq!(
            read_session_tool(&store, "01SESSA").as_deref(),
            Some("claude-code")
        );
        // Subsequent upsert with empty tool — must NOT overwrite.
        store
            .upsert_session("01SESSA", "", Some("sonnet"), None, None, now())
            .unwrap();
        assert_eq!(
            read_session_tool(&store, "01SESSA").as_deref(),
            Some("claude-code"),
            "empty tool on conflict must preserve the existing value"
        );
    }

    #[test]
    fn upsert_session_inserts_empty_tool_when_first_write_passes_empty() {
        // First write with empty tool: schema declares
        // `tool TEXT NOT NULL` so the literal `''` is stored
        // (NULL would violate the constraint). The backfill
        // CLI later stamps it via `set_session_tool`; the
        // `sessions_missing_tool` query catches both `''` and
        // `NULL`.
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .upsert_session("01SESSB", "", None, None, None, now())
            .unwrap();
        assert_eq!(read_session_tool(&store, "01SESSB").as_deref(), Some(""));
    }

    #[test]
    fn set_session_tool_stamps_value_and_returns_rowcount() {
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .upsert_session("01SESSC", "", None, None, None, now())
            .unwrap();
        let touched = store.set_session_tool("01SESSC", "claude-code").unwrap();
        assert_eq!(touched, 1);
        assert_eq!(
            read_session_tool(&store, "01SESSC").as_deref(),
            Some("claude-code")
        );
        // Empty tool is a no-op.
        let touched_empty = store.set_session_tool("01SESSC", "").unwrap();
        assert_eq!(touched_empty, 0);
    }

    #[test]
    fn sessions_missing_tool_lists_only_null_or_empty_rows() {
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .upsert_session("01SESSD", "claude-code", None, None, None, now())
            .unwrap();
        store
            .upsert_session("01SESSE", "", None, None, None, now())
            .unwrap();
        let pending = store.sessions_missing_tool(10).unwrap();
        let ids: Vec<String> = pending.iter().map(|(id, _)| id.clone()).collect();
        assert!(!ids.contains(&"01SESSD".to_string()));
        assert!(ids.contains(&"01SESSE".to_string()));
    }

    // ----------------------------------------------------------------
    // Phase11s §2.3 / §2.4 — consumer_offsets ledger.
    // ----------------------------------------------------------------

    #[test]
    fn consumer_offset_lookup_returns_none_for_unknown_partition() {
        let store = MetadataStore::open_in_memory().unwrap();
        let row = store
            .consumer_offset_lookup("cortex-graph-0", "cortex.events.enriched")
            .unwrap();
        assert!(row.is_none(), "fresh DB must have no offset rows");
    }

    #[test]
    fn consumer_offset_upsert_advances_monotonically() {
        // Phase11s §2.3 — repeated upserts MUST advance the stored
        // offset, never roll it back. This protects against an
        // out-of-order ack landing after a higher-offset ack
        // already persisted.
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .consumer_offset_upsert(
                "cortex-graph-0",
                "cortex.events.enriched",
                100,
                Some("01EVT"),
            )
            .unwrap();
        // Smaller offset must NOT regress the stored value.
        store
            .consumer_offset_upsert(
                "cortex-graph-0",
                "cortex.events.enriched",
                50,
                Some("01OLD"),
            )
            .unwrap();
        let row = store
            .consumer_offset_lookup("cortex-graph-0", "cortex.events.enriched")
            .unwrap()
            .expect("upsert wrote a row");
        assert_eq!(row.last_offset, 100, "MAX semantics must hold");
    }

    #[test]
    fn consumer_offset_set_can_rewind() {
        // Phase11s §2.4 — `cortex-ops graph replay` rewinds the
        // cursor. `consumer_offset_set` is the rewind primitive
        // (distinct from `_upsert` which is monotonic). Pin the
        // contract so a future change does not silently apply
        // MAX semantics to rewinds.
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .consumer_offset_upsert(
                "cortex-graph-0",
                "cortex.events.enriched",
                500,
                Some("01HEAD"),
            )
            .unwrap();
        store
            .consumer_offset_set("cortex-graph-0", "cortex.events.enriched", 100)
            .unwrap();
        let row = store
            .consumer_offset_lookup("cortex-graph-0", "cortex.events.enriched")
            .unwrap()
            .expect("rewind preserves the row");
        assert_eq!(row.last_offset, 100);
        assert!(row.last_event_id.is_none(), "rewind clears last_event_id");
    }

    // ---------------------------------------------------------
    // Phase12f — record_cron_run TOCTOU regression suite
    // ---------------------------------------------------------

    #[test]
    fn record_cron_run_failed_increments_failure_streak() {
        let store = MetadataStore::open_in_memory().unwrap();
        store
            .upsert_cron_job_if_absent(
                "test.sweep",
                "0 3 * * *",
                "cortex-ops sweep",
                true,
                "2026-05-08T03:00:00+00:00",
            )
            .unwrap();
        let s1 = store
            .record_cron_run(
                "test.sweep",
                Utc::now(),
                "failed",
                None,
                None,
                Some("first failure"),
                "2026-05-09T03:00:00+00:00",
            )
            .unwrap();
        assert_eq!(s1, 1);
        let s2 = store
            .record_cron_run(
                "test.sweep",
                Utc::now(),
                "failed",
                None,
                None,
                Some("second failure"),
                "2026-05-10T03:00:00+00:00",
            )
            .unwrap();
        assert_eq!(s2, 2);
        let s3 = store
            .record_cron_run(
                "test.sweep",
                Utc::now(),
                "success",
                None,
                None,
                None,
                "2026-05-11T03:00:00+00:00",
            )
            .unwrap();
        assert_eq!(s3, 0, "success resets the streak");
    }

    #[test]
    fn record_cron_run_concurrent() {
        // Phase12f §2 — drive 4 OS threads × 250 calls each on the
        // same job, all backed by independent `MetadataStore` handles
        // pointing at the same SQLite file. The bug the new
        // single-statement UPDATE fixes is the read-modify-write race
        // on `failure_streak`: with the old SELECT-then-UPDATE path,
        // two concurrent supervisors would both read the same streak,
        // both compute streak+1, both UPDATE — net advance of 1
        // instead of 2. With the CASE-based atomic UPDATE every
        // failure increments exactly once.
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cron_concurrency.sqlite");

        // Seed the row through one connection so all worker threads
        // observe a starting point.
        {
            let store = MetadataStore::open(&db_path).unwrap();
            store
                .upsert_cron_job_if_absent(
                    "concurrent.sweep",
                    "0 4 * * *",
                    "cortex-ops concurrent",
                    true,
                    "2026-05-08T04:00:00+00:00",
                )
                .unwrap();
        }

        // Pre-tick the failure streak to the value we expect after
        // every-failure-counted: 4 threads × 250 calls = 1000 writes.
        // With the old race the observed streak was lower than 1000;
        // with the new path it equals 1000 exactly because every call
        // here uses `status="failed"`.
        let total_threads = 4;
        let calls_per_thread = 250;
        let total_calls = total_threads * calls_per_thread;
        let db_path = Arc::new(db_path);

        let mut handles = Vec::with_capacity(total_threads);
        for thread_idx in 0..total_threads {
            let path = db_path.clone();
            handles.push(thread::spawn(move || {
                let store = MetadataStore::open(&*path).unwrap();
                for call_idx in 0..calls_per_thread {
                    store
                        .record_cron_run(
                            "concurrent.sweep",
                            Utc::now(),
                            "failed",
                            Some(&format!("t{thread_idx}c{call_idx}")),
                            None,
                            Some("synthetic failure"),
                            "2026-05-08T04:00:00+00:00",
                        )
                        .expect("concurrent record_cron_run");
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread");
        }

        let store = MetadataStore::open(&*db_path).unwrap();
        let row = store
            .get_cron_job("concurrent.sweep")
            .unwrap()
            .expect("seeded row exists");
        assert_eq!(
            row.failure_streak, total_calls as u32,
            "every failure must advance the streak — observed {}, expected {} \
             (the old TOCTOU race produces a lower number under contention)",
            row.failure_streak, total_calls
        );
        assert_eq!(
            row.last_status.as_deref(),
            Some("failed"),
            "last_status reflects the last committing writer"
        );
    }

    #[test]
    fn record_cron_run_concurrent_with_success_resets() {
        // Mixed-status drive: each thread alternates failed/success.
        // Final state depends on which thread committed last; what we
        // pin is the invariant that the row exists, last_status is
        // one of the two valid values, and failure_streak is bounded
        // by the number of trailing `failed` writes from the last
        // committing thread (we cannot predict that exactly, so we
        // assert the bounds).
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let db_path = Arc::new(dir.path().join("cron_mixed.sqlite"));
        {
            let store = MetadataStore::open(&*db_path).unwrap();
            store
                .upsert_cron_job_if_absent(
                    "mixed.sweep",
                    "0 5 * * *",
                    "cortex-ops mixed",
                    true,
                    "2026-05-08T05:00:00+00:00",
                )
                .unwrap();
        }

        let total_threads = 4;
        let calls_per_thread = 100;
        let mut handles = Vec::with_capacity(total_threads);
        for thread_idx in 0..total_threads {
            let path = db_path.clone();
            handles.push(thread::spawn(move || {
                let store = MetadataStore::open(&*path).unwrap();
                for call_idx in 0..calls_per_thread {
                    let status = if (thread_idx + call_idx) % 2 == 0 {
                        "failed"
                    } else {
                        "success"
                    };
                    store
                        .record_cron_run(
                            "mixed.sweep",
                            Utc::now(),
                            status,
                            None,
                            None,
                            None,
                            "2026-05-08T05:00:00+00:00",
                        )
                        .expect("mixed record_cron_run");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let store = MetadataStore::open(&*db_path).unwrap();
        let row = store.get_cron_job("mixed.sweep").unwrap().unwrap();
        let last = row.last_status.as_deref();
        assert!(
            last == Some("failed") || last == Some("success"),
            "last_status must be one of the two values written, got {last:?}"
        );
        // failure_streak is bounded by the per-thread `calls_per_thread`
        // — no thread ever issues that many trailing failures in this
        // mix because the alternation forces a reset. So the streak
        // post-execution is always strictly less than calls_per_thread.
        assert!(
            (row.failure_streak as usize) < calls_per_thread,
            "alternating mix should keep streak below per-thread call count, got {}",
            row.failure_streak
        );
    }

    #[test]
    fn consumer_offset_partitions_by_consumer_and_stream() {
        // Two replicas + two streams = four independent rows.
        let store = MetadataStore::open_in_memory().unwrap();
        for (cid, stream, off) in [
            ("cortex-graph-0", "cortex.events.enriched", 10u64),
            ("cortex-graph-0", "cortex.events.bootstrap", 20),
            ("cortex-graph-1", "cortex.events.enriched", 30),
            ("cortex-graph-1", "cortex.events.bootstrap", 40),
        ] {
            store
                .consumer_offset_upsert(cid, stream, off, None)
                .unwrap();
        }
        let r = store
            .consumer_offset_lookup("cortex-graph-1", "cortex.events.enriched")
            .unwrap()
            .unwrap();
        assert_eq!(r.last_offset, 30);
        let r = store
            .consumer_offset_lookup("cortex-graph-0", "cortex.events.bootstrap")
            .unwrap()
            .unwrap();
        assert_eq!(r.last_offset, 20);
    }
}
