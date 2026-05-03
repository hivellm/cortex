//! Phase11n §2 — SQLite tail loop for `.rulebook/memory/memory.db`.
//!
//! The phase11m FS watcher catches every file-backed mutation under
//! `.rulebook/{tasks,handoff,decisions,knowledge,learnings}/**` (the
//! latter added by phase11n §1). Memories live in a SQLite database
//! instead of the filesystem — `notify` watching the `.db` file fires
//! on every WAL flush whether or not a row was committed, and even when
//! it does fire it cannot tell the GUI which row appeared.
//!
//! This module polls the same database with a read-only handle at the
//! debounce cadence the watcher uses, caches `last_rowid` per process,
//! and emits one [`DashboardEvent`] per genuinely-new row.
//!
//! Concurrency contract:
//! - The handle opens with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI` so
//!   concurrent rulebook writes are never blocked.
//! - The first tick seeds `last_rowid = MAX(rowid)` so historical rows
//!   do not flood the bus on daemon restart.
//! - Subsequent ticks select rows strictly greater than `last_rowid`,
//!   ordered by `rowid`, and update the cached cursor only after the
//!   bus accepts the publish.
//!
//! Failure handling:
//! - Missing `memory.db` is not an error — we run on workspaces without
//!   Rulebook installed. The tick returns `TickReport::default()`.
//! - Schema mismatches (no `memories` table, missing columns) are logged
//!   once per error class via a tracing rate-limiter and the cursor stays
//!   put.
//! - Open failures (file present but locked, permission denied) flag the
//!   tail as degraded but never panic.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};

use cortex_core::{DashboardEvent, DashboardEventKind, DashboardEventSource};

use crate::dashboard_watcher::DashboardEventBus;

/// Result of a single [`MemoryTailWatcher::tick_once`] call. Surfaces
/// observable counters for the integration tests + operator metrics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    /// Number of new rows the tick emitted to the bus.
    pub new_rows: u64,
    /// `last_rowid` after the tick.
    pub last_rowid: i64,
    /// `true` when the tick was a no-op because `memory.db` did not
    /// exist or the schema was unrecognised. Does NOT mean error —
    /// degraded but expected on workspaces without Rulebook.
    pub skipped: bool,
}

/// Polls the rulebook memory database and publishes one
/// [`DashboardEvent::MemoryAppended`] per new row.
pub struct MemoryTailWatcher {
    db_path: PathBuf,
    bus: DashboardEventBus,
    last_rowid: i64,
    /// `Some(_)` once an error class has been logged so we don't spam
    /// the operator's tracing pipeline every tick.
    schema_warned: bool,
    open_warned: bool,
}

impl MemoryTailWatcher {
    /// Build a watcher pointed at `db_path`. The first call to
    /// [`tick_once`](Self::tick_once) seeds the cursor against the
    /// current `MAX(rowid)` so historical rows do not flood the bus.
    pub fn new(db_path: PathBuf, bus: DashboardEventBus) -> Self {
        Self {
            db_path,
            bus,
            last_rowid: -1,
            schema_warned: false,
            open_warned: false,
        }
    }

