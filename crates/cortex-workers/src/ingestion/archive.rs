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
    ///
    /// ADR-012 — returns the partition path the envelope was written
    /// to so the ingestion router can stamp `event_identity.archive_partition`
    /// without re-deriving the path. In-memory test impls return a
    /// synthetic `PathBuf` keyed by `stream_tag` so identity-index
    /// integration tests can still assert the round-trip.
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<PathBuf, ArchiveError>;
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

    fn ensure_open(&self, stream_tag: &str, ts: DateTime<Utc>) -> Result<(), ArchiveError> {
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
        // Phase8a / 2026-04-28 incident: the writer used to always
        // pick sequence `0`, so a hard-killed previous run left a
        // half-flushed zstd tail on `raw-00000.parquet` that the
        // next run silently appended onto. The next archive_loader
        // refresh would then see "Data corruption detected" and
        // truncate the recovered set at the boundary — silent data
        // loss until the operator manually rotated the file.
        //
        // Pick the next free sequence number so a fresh process
        // always opens a clean file. Graceful shutdown still flushes
        // the encoder cleanly into whatever sequence is currently
        // open; only an abrupt termination strands the file, and
        // the next run side-steps it instead of corrupting it.
        let sequence = next_free_sequence(&dir, stream_tag)?;
        let filename = archive_filename(stream_tag, sequence);
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

/// Pick the next free sequence number for `<stream>-NNNNN.parquet`
/// inside `dir`. `next_free_sequence(dir, "raw")` returns `0` on a
/// virgin partition; on a partition that already contains
/// `raw-00000.parquet` and `raw-00002.parquet`, it returns `3`
/// (always `max(existing) + 1` so a re-opening process never
/// overwrites a sibling).
///
/// Files that don't match the canonical `<stream>-NNNNN.parquet`
/// shape are ignored — including the post-incident
/// `raw-00000.parquet.corrupted-*` quarantine names produced by the
/// 2026-04-28 manual recovery.
fn next_free_sequence(dir: &Path, stream_tag: &str) -> Result<u32, ArchiveError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let prefix = format!("{stream_tag}-");
    let suffix = ".parquet";
    let mut highest: Option<u32> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        let middle = match s.strip_prefix(&prefix).and_then(|t| t.strip_suffix(suffix)) {
            Some(m) => m,
            None => continue,
        };
        // `middle` must be exactly the 5-digit zero-padded sequence
        // with no extra suffix — otherwise we'd accidentally include
        // operator-quarantined `raw-00000.parquet.corrupted-1907`
        // entries and pick a sequence past one of those.
        if middle.len() != 5 || !middle.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(n) = middle.parse::<u32>() {
            highest = Some(highest.map_or(n, |cur| cur.max(n)));
        }
    }
    Ok(highest.map_or(0, |n| n.saturating_add(1)))
}

impl ArchiveWriter for NdJsonZstdArchive {
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<PathBuf, ArchiveError> {
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
        // ADR-012 — surface the partition path so the router can
        // stamp `event_identity.archive_partition` on the upsert.
        Ok(af.path.clone())
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
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<PathBuf, ArchiveError> {
        self.rows
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?
            .push((stream_tag.to_string(), envelope.clone()));
        // ADR-012 — synthetic partition path keyed by stream tag.
        // Real callers expect a `PathBuf`; the in-memory archive
        // has no on-disk file so the deterministic `mem://<tag>`
        // form lets identity-index integration tests assert the
        // upsert without colliding with NdJsonZstd paths.
        Ok(PathBuf::from(format!("mem://{stream_tag}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"").unwrap();
    }

    #[test]
    fn next_free_sequence_is_zero_on_empty_or_missing_dir() {
        let tmp = TempDir::new().unwrap();
        // Existing but empty directory.
        assert_eq!(next_free_sequence(tmp.path(), "raw").unwrap(), 0);
        // Non-existent directory.
        assert_eq!(
            next_free_sequence(&tmp.path().join("missing"), "raw").unwrap(),
            0,
        );
    }

    #[test]
    fn next_free_sequence_picks_one_past_highest_existing() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "raw-00000.parquet");
        touch(tmp.path(), "raw-00002.parquet");
        // Different stream — must NOT contribute.
        touch(tmp.path(), "bootstrap-00010.parquet");
        assert_eq!(next_free_sequence(tmp.path(), "raw").unwrap(), 3);
    }

    #[test]
    fn next_free_sequence_ignores_quarantined_corrupted_names() {
        let tmp = TempDir::new().unwrap();
        // Operator-quarantined names from the 2026-04-28 incident.
        touch(tmp.path(), "raw-00000.parquet.corrupted-1907");
        touch(tmp.path(), "raw-00001.parquet.corrupted-2113");
        // No canonical files yet → next seq is 0.
        assert_eq!(next_free_sequence(tmp.path(), "raw").unwrap(), 0);
        // After a clean run lands raw-00000.parquet, the next is 1
        // — quarantined sidecars never bump the sequence.
        touch(tmp.path(), "raw-00000.parquet");
        assert_eq!(next_free_sequence(tmp.path(), "raw").unwrap(), 1);
    }

    #[test]
    fn next_free_sequence_skips_malformed_filenames() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "raw-abc.parquet");
        touch(tmp.path(), "raw-1.parquet");
        touch(tmp.path(), "raw-000000.parquet");
        touch(tmp.path(), "raw-00007.parquet");
        // Only `raw-00007.parquet` matches the 5-digit canonical
        // shape; everything else is ignored.
        assert_eq!(next_free_sequence(tmp.path(), "raw").unwrap(), 8);
    }

    #[test]
    fn ensure_open_picks_fresh_file_when_zero_already_exists() {
        let tmp = TempDir::new().unwrap();
        let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 1);
        let ts = chrono::DateTime::parse_from_rfc3339("2026-04-28T22:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Pre-seed a corrupt-tail leftover from a previous run.
        let dir = archive_partition(tmp.path(), ts);
        std::fs::create_dir_all(&dir).unwrap();
        let preexisting = dir.join("raw-00000.parquet");
        std::fs::write(&preexisting, b"\xDEAD_TAIL_BYTES").unwrap();

        archive.ensure_open("raw", ts).unwrap();
        let opened = archive.open_paths();
        assert_eq!(opened.len(), 1);
        let chosen = &opened[0];
        // The chosen file MUST NOT be the pre-existing corrupt
        // sibling — it must be a sibling at the next free index.
        assert_ne!(chosen, &preexisting);
        assert!(
            chosen
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("00001"),
            "expected sequence-1 file, got {}",
            chosen.display(),
        );
        // The pre-existing corrupt sibling stays on disk untouched
        // — operators decide what to do with it.
        assert!(preexisting.exists());
    }
}

/// Read all events from a given archive file (test / diagnostic helper).
pub fn read_archive_file(path: &Path) -> Result<Vec<Value>, ArchiveError> {
    let raw = std::fs::read(path)?;
    let decoded = zstd::decode_all(raw.as_slice())?;
    let text =
        String::from_utf8(decoded).map_err(|e| ArchiveError::Internal(format!("utf8: {e}")))?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}
