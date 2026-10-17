//! Opt-in archive indexing with bounded discovery, decoding and durable projections.
//!
//! Enabling this worker explicitly starts a bounded historical baseline. It never
//! changes archive bytes. SQLite commits projections and frame-safe cursors together;
//! reopening restores the projection without decoding closed history again.
//! Deduplication covers retained logical events and checkpoint retries. A new
//! physical copy of an event already evicted from the bounded cache may reappear.
//! Fingerprints assume append-only source files; arbitrary edits preserving all
//! sampled bytes and filesystem identity/timestamps cannot be detected cheaply.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, Metadata, ReadDir};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use cortex_core::events::Envelope;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::archive_tail::{TailCheckpoint, TailLimits, TailReader};

const MAX_ENTRIES: usize = 256;
const MAX_READERS: usize = 8;
const MAX_PROJECTIONS: usize = 2000;
const MAX_PROJECTION_BYTES: usize = 64 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_COMPRESSED_BYTES: usize = 4 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINES: usize = 256;
const POLL_DURATION: Duration = Duration::from_millis(100);
const FINGERPRINT_BYTES: usize = 4096;

/// Index errors leave the last successful SQLite checkpoint usable.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveIndexError {
    /// Filesystem discovery or source access failed.
    #[error("archive index io: {0}")]
    Io(#[from] io::Error),
    /// SQLite failed; the current decoder is discarded before retrying.
    #[error("archive index sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A projection could not be encoded or decoded.
    #[error("archive index json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Bounded work and remaining durable state after one worker tick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArchivePollReport {
    /// Directory entries examined this tick, at most 256.
    pub discovery_entries: usize,
    /// A successful full discovery cycle has completed this tick.
    pub discovery_complete: bool,
    /// Initial historical discovery or processing has not completed.
    pub historical_backfill: bool,
    /// Files decoded this tick, at most eight.
    pub files_polled: usize,
    /// Complete lines processed, including replay, at most 256.
    pub events_processed: usize,
    /// Newly decoded valid envelopes, including non-projectable kinds.
    pub envelopes_parsed: usize,
    /// Newly committed projection rows, excluding duplicate retries.
    pub projections_written: usize,
    /// Compressed source and fingerprint bytes read this tick.
    pub bytes_read: usize,
    /// Bytes emitted by the zstd decoders this tick.
    pub bytes_decoded: usize,
    /// Files with remaining processing work known to the manifest.
    pub pending_files: u64,
    /// Files whose last observed EOF was inside a frame or line.
    pub partial_files: u64,
    /// Retained projection rows; capped at 2000 per archive root.
    pub retained_projections: u64,
    /// Bounded diagnostics, including persistent corrupt-source failures.
    pub errors: Vec<String>,
}

impl ArchivePollReport {
    fn error(&mut self, error: impl std::fmt::Display) {
        if self.errors.len() < 16 {
            self.errors
                .push(error.to_string().chars().take(512).collect());
        }
    }
}

/// Apply this module's additive schema, also when indexing is disabled.
pub fn apply_archive_index_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS archive_index_roots (
            root TEXT PRIMARY KEY, epoch INTEGER NOT NULL DEFAULT 0,
            baseline_complete INTEGER NOT NULL DEFAULT 0,
            pending_files INTEGER NOT NULL DEFAULT 0, partial_files INTEGER NOT NULL DEFAULT 0,
            baseline_files INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS archive_index_files (
            root TEXT NOT NULL, path TEXT NOT NULL, generation INTEGER NOT NULL,
            identity TEXT NOT NULL, size INTEGER NOT NULL, modified TEXT NOT NULL,
            prefix_len INTEGER NOT NULL DEFAULT 0, prefix_hash TEXT NOT NULL DEFAULT '',
            anchor_hash TEXT NOT NULL DEFAULT '', frame_start INTEGER NOT NULL DEFAULT 0,
            line_ordinal INTEGER NOT NULL DEFAULT 0, last_seen INTEGER NOT NULL,
            last_polled INTEGER NOT NULL DEFAULT 0, pending INTEGER NOT NULL DEFAULT 1,
            partial INTEGER NOT NULL DEFAULT 0, baseline INTEGER NOT NULL DEFAULT 1,
            error TEXT, PRIMARY KEY(root,path)
        );
        CREATE INDEX IF NOT EXISTS archive_index_pending
            ON archive_index_files(root,pending,last_polled);
        CREATE INDEX IF NOT EXISTS archive_index_seen
            ON archive_index_files(root,last_seen);
        CREATE INDEX IF NOT EXISTS archive_index_errors
            ON archive_index_files(root,error);
        CREATE TRIGGER IF NOT EXISTS archive_index_file_insert AFTER INSERT ON archive_index_files BEGIN
            UPDATE archive_index_roots SET pending_files=pending_files+NEW.pending,
                partial_files=partial_files+NEW.partial,baseline_files=baseline_files+NEW.baseline WHERE root=NEW.root;
        END;
        CREATE TRIGGER IF NOT EXISTS archive_index_file_update AFTER UPDATE ON archive_index_files BEGIN
            UPDATE archive_index_roots SET pending_files=pending_files+NEW.pending-OLD.pending,
                partial_files=partial_files+NEW.partial-OLD.partial,
                baseline_files=baseline_files+NEW.baseline-OLD.baseline WHERE root=NEW.root;
        END;
        CREATE TRIGGER IF NOT EXISTS archive_index_file_delete AFTER DELETE ON archive_index_files BEGIN
            UPDATE archive_index_roots SET pending_files=pending_files-OLD.pending,
                partial_files=partial_files-OLD.partial,baseline_files=baseline_files-OLD.baseline WHERE root=OLD.root;
        END;
        CREATE TABLE IF NOT EXISTS archive_index_projections (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT, root TEXT NOT NULL,
            path TEXT NOT NULL, generation INTEGER NOT NULL, event_id TEXT NOT NULL,
            body TEXT NOT NULL, UNIQUE(root,path,generation,event_id)
        );
        CREATE INDEX IF NOT EXISTS archive_index_projection_root
            ON archive_index_projections(root,sequence);
        CREATE INDEX IF NOT EXISTS archive_index_projection_event
            ON archive_index_projections(root,event_id,sequence);
        CREATE UNIQUE INDEX IF NOT EXISTS archive_index_logical_event
            ON archive_index_projections(root,event_id);
        CREATE TABLE IF NOT EXISTS archive_index_sources (
            root TEXT NOT NULL, path TEXT NOT NULL, generation INTEGER NOT NULL,
            event_id TEXT NOT NULL, PRIMARY KEY(root,path,generation,event_id)
        );
        CREATE INDEX IF NOT EXISTS archive_index_source_event
            ON archive_index_sources(root,event_id);
        CREATE TABLE IF NOT EXISTS archive_index_suppressed (
            event_id TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS archive_index_projection_versions (
            root TEXT PRIMARY KEY, version TEXT NOT NULL
        );",
    )
}