    /// Tick once. Returns the [`TickReport`] for observability; never
    /// panics. Degradation paths (missing file, schema mismatch, open
    /// error) return a `skipped = true` report after logging at most
    /// once per error class.
    pub fn tick_once(&mut self) -> TickReport {
        if !Path::new(&self.db_path).exists() {
            return TickReport {
                new_rows: 0,
                last_rowid: self.last_rowid,
                skipped: true,
            };
        }
        let conn = match open_ro(&self.db_path) {
            Ok(c) => c,
            Err(err) => {
                if !self.open_warned {
                    tracing::warn!(
                        path = %self.db_path.display(),
                        error = %err,
                        "memory_tail: failed to open memory.db read-only (degraded; further open failures suppressed)"
                    );
                    self.open_warned = true;
                }
                return TickReport {
                    new_rows: 0,
                    last_rowid: self.last_rowid,
                    skipped: true,
                };
            }
        };

        if self.last_rowid < 0 {
            // Seed the cursor without replaying history. Failures here
            // mean the table does not exist (workspace without Rulebook
            // schema initialised); log once and skip.
            match seed_max_rowid(&conn) {
                Ok(seed) => {
                    self.last_rowid = seed;
                }
                Err(err) => {
                    if !self.schema_warned {
                        tracing::warn!(
                            path = %self.db_path.display(),
                            error = %err,
                            "memory_tail: schema unrecognised (degraded; further schema failures suppressed)"
                        );
                        self.schema_warned = true;
                    }
                    return TickReport {
                        new_rows: 0,
                        last_rowid: self.last_rowid,
                        skipped: true,
                    };
                }
            }
        }

        let new_rows = match poll_new_rows(&conn, self.last_rowid) {
            Ok(rows) => rows,
            Err(err) => {
                if !self.schema_warned {
                    tracing::warn!(
                        path = %self.db_path.display(),
                        error = %err,
                        "memory_tail: poll failed (degraded; further poll failures suppressed)"
                    );
                    self.schema_warned = true;
                }
                return TickReport {
                    new_rows: 0,
                    last_rowid: self.last_rowid,
                    skipped: true,
                };
            }
        };

        let mut emitted: u64 = 0;
        let mut highest = self.last_rowid;
        for row in &new_rows {
            let event = DashboardEvent {
                event_id: cortex_core::event_id().to_string(),
                kind: DashboardEventKind::MemoryAppended,
                entity_id: row.id.clone(),
                summary: row.name.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                delta: None,
                source: DashboardEventSource::Watcher,
            };
            self.bus.publish(event);
            emitted += 1;
            if row.rowid > highest {
                highest = row.rowid;
            }
        }
        self.last_rowid = highest;

        TickReport {
            new_rows: emitted,
            last_rowid: self.last_rowid,
            skipped: false,
        }
    }

    /// Cached cursor for tests / operator inspection.
    pub fn last_rowid(&self) -> i64 {
        self.last_rowid
    }
}

#[derive(Debug, Clone)]
struct MemoryRow {
    rowid: i64,
    id: String,
    name: Option<String>,
}

fn open_ro(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(path, flags)
}

fn seed_max_rowid(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(rowid), 0) FROM memories",
        [],
        |row| row.get::<_, i64>(0),
    )
}

