//! Durable archive writer.
//!
//! Writes one zstd-compressed NDJSON file per hour per stream tag under
//! the partition layout declared in `cortex_storage::archive` — so spec 02
//! owns the *where*, and this module owns the *how*. Format: one event per
//! line; each line is the full envelope as JSON plus an `_archived_at`
//! timestamp appended by the writer.
//!
//! NDJSON + Zstd is the MVP format; the spec contemplates Parquet once the
//! query-side DuckDB tooling lands. The path + naming convention already
//! match the Parquet layout so migration is a codec swap.

use chrono::{DateTime, Utc};
use cortex_storage::{archive_filename, archive_partition};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Errors returned by [`ArchiveWriter`].
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// Filesystem or underlying I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization error.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Internal bug (poisoned mutex, etc.).
    #[error("internal: {0}")]
    Internal(String),
}

/// Abstract archive writer used by the HTTP router.
pub trait ArchiveWriter: Send + Sync + 'static {
    /// Persist a single event envelope. Must not return until the bytes
    /// are guaranteed durable (flushed + fsynced in production; flushed
    /// in dev).
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<(), ArchiveError>;
}

/// Zstd-compressed NDJSON archive. Rotates hourly per stream tag.
pub struct NdJsonZstdArchive {
    root: PathBuf,
    level: i32,
    /// `(stream_tag, hour_bucket) -> open writer`.
    open: Mutex<std::collections::HashMap<(String, String), ArchiveFile>>,
}

struct ArchiveFile {
    path: PathBuf,
    encoder: zstd::stream::write::AutoFinishEncoder<'static, BufWriter<File>>,
}

impl NdJsonZstdArchive {
    /// Create a new archive rooted at `root`.
    pub fn new(root: PathBuf, level: i32) -> Self {
        Self {
            root,
            level,
            open: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn ensure_open(
        &self,
        stream_tag: &str,
        ts: DateTime<Utc>,
    ) -> Result<(), ArchiveError> {
        let bucket = format!("{}", ts.format("%Y%m%d%H"));
        let mut open = self
            .open
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?;
        let key = (stream_tag.to_string(), bucket);
        if open.contains_key(&key) {
            return Ok(());
        }
        let dir = archive_partition(&self.root, ts);
        std::fs::create_dir_all(&dir)?;
        let filename = archive_filename(stream_tag, 0);
        let path = dir.join(&filename);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let encoder = zstd::Encoder::new(BufWriter::new(file), self.level)?.auto_finish();
        open.insert(key, ArchiveFile { path, encoder });
        Ok(())
    }

    /// Flush every currently-open file. Use before graceful shutdown.
    pub fn flush(&self) -> Result<(), ArchiveError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?;
        for af in open.values_mut() {
            af.encoder.flush()?;
        }
        Ok(())
    }

    /// Return the currently-open file paths — test introspection only.
    pub fn open_paths(&self) -> Vec<PathBuf> {
        match self.open.lock() {
            Ok(g) => g.values().map(|a| a.path.clone()).collect(),
            Err(_) => vec![],
        }
    }
}

impl ArchiveWriter for NdJsonZstdArchive {
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<(), ArchiveError> {
        let occurred_at = envelope
            .get("occurred_at")
            .and_then(|s| s.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        self.ensure_open(stream_tag, occurred_at)?;

        let mut owned = envelope.clone();
        if let Value::Object(map) = &mut owned {
            map.insert(
                "_archived_at".into(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }
        let line = serde_json::to_string(&owned)?;

        let bucket = format!("{}", occurred_at.format("%Y%m%d%H"));
        let key = (stream_tag.to_string(), bucket);
        let mut open = self
            .open
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?;
        let af = open
            .get_mut(&key)
            .ok_or_else(|| ArchiveError::Internal("archive file disappeared".into()))?;
        af.encoder.write_all(line.as_bytes())?;
        af.encoder.write_all(b"\n")?;
        af.encoder.flush()?;
        Ok(())
    }
}

/// Archive implementation that keeps every line in memory — tests only.
#[derive(Default)]
pub struct InMemoryArchive {
    rows: Mutex<Vec<(String, Value)>>,
}

impl InMemoryArchive {
    /// Snapshot the stored rows.
    pub fn rows(&self) -> Vec<(String, Value)> {
        self.rows.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl ArchiveWriter for InMemoryArchive {
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<(), ArchiveError> {
        self.rows
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?
            .push((stream_tag.to_string(), envelope.clone()));
        Ok(())
    }
}

/// Read all events from a given archive file (test / diagnostic helper).
pub fn read_archive_file(path: &Path) -> Result<Vec<Value>, ArchiveError> {
    let raw = std::fs::read(path)?;
    let decoded = zstd::decode_all(raw.as_slice())?;
    let text = String::from_utf8(decoded)
        .map_err(|e| ArchiveError::Internal(format!("utf8: {e}")))?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}