/// Permanently suppress an explicitly forgotten event across roots and replays.
/// Callers serialize publication and this operation to avoid stale RAM snapshots.
pub fn suppress_archive_event(conn: &Connection, event_id: &str) -> rusqlite::Result<()> {
    apply_archive_index_schema(conn)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO archive_index_suppressed(event_id) VALUES (?1)",
        [event_id],
    )?;
    tx.execute(
        "DELETE FROM archive_index_projections WHERE event_id=?1",
        [event_id],
    )?;
    tx.execute(
        "DELETE FROM archive_index_sources WHERE event_id=?1",
        [event_id],
    )?;
    tx.commit()
}

struct Directory {
    entries: ReadDir,
    depth: usize,
}

#[derive(Clone)]
struct Source {
    path: String,
    generation: i64,
    identity: String,
    size: u64,
    modified: String,
    prefix_len: usize,
    prefix_hash: String,
    anchor_hash: String,
    checkpoint: TailCheckpoint,
}

struct Stamp {
    identity: String,
    size: u64,
    modified: String,
}

/// Single-worker coordinator. Keep it alive between ticks to retain decoder state.
/// Directory traversal is metadata-only and restarts safely after a process restart.
pub struct ArchiveIndex {
    root: PathBuf,
    root_key: String,
    conn: Connection,
    directories: Vec<Directory>,
    discovery_started: bool,
    discovery_failed: bool,
    epoch: i64,
    cleanup_epoch: Option<i64>,
    baseline_complete: bool,
    readers: HashMap<String, TailReader>,
    reader_order: VecDeque<String>,
    poll_sequence: i64,
}