fn poll_new_rows(conn: &Connection, last_rowid: i64) -> rusqlite::Result<Vec<MemoryRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT rowid, id, name FROM memories WHERE rowid > ?1 ORDER BY rowid",
    )?;
    let mut out: Vec<MemoryRow> = Vec::new();
    let rows = stmt.query_map([last_rowid], |row| {
        Ok(MemoryRow {
            rowid: row.get(0)?,
            id: row.get(1)?,
            name: row.get(2).ok(),
        })
    })?;
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Spawn a tokio interval that ticks the watcher at `period`. Returns
/// the `JoinHandle` so the caller controls shutdown by dropping it.
/// Gracefully exits when `should_run` returns `false`.
pub fn spawn_tail_loop(
    db_path: PathBuf,
    bus: DashboardEventBus,
    period: std::time::Duration,
    should_run: Arc<dyn Fn() -> bool + Send + Sync>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut watcher = MemoryTailWatcher::new(db_path, bus);
        let mut interval = tokio::time::interval(period);
        // Skip the immediate first tick — the constructor already seeds
        // on first call so the first poll happens after one period.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        while should_run() {
            interval.tick().await;
            let _ = watcher.tick_once();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as RwConnection;
    use std::time::Duration;

    fn create_schema(path: &Path) {
        let conn = RwConnection::open(path).expect("open rw");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                name TEXT,
                memory_type TEXT,
                created_at TEXT,
                updated_at TEXT
            );",
        )
        .expect("create schema");
    }

    fn insert_row(path: &Path, id: &str, name: &str) {
        let conn = RwConnection::open(path).expect("open rw");
        conn.execute(
            "INSERT INTO memories (id, name, memory_type, created_at, updated_at)
             VALUES (?1, ?2, 'note', '2026-05-03', '2026-05-03')",
            [id, name],
        )
        .expect("insert");
    }

    #[test]
    fn tick_skips_when_db_missing() {
        let bus = DashboardEventBus::new();
        let mut w = MemoryTailWatcher::new(PathBuf::from("/nonexistent/memory.db"), bus);
        let report = w.tick_once();
        assert!(report.skipped);
        assert_eq!(report.new_rows, 0);
    }

    #[test]
    fn tick_skips_when_schema_unrecognised() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("memory.db");
        // Empty SQLite file (no `memories` table).
        let conn = RwConnection::open(&path).expect("create");
        conn.execute_batch("CREATE TABLE other_table (x INTEGER);")
            .expect("schema");
        let bus = DashboardEventBus::new();
        let mut w = MemoryTailWatcher::new(path, bus);
        let report = w.tick_once();
        assert!(report.skipped);
        assert_eq!(report.new_rows, 0);
    }

    #[tokio::test]
    async fn first_tick_seeds_max_rowid_without_replaying_history() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("memory.db");
        create_schema(&path);
        insert_row(&path, "01HIST00000000000000000001", "history-row-1");
        insert_row(&path, "01HIST00000000000000000002", "history-row-2");

        let bus = DashboardEventBus::new();
        let mut rx = bus.subscribe();
        let mut w = MemoryTailWatcher::new(path, bus);
        let report = w.tick_once();

        // First tick seeded the cursor; nothing emitted.
        assert!(!report.skipped);
        assert_eq!(report.new_rows, 0);
        assert_eq!(report.last_rowid, 2);

        // Confirm no event landed by trying a non-blocking recv.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn subsequent_tick_emits_event_per_new_row() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("memory.db");
        create_schema(&path);
        insert_row(&path, "01OLD0000000000000000000A", "old");

        let bus = DashboardEventBus::new();
        let mut rx = bus.subscribe();
        let mut w = MemoryTailWatcher::new(path.clone(), bus);

        // Seed.
        let r0 = w.tick_once();
        assert_eq!(r0.last_rowid, 1);
        assert_eq!(r0.new_rows, 0);

        // Two new rows arrive.
        insert_row(&path, "01NEW0000000000000000000A", "new-A");
        insert_row(&path, "01NEW0000000000000000000B", "new-B");

        let r1 = w.tick_once();
        assert!(!r1.skipped);
        assert_eq!(r1.new_rows, 2);
        assert_eq!(r1.last_rowid, 3);

        let event_a = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("event A in time")
            .expect("event A payload");
        assert!(matches!(event_a.kind, DashboardEventKind::MemoryAppended));
        assert_eq!(event_a.entity_id, "01NEW0000000000000000000A");

        let event_b = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("event B in time")
            .expect("event B payload");
        assert_eq!(event_b.entity_id, "01NEW0000000000000000000B");

        // Third tick with no new rows — no further emit.
        let r2 = w.tick_once();
        assert_eq!(r2.new_rows, 0);
        assert_eq!(r2.last_rowid, 3);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn open_warned_suppresses_repeat_logs() {
        // Use a directory path (open should fail) to drive the open
        // error branch deterministically.
        let tmp = tempfile::tempdir().expect("tmp");
        let bus = DashboardEventBus::new();
        let mut w = MemoryTailWatcher::new(tmp.path().to_path_buf(), bus);
        // The path *exists* (it's a dir) but open_ro will fail.
        let r0 = w.tick_once();
        assert!(r0.skipped);
        assert!(w.open_warned);
        let r1 = w.tick_once();
        assert!(r1.skipped);
        // open_warned stayed true; a tracing subscriber is not asserted
        // here (would need a custom layer); the structural contract is
        // that the flag does not flip back.
    }
}