impl ArchiveIndex {
    /// Open a dedicated SQLite connection. No baseline work occurs until `poll`.
    /// The caller must explicitly opt in before creating or polling this worker.
    pub fn open(root: &Path, metadata_path: &Path) -> Result<Self, ArchiveIndexError> {
        let root = std::path::absolute(root)?;
        validate_components(&root)?;
        let root_key = root.to_string_lossy().into_owned();
        let conn = Connection::open(metadata_path)?;
        conn.busy_timeout(Duration::from_millis(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        apply_archive_index_schema(&conn)?;
        conn.execute(
            "INSERT OR IGNORE INTO archive_index_roots(root) VALUES (?1)",
            [&root_key],
        )?;
        let (epoch, baseline_complete) = conn.query_row(
            "SELECT epoch, baseline_complete FROM archive_index_roots WHERE root=?1",
            [&root_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let poll_sequence = conn.query_row(
            "SELECT COALESCE(MAX(last_polled),0) FROM archive_index_files WHERE root=?1",
            [&root_key],
            |row| row.get(0),
        )?;
        Ok(Self {
            root,
            root_key,
            conn,
            directories: Vec::new(),
            discovery_started: false,
            discovery_failed: false,
            epoch,
            cleanup_epoch: None,
            baseline_complete,
            readers: HashMap::new(),
            reader_order: VecDeque::new(),
            poll_sequence,
        })
    }

    /// Invalidate derived data once when the caller's projection contract changes.
    /// Source bytes and permanent suppression survive; subsequent `poll` calls
    /// rebuild from reset checkpoints within the ordinary decode/discovery limits.
    pub fn require_projection_version(&mut self, version: &str) -> Result<(), ArchiveIndexError> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT version FROM archive_index_projection_versions WHERE root=?1",
                [&self.root_key],
                |row| row.get(0),
            )
            .optional()?;
        if current.as_deref() == Some(version) {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM archive_index_projections WHERE root=?1",
            [&self.root_key],
        )?;
        tx.execute(
            "DELETE FROM archive_index_sources WHERE root=?1",
            [&self.root_key],
        )?;
        tx.execute(
            "UPDATE archive_index_files SET frame_start=0,line_ordinal=0,pending=1,
             partial=0,baseline=1,error=NULL WHERE root=?1",
            [&self.root_key],
        )?;
        tx.execute(
            "INSERT INTO archive_index_projection_versions(root,version) VALUES (?1,?2)
             ON CONFLICT(root) DO UPDATE SET version=excluded.version",
            params![self.root_key, version],
        )?;
        tx.commit()?;
        self.readers.clear();
        self.reader_order.clear();
        Ok(())
    }

    /// Discover and process bounded work. The 100 ms budget is cooperative;
    /// synchronous filesystem calls and the caller's projection callback can overrun it.
    pub fn poll<F>(&mut self, mut project: F) -> Result<ArchivePollReport, ArchiveIndexError>
    where
        F: FnMut(&Envelope) -> Option<Value>,
    {
        let started = Instant::now();
        let mut report = ArchivePollReport::default();
        // Split the soft budget so a large directory cannot starve known files.
        self.discover(started, &mut report)?;
        self.cleanup_missing(started, &mut report)?;
        let sources = self.pending_sources()?;
        for mut source in sources {
            if started.elapsed() >= POLL_DURATION
                || report.events_processed >= MAX_LINES
                || report.bytes_read + FINGERPRINT_BYTES * 4 >= MAX_COMPRESSED_BYTES
                || report.bytes_decoded >= MAX_DECODED_BYTES
            {
                break;
            }
            let path = self.root.join(&source.path);
            let metadata =
                match validate_components(&path).and_then(|()| fs::symlink_metadata(&path)) {
                    Ok(metadata) if metadata.is_file() => metadata,
                    Ok(_) => {
                        self.remove_source(&source.path)?;
                        report.error(format!("{}: source is no longer regular", source.path));
                        continue;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.remove_source(&source.path)?;
                        continue;
                    }
                    Err(error) => {
                        report.error(format!("{}: {error}", source.path));
                        self.advance_failed_source(&source.path, &error.to_string())?;
                        continue;
                    }
                };
            let stamp = stamp(&metadata);
            let changed = stamp.identity != source.identity
                || stamp.size < source.size
                || stamp.size < source.checkpoint.frame_start
                || (stamp.size == source.size && stamp.modified != source.modified);
            let append_ok = match (|| -> io::Result<bool> {
                if !changed && source.prefix_len > 0 && stamp.size > source.size {
                    let prefix = fingerprint(&path, 0, source.prefix_len, &mut report)?;
                    let anchor_len = source.size.min(FINGERPRINT_BYTES as u64) as usize;
                    let anchor = fingerprint(
                        &path,
                        source.size - anchor_len as u64,
                        anchor_len,
                        &mut report,
                    )?;
                    Ok(prefix == source.prefix_hash && anchor == source.anchor_hash)
                } else {
                    Ok(true)
                }
            })() {
                Ok(value) => value,
                Err(error) => {
                    report.error(format!("{}: {error}", source.path));
                    self.advance_failed_source(&source.path, &error.to_string())?;
                    continue;
                }
            };
            if changed || !append_ok {
                self.reset_source(&mut source, &stamp)?;
            }
            let prefix_len = stamp.size.min(FINGERPRINT_BYTES as u64) as usize;
            let (prefix_hash, anchor_hash) = match (|| -> io::Result<(String, String)> {
                if stamp.size != source.size || source.prefix_hash.is_empty() {
                    let prefix = fingerprint(&path, 0, prefix_len, &mut report)?;
                    let anchor_len = stamp.size.min(FINGERPRINT_BYTES as u64) as usize;
                    let anchor = fingerprint(
                        &path,
                        stamp.size - anchor_len as u64,
                        anchor_len,
                        &mut report,
                    )?;
                    Ok((prefix, anchor))
                } else {
                    Ok((source.prefix_hash.clone(), source.anchor_hash.clone()))
                }
            })() {
                Ok(value) => value,
                Err(error) => {
                    report.error(format!("{}: {error}", source.path));
                    self.advance_failed_source(&source.path, &error.to_string())?;
                    continue;
                }
            };
            let mut reader = match self.readers.remove(&source.path) {
                Some(reader) => reader,
                None => match TailReader::open(&path, source.checkpoint) {
                    Ok(reader) => reader,
                    Err(error) => {
                        report.error(format!("{}: {error}", source.path));
                        self.advance_failed_source(&source.path, &error.to_string())?;
                        continue;
                    }
                },
            };
            self.reader_order.retain(|path| path != &source.path);
            let batch = reader.poll(TailLimits {
                max_compressed_bytes: MAX_COMPRESSED_BYTES - report.bytes_read,
                max_decoded_bytes: MAX_DECODED_BYTES - report.bytes_decoded,
                max_lines: MAX_LINES - report.events_processed,
                max_line_bytes: 1024 * 1024,
                max_duration: POLL_DURATION.saturating_sub(started.elapsed()),
            });
            report.files_polled += 1;
            report.events_processed += batch.lines_processed;
            report.bytes_read += batch.bytes_read;
            report.bytes_decoded += batch.bytes_decoded;
            let mut projections = Vec::new();
            let mut projection_error = None;
            for line in &batch.lines {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                match serde_json::from_slice::<Envelope>(line) {
                    Ok(envelope) => {
                        report.envelopes_parsed += 1;
                        if envelope.event_id.is_empty() || envelope.event_id.len() > 256 {
                            report.error(format!("{}: invalid event identity", source.path));
                            projection_error = Some("invalid event identity".to_string());
                            continue;
                        }
                        if let Some(value) = project(&envelope) {
                            let body = serde_json::to_string(&value)?;
                            if body.len() <= MAX_PROJECTION_BYTES {
                                projections.push((envelope.event_id, body));
                            } else {
                                report.error(format!("{}: projection exceeds 64 KiB", source.path));
                                projection_error = Some("projection exceeds 64 KiB".to_string());
                            }
                        }
                    }
                    Err(_) => {
                        report.error(format!("{}: malformed envelope", source.path));
                        projection_error = Some("malformed envelope".to_string());
                    }
                }
            }
            let error = batch.error.as_ref().map(ToString::to_string);
            if let Some(error) = &error {
                report.error(format!("{}: {error}", source.path));
            }
            self.poll_sequence += 1;
            // The reader is outside the cache until commit succeeds. Every `?`
            // here discards speculative decoder state on a failed transaction.
            let tx = self.conn.transaction()?;
            let mut written = 0;
            for (event_id, body) in projections {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT body FROM archive_index_projections WHERE root=?1 AND event_id=?2",
                        params![self.root_key, event_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing.as_ref().is_some_and(|prior| prior != &body) {
                    report.error(format!(
                        "{}: conflicting projection for event {event_id}",
                        source.path
                    ));
                    projection_error = Some(format!("conflicting projection for event {event_id}"));
                    continue;
                }
                written += tx.execute(
                    "INSERT OR IGNORE INTO archive_index_projections(root,path,generation,event_id,body)
                     SELECT ?1,?2,?3,?4,?5 WHERE NOT EXISTS
                       (SELECT 1 FROM archive_index_suppressed WHERE event_id=?4)",
                    params![self.root_key, source.path, source.generation, event_id, body],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO archive_index_sources(root,path,generation,event_id)
                     SELECT ?1,?2,?3,?4 WHERE EXISTS
                       (SELECT 1 FROM archive_index_projections WHERE root=?1 AND event_id=?4)",
                    params![self.root_key, source.path, source.generation, event_id],
                )?;
            }
            let diagnostic = error.clone().or(projection_error);
            tx.execute(
                "UPDATE archive_index_files SET identity=?3,size=?4,modified=?5,prefix_len=?6,
                 prefix_hash=?7,anchor_hash=?8,frame_start=?9,line_ordinal=?10,last_polled=?11,
                 pending=?12,partial=?13,error=?14,baseline=CASE WHEN ?15 THEN 0 ELSE baseline END
                 WHERE root=?1 AND path=?2",
                params![
                    self.root_key,
                    source.path,
                    stamp.identity,
                    stamp.size as i64,
                    stamp.modified,
                    prefix_len as i64,
                    prefix_hash,
                    anchor_hash,
                    batch.checkpoint.frame_start as i64,
                    batch.checkpoint.line_ordinal as i64,
                    self.poll_sequence,
                    !batch.eof && error.is_none(),
                    batch.partial,
                    diagnostic,
                    batch.eof && !batch.replaying
                ],
            )?;
            tx.execute(
                "DELETE FROM archive_index_projections WHERE root=?1 AND sequence NOT IN
                 (SELECT sequence FROM archive_index_projections WHERE root=?1
                  ORDER BY sequence DESC LIMIT ?2)",
                params![self.root_key, MAX_PROJECTIONS as i64],
            )?;
            tx.execute(
                "DELETE FROM archive_index_sources WHERE root=?1 AND NOT EXISTS
                 (SELECT 1 FROM archive_index_projections p WHERE p.root=?1 AND p.event_id=archive_index_sources.event_id)",
                [&self.root_key],
            )?;
            // Source references are a bounded cache, not a second event archive.
            tx.execute(
                "DELETE FROM archive_index_sources WHERE root=?1 AND rowid NOT IN
                 (SELECT MAX(rowid) FROM archive_index_sources WHERE root=?1 GROUP BY event_id)
                 AND rowid NOT IN
                 (SELECT rowid FROM archive_index_sources WHERE root=?1 ORDER BY rowid DESC LIMIT 14000)",
                [&self.root_key],
            )?;
            tx.execute(
                "DELETE FROM archive_index_projections WHERE root=?1 AND NOT EXISTS
                 (SELECT 1 FROM archive_index_sources s WHERE s.root=?1 AND s.event_id=archive_index_projections.event_id)",
                [&self.root_key],
            )?;
            tx.commit()?;
            report.projections_written += written;
            if error.is_none() && (!batch.eof || batch.partial || batch.replaying) {
                while self.readers.len() >= MAX_READERS {
                    if let Some(oldest) = self.reader_order.pop_front() {
                        self.readers.remove(&oldest);
                    } else {
                        break;
                    }
                }
                self.reader_order.push_back(source.path.clone());
                self.readers.insert(source.path, reader);
            }
        }
        self.fill_report(&mut report)?;
        Ok(report)
    }

    /// Recent unique logical events, newest first, at most 2000 rows/eight MiB.
    pub fn snapshot(&self) -> Result<Vec<Value>, ArchiveIndexError> {
        let mut statement = self.conn.prepare(
            "SELECT p.body FROM archive_index_projections p WHERE p.root=?1
             AND NOT EXISTS (SELECT 1 FROM archive_index_suppressed s WHERE s.event_id=p.event_id)
             AND NOT EXISTS (SELECT 1 FROM archive_index_projections n
               WHERE n.root=p.root AND n.event_id=p.event_id AND n.sequence>p.sequence)
             ORDER BY p.sequence DESC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.root_key, MAX_PROJECTIONS as i64])?;
        let mut values = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let body: String = row.get(0)?;
            if bytes + body.len() > MAX_SNAPSHOT_BYTES {
                break;
            }
            bytes += body.len();
            values.push(serde_json::from_str(&body)?);
        }
        Ok(values)
    }

    /// Suppress an explicitly forgotten event without changing raw history.
    pub fn suppress_event(&self, event_id: &str) -> Result<(), ArchiveIndexError> {
        suppress_archive_event(&self.conn, event_id)?;
        Ok(())
    }

    fn discover(
        &mut self,
        started: Instant,
        report: &mut ArchivePollReport,
    ) -> Result<(), ArchiveIndexError> {
        if !self.discovery_started {
            match validate_components(&self.root).and_then(|()| fs::read_dir(&self.root)) {
                Ok(entries) => {
                    self.epoch += 1;
                    self.conn.execute(
                        "UPDATE archive_index_roots SET epoch=?2 WHERE root=?1",
                        params![self.root_key, self.epoch],
                    )?;
                    self.directories.push(Directory { entries, depth: 0 });
                    self.discovery_started = true;
                    self.discovery_failed = false;
                }
                Err(error) => {
                    report.error(format!("archive discovery: {error}"));
                    return Ok(());
                }
            }
        }
        while report.discovery_entries < MAX_ENTRIES && started.elapsed() < POLL_DURATION / 3 {
            let Some(directory) = self.directories.last_mut() else {
                break;
            };
            let depth = directory.depth;
            let entry = match directory.entries.next() {
                Some(Ok(entry)) => entry,
                Some(Err(error)) => {
                    report.discovery_entries += 1;
                    self.discovery_failed = true;
                    report.error(error);
                    continue;
                }
                None => {
                    self.directories.pop();
                    continue;
                }
            };
            report.discovery_entries += 1;
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.discovery_failed = true;
                    report.error(error);
                    continue;
                }
            };
            if is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                if entry.file_name() == "_quarantine" {
                    continue;
                }
                if depth >= 64 {
                    self.discovery_failed = true;
                    report.error("archive discovery: directory depth exceeds 64");
                } else {
                    match fs::read_dir(&path) {
                        Ok(entries) => self.directories.push(Directory {
                            entries,
                            depth: depth + 1,
                        }),
                        Err(error) => {
                            self.discovery_failed = true;
                            report.error(error);
                        }
                    }
                }
            } else if metadata.is_file()
                && path.extension().and_then(|s| s.to_str()) == Some("parquet")
            {
                let Some(relative) = path.strip_prefix(&self.root).ok().and_then(Path::to_str)
                else {
                    self.discovery_failed = true;
                    report.error("archive path is not representable as UTF-8");
                    continue;
                };
                let stamp = stamp(&metadata);
                self.conn.execute(
                    "INSERT INTO archive_index_files(root,path,generation,identity,size,modified,last_seen,baseline)
                     VALUES (?1,?2,1,?3,?4,?5,?6,?7)
                     ON CONFLICT(root,path) DO UPDATE SET last_seen=excluded.last_seen,
                     pending=CASE WHEN archive_index_files.identity!=excluded.identity
                        OR archive_index_files.size!=excluded.size OR archive_index_files.modified!=excluded.modified
                        THEN 1 ELSE archive_index_files.pending END",
                    params![self.root_key, relative, stamp.identity, stamp.size as i64, stamp.modified,
                        self.epoch, !self.baseline_complete],
                )?;
            }
        }
        if self.directories.is_empty() && self.discovery_started {
            self.discovery_started = false;
            if !self.discovery_failed {
                report.discovery_complete = true;
                self.cleanup_epoch = Some(self.epoch);
                self.baseline_complete = true;
                self.conn.execute(
                    "UPDATE archive_index_roots SET baseline_complete=1 WHERE root=?1",
                    [&self.root_key],
                )?;
            }
        }
        Ok(())
    }

    fn cleanup_missing(
        &mut self,
        started: Instant,
        report: &mut ArchivePollReport,
    ) -> Result<(), ArchiveIndexError> {
        let Some(epoch) = self.cleanup_epoch else {
            return Ok(());
        };
        let paths: Vec<String> = {
            let mut statement = self.conn.prepare(
                "SELECT path FROM archive_index_files WHERE root=?1 AND last_seen<?2 LIMIT 256",
            )?;
            let rows = statement.query_map(params![self.root_key, epoch], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let count = paths.len();
        let mut removed = 0;
        for path in paths {
            if started.elapsed() >= POLL_DURATION {
                break;
            }
            self.remove_source(&path)?;
            removed += 1;
        }
        if count < MAX_ENTRIES && removed == count {
            self.cleanup_epoch = None;
        }
        if self.cleanup_epoch.is_some() {
            report.discovery_complete = false;
        }
        Ok(())
    }

    fn pending_sources(&self) -> Result<Vec<Source>, ArchiveIndexError> {
        let active = serde_json::to_string(&self.reader_order)?;
        let mut sources = Vec::new();
        // An active frame must finish replay before eviction. Otherwise nine
        // large legacy frames can rotate forever without passing their cursors.
        let mut cached = self.conn.prepare(
            "SELECT path,generation,identity,size,modified,prefix_len,prefix_hash,anchor_hash,
             frame_start,line_ordinal FROM archive_index_files WHERE root=?1 AND path=?2 AND pending=1",
        )?;
        for path in &self.reader_order {
            if let Some(source) = cached
                .query_row(params![self.root_key, path], source_from_row)
                .optional()?
            {
                sources.push(source);
            }
        }
        if sources.len() < MAX_READERS {
            let mut statement = self.conn.prepare(
                "SELECT path,generation,identity,size,modified,prefix_len,prefix_hash,anchor_hash,
                 frame_start,line_ordinal FROM archive_index_files WHERE root=?1 AND pending=1
                 AND path NOT IN (SELECT value FROM json_each(?2)) ORDER BY last_polled LIMIT ?3",
            )?;
            let rows = statement.query_map(
                params![self.root_key, active, (MAX_READERS - sources.len()) as i64],
                source_from_row,
            )?;
            sources.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }
        Ok(sources)
    }

    fn remove_source(&mut self, path: &str) -> Result<(), ArchiveIndexError> {
        self.readers.remove(path);
        self.reader_order.retain(|item| item != path);
        let tx = self.conn.transaction()?;
        remove_source_projection(&tx, &self.root_key, path)?;
        tx.execute(
            "DELETE FROM archive_index_files WHERE root=?1 AND path=?2",
            params![self.root_key, path],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn reset_source(
        &mut self,
        source: &mut Source,
        stamp: &Stamp,
    ) -> Result<(), ArchiveIndexError> {
        self.readers.remove(&source.path);
        self.reader_order.retain(|path| path != &source.path);
        let tx = self.conn.transaction()?;
        remove_source_projection(&tx, &self.root_key, &source.path)?;
        tx.execute(
            "UPDATE archive_index_files SET generation=generation+1,identity=?3,size=?4,modified=?5,
             prefix_len=0,prefix_hash='',anchor_hash='',frame_start=0,line_ordinal=0,partial=0,error=NULL,pending=1
             WHERE root=?1 AND path=?2",
            params![self.root_key, source.path, stamp.identity, stamp.size as i64, stamp.modified],
        )?;
        tx.commit()?;
        source.generation += 1;
        source.identity.clone_from(&stamp.identity);
        source.size = stamp.size;
        source.modified.clone_from(&stamp.modified);
        source.prefix_len = 0;
        source.prefix_hash.clear();
        source.anchor_hash.clear();
        source.checkpoint = TailCheckpoint::default();
        Ok(())
    }

    fn advance_failed_source(&mut self, path: &str, error: &str) -> Result<(), ArchiveIndexError> {
        self.readers.remove(path);
        self.reader_order.retain(|cached| cached != path);
        self.poll_sequence += 1;
        self.conn.execute(
            "UPDATE archive_index_files SET last_polled=?3,error=?4 WHERE root=?1 AND path=?2",
            params![self.root_key, path, self.poll_sequence, error],
        )?;
        Ok(())
    }

    fn fill_report(&self, report: &mut ArchivePollReport) -> Result<(), ArchiveIndexError> {
        let (pending, partial, baseline): (i64, i64, i64) = self.conn.query_row(
            "SELECT pending_files,partial_files,baseline_files
             FROM archive_index_roots WHERE root=?1",
            [&self.root_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        report.pending_files = pending.max(0) as u64;
        report.partial_files = partial.max(0) as u64;
        report.historical_backfill = !self.baseline_complete || baseline > 0;
        let retained: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM archive_index_projections WHERE root=?1",
            [&self.root_key],
            |row| row.get(0),
        )?;
        report.retained_projections = retained.max(0) as u64;
        let mut statement = self.conn.prepare("SELECT path,error FROM archive_index_files WHERE root=?1 AND error IS NOT NULL LIMIT 16")?;
        let rows = statement.query_map([&self.root_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (path, error) = row?;
            report.error(format!("{path}: {error}"));
        }
        Ok(())
    }
}

fn source_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        path: row.get(0)?,
        generation: row.get(1)?,
        identity: row.get(2)?,
        size: row.get::<_, i64>(3)? as u64,
        modified: row.get(4)?,
        prefix_len: row.get::<_, i64>(5)? as usize,
        prefix_hash: row.get(6)?,
        anchor_hash: row.get(7)?,
        checkpoint: TailCheckpoint {
            frame_start: row.get::<_, i64>(8)? as u64,
            line_ordinal: row.get::<_, i64>(9)? as u64,
        },
    })
}

fn remove_source_projection(conn: &Connection, root: &str, path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM archive_index_sources WHERE root=?1 AND path=?2",
        params![root, path],
    )?;
    conn.execute(
        "DELETE FROM archive_index_projections WHERE root=?1 AND NOT EXISTS
         (SELECT 1 FROM archive_index_sources s WHERE s.root=?1 AND s.event_id=archive_index_projections.event_id)",
        [root],
    )?;
    Ok(())
}

fn fingerprint(
    path: &Path,
    offset: u64,
    length: usize,
    report: &mut ArchivePollReport,
) -> io::Result<String> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    let mut read = 0;
    while read < bytes.len() {
        let count = file.read(&mut bytes[read..])?;
        report.bytes_read += count;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "archive changed during fingerprint",
            ));
        }
        read += count;
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn stamp(metadata: &Metadata) -> Stamp {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", metadata.created().ok());
    Stamp {
        identity,
        size: metadata.len(),
        modified: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| time.as_nanos().to_string())
            .unwrap_or_default(),
    }
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn validate_components(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "archive path contains parent traversal",
            ));
        }
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "archive symlinks and reparse points are excluded",
                ))
            }
            Ok(_) => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn event(id: &str, text: &str) -> Envelope {
        serde_json::from_value(serde_json::json!({
            "event_id": id, "schema_version":"1", "occurred_at":"2020-01-01T00:00:00Z",
            "session_id":"prior-session", "stream":"live", "tool":"test", "kind":"turn",
            "context":{"repo":"fixture", "platform":"test", "extras":{}},
            "payload":{"user_message":text}, "content_hash":"fixture-hash"
        }))
        .unwrap()
    }

    fn project(envelope: &Envelope) -> Option<Value> {
        Some(serde_json::json!({"id":envelope.event_id,"payload":envelope.payload}))
    }

    fn append(path: &Path, events: &[Envelope]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut data = Vec::new();
        for event in events {
            serde_json::to_writer(&mut data, event).unwrap();
            data.push(b'\n');
        }
        let compressed = zstd::encode_all(data.as_slice(), 1).unwrap();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(&compressed).unwrap();
        file.sync_all().unwrap();
    }

    fn index(root: &Path) -> ArchiveIndex {
        ArchiveIndex::open(root, &root.join("metadata.sqlite")).unwrap()
    }

    fn drain(index: &mut ArchiveIndex) -> usize {
        let mut processed = 0;
        for _ in 0..1500 {
            let report = index.poll(project).unwrap();
            assert!(report.discovery_entries <= MAX_ENTRIES);
            assert!(report.events_processed <= MAX_LINES);
            assert!(report.bytes_read <= MAX_COMPRESSED_BYTES);
            assert!(report.bytes_decoded <= MAX_DECODED_BYTES);
            assert!(report.files_polled <= MAX_READERS);
            assert!(index.readers.len() <= MAX_READERS);
            assert!(report.retained_projections <= MAX_PROJECTIONS as u64);
            processed += report.envelopes_parsed;
            if report.discovery_complete && report.pending_files == 0 && !report.historical_backfill
            {
                return processed;
            }
        }
        panic!("bounded indexing did not finish finite input");
    }

    #[test]
    fn archive_index_projection_upgrade_is_bounded_durable_and_preserves_suppression() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events/raw.parquet");
        let events: Vec<_> = (0..600)
            .map(|i| event(&format!("event-{i}"), "projection upgrade"))
            .collect();
        append(&path, &events);
        let original = fs::read(&path).unwrap();
        let mut worker = index(directory.path());
        assert_eq!(drain(&mut worker), 600);
        worker.suppress_event("event-0").unwrap();
        worker.require_projection_version("classified-v1").unwrap();
        assert!(worker.snapshot().unwrap().is_empty());
        let first = worker.poll(project).unwrap();
        assert!(first.events_processed <= MAX_LINES);
        assert!(first.bytes_read <= MAX_COMPRESSED_BYTES);
        assert!(first.bytes_decoded <= MAX_DECODED_BYTES);
        assert!(first.historical_backfill);
        let partial = worker.snapshot().unwrap();
        drop(worker);
        let mut worker = index(directory.path());
        worker.require_projection_version("classified-v1").unwrap();
        assert_eq!(worker.snapshot().unwrap(), partial);
        drain(&mut worker);
        let hits = worker.snapshot().unwrap();
        assert_eq!(hits.len(), 599);
        assert!(hits.iter().all(|hit| hit["id"] != "event-0"));
        worker.require_projection_version("classified-v1").unwrap();
        let unchanged = worker.poll(project).unwrap();
        assert_eq!((unchanged.bytes_read, unchanged.bytes_decoded), (0, 0));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn archive_index_restart_and_append_skip_closed_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events/old/raw-00000.parquet");
        append(&path, &[event("first", "remember exact prior decision")]);
        let mut worker = index(directory.path());
        assert_eq!(drain(&mut worker), 1);
        let expected = worker.snapshot().unwrap();
        drop(worker);
        let mut worker = index(directory.path());
        assert_eq!(worker.snapshot().unwrap(), expected);
        let unchanged = worker.poll(project).unwrap();
        assert_eq!(unchanged.bytes_read, 0);
        assert_eq!(unchanged.bytes_decoded, 0);
        // Finish the discovery round that poll started: under load its time
        // budget can leave the round open past the append, and that round
        // already scanned the old partition.
        assert_eq!(drain(&mut worker), 0);
        append(
            &path,
            &[event("second", "late event in old timestamp partition")],
        );
        assert_eq!(drain(&mut worker), 1);
        assert_eq!(worker.snapshot().unwrap().len(), 2);
        let unchanged = worker.poll(project).unwrap();
        assert_eq!(unchanged.bytes_decoded, 0);
    }

    #[test]
    fn archive_index_failed_transaction_reopens_durable_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let events: Vec<_> = (0..400)
            .map(|i| event(&format!("event-{i}"), "atomic checkpoint"))
            .collect();
        append(&directory.path().join("raw.parquet"), &events);
        let mut worker = index(directory.path());
        worker.conn.execute_batch("CREATE TRIGGER reject_projection BEFORE INSERT ON archive_index_projections BEGIN SELECT RAISE(FAIL,'fixture failure'); END;").unwrap();
        let mut failed = false;
        for _ in 0..20 {
            if worker.poll(project).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(worker.readers.is_empty());
        assert!(worker.snapshot().unwrap().is_empty());
        worker
            .conn
            .execute_batch("DROP TRIGGER reject_projection;")
            .unwrap();
        assert_eq!(drain(&mut worker), 400);
        assert_eq!(worker.snapshot().unwrap().len(), 400);
    }

    #[test]
    fn archive_index_more_frames_than_reader_slots_eventually_finish() {
        let directory = tempfile::tempdir().unwrap();
        for file in 0..9 {
            let events: Vec<_> = (0..600)
                .map(|i| event(&format!("{file}-{i}"), "large legacy frame"))
                .collect();
            append(&directory.path().join(format!("{file}.parquet")), &events);
        }
        let mut worker = index(directory.path());
        assert_eq!(drain(&mut worker), 5400);
        assert_eq!(worker.snapshot().unwrap().len(), MAX_PROJECTIONS);
        assert_eq!(worker.poll(project).unwrap().bytes_decoded, 0);
    }

    #[test]
    fn archive_index_dedup_conflict_and_forget_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut worker = index(directory.path());
        append(
            &directory.path().join("first.parquet"),
            &[event("same-id", "original")],
        );
        drain(&mut worker);
        let original = worker.snapshot().unwrap();
        for i in 0..20 {
            append(
                &directory.path().join(format!("retry-{i}.parquet")),
                &[event("same-id", "original")],
            );
        }
        drain(&mut worker);
        assert_eq!(worker.snapshot().unwrap(), original);
        let count: i64 = worker
            .conn
            .query_row(
                "SELECT COUNT(*) FROM archive_index_projections",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        append(
            &directory.path().join("conflict.parquet"),
            &[event("same-id", "forged replacement")],
        );
        let mut saw_conflict = false;
        for _ in 0..30 {
            let report = worker.poll(project).unwrap();
            saw_conflict |= report
                .errors
                .iter()
                .any(|error| error.contains("conflicting projection"));
            if report.pending_files == 0 && report.discovery_complete {
                break;
            }
        }
        assert!(saw_conflict);
        assert_eq!(worker.snapshot().unwrap(), original);
        worker.suppress_event("same-id").unwrap();
        assert!(worker.snapshot().unwrap().is_empty());
        drop(worker);
        append(
            &directory.path().join("after-forget.parquet"),
            &[event("same-id", "original")],
        );
        let mut worker = index(directory.path());
        drain(&mut worker);
        assert!(worker.snapshot().unwrap().is_empty());
    }

    #[test]
    fn archive_index_replacement_and_deletion_invalidate_source_projection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.parquet");
        append(&path, &[event("old", &"old content ".repeat(500))]);
        let mut worker = index(directory.path());
        drain(&mut worker);
        File::create(&path).unwrap();
        append(&path, &[event("new", "new")]);
        drain(&mut worker);
        let snapshot = worker.snapshot().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["id"], "new");
        fs::remove_file(&path).unwrap();
        drain(&mut worker);
        assert!(worker.snapshot().unwrap().is_empty());
    }

    #[test]
    fn archive_index_duplicate_source_remains_when_first_source_disappears() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.parquet");
        let second = directory.path().join("second.parquet");
        append(&first, &[event("shared", "valid in either source")]);
        append(&second, &[event("shared", "valid in either source")]);
        let mut worker = index(directory.path());
        drain(&mut worker);
        fs::remove_file(first).unwrap();
        drain(&mut worker);
        assert_eq!(worker.snapshot().unwrap().len(), 1);
        fs::remove_file(second).unwrap();
        drain(&mut worker);
        assert!(worker.snapshot().unwrap().is_empty());
    }

    #[test]
    fn archive_index_discovery_is_bounded_and_quarantine_is_excluded() {
        let directory = tempfile::tempdir().unwrap();
        for i in 0..600 {
            fs::write(directory.path().join(format!("unrelated-{i}")), b"").unwrap();
        }
        append(
            &directory.path().join("events/raw.parquet"),
            &[event("visible", "live")],
        );
        append(
            &directory.path().join("events/_quarantine/raw.parquet"),
            &[event("quarantined", "hidden")],
        );
        let mut worker = index(directory.path());
        let first = worker.poll(project).unwrap();
        assert!(first.discovery_entries <= MAX_ENTRIES);
        assert!(!first.discovery_complete);
        drain(&mut worker);
        let snapshot = worker.snapshot().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["id"], "visible");
    }

    #[test]
    fn archive_index_missing_root_does_not_erase_committed_projection() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("archive");
        append(
            &root.join("raw.parquet"),
            &[event("keep", "temporary mount outage")],
        );
        let mut worker =
            ArchiveIndex::open(&root, &directory.path().join("metadata.sqlite")).unwrap();
        drain(&mut worker);
        fs::rename(&root, directory.path().join("unmounted")).unwrap();
        let report = worker.poll(project).unwrap();
        assert!(!report.errors.is_empty());
        assert_eq!(worker.snapshot().unwrap().len(), 1);
    }

    #[test]
    fn archive_index_oversized_projection_is_not_retained_and_other_events_progress() {
        let directory = tempfile::tempdir().unwrap();
        append(
            &directory.path().join("raw.parquet"),
            &[event("huge", "oversized"), event("small", "valid")],
        );
        let mut worker = index(directory.path());
        let mut saw_error = false;
        for _ in 0..30 {
            let report = worker
                .poll(|envelope| {
                    if envelope.event_id == "huge" {
                        Some(Value::String("x".repeat(MAX_PROJECTION_BYTES + 1)))
                    } else {
                        project(envelope)
                    }
                })
                .unwrap();
            saw_error |= report
                .errors
                .iter()
                .any(|error| error.contains("projection exceeds"));
            if report.pending_files == 0 && report.discovery_complete {
                break;
            }
        }
        assert!(saw_error);
        let snapshot = worker.snapshot().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["id"], "small");
    }

    #[test]
    fn archive_index_live_frame_resumes_after_restart_without_duplicate_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.parquet");
        let mut encoder =
            zstd::stream::write::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
        serde_json::to_writer(&mut encoder, &event("first", "before restart")).unwrap();
        encoder.write_all(b"\n").unwrap();
        encoder.flush().unwrap();
        let mut worker = index(directory.path());
        drain(&mut worker);
        assert_eq!(worker.poll(project).unwrap().partial_files, 1);
        drop(worker);
        serde_json::to_writer(&mut encoder, &event("second", "after restart")).unwrap();
        encoder.write_all(b"\n").unwrap();
        encoder.flush().unwrap();
        let mut worker = index(directory.path());
        assert_eq!(worker.snapshot().unwrap().len(), 1);
        assert_eq!(drain(&mut worker), 1);
        assert_eq!(worker.snapshot().unwrap().len(), 2);
        encoder.finish().unwrap();
        drain(&mut worker);
        assert_eq!(worker.poll(project).unwrap().partial_files, 0);
    }

    #[test]
    fn archive_index_corrupt_file_does_not_block_healthy_source() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("broken.parquet"), b"not a zstd frame").unwrap();
        append(
            &directory.path().join("valid.parquet"),
            &[event("healthy", "queryable despite corruption")],
        );
        let mut worker = index(directory.path());
        let mut saw_corruption = false;
        for _ in 0..30 {
            let report = worker.poll(project).unwrap();
            saw_corruption |= report.errors.iter().any(|error| error.contains("corrupt"));
            if report.pending_files == 0 && report.discovery_complete {
                break;
            }
        }
        assert!(saw_corruption);
        assert_eq!(worker.snapshot().unwrap().len(), 1);
        let unchanged = worker.poll(project).unwrap();
        assert_eq!(unchanged.bytes_decoded, 0);
        assert!(unchanged
            .errors
            .iter()
            .any(|error| error.contains("corrupt")));
    }

    #[test]
    fn archive_index_reference_retention_preserves_one_source_per_logical_event() {
        let directory = tempfile::tempdir().unwrap();
        append(
            &directory.path().join("first.parquet"),
            &[
                event("unique", "old unique"),
                event("common", "many copies"),
            ],
        );
        let mut worker = index(directory.path());
        drain(&mut worker);
        {
            let tx = worker.conn.transaction().unwrap();
            for i in 0..16010 {
                tx.execute("INSERT INTO archive_index_sources(root,path,generation,event_id) VALUES (?1,?2,1,'common')",
                    params![worker.root_key,format!("copy-{i}")]).unwrap();
            }
            tx.commit().unwrap();
        }
        append(
            &directory.path().join("new.parquet"),
            &[event("new", "trigger retention")],
        );
        drain(&mut worker);
        worker.remove_source("nonexistent.parquet").unwrap();
        assert_eq!(worker.snapshot().unwrap().len(), 3);
        let references: i64 = worker
            .conn
            .query_row("SELECT COUNT(*) FROM archive_index_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(references <= 16000);
    }
}
