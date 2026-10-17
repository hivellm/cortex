//! Durable archive writer.
//!
//! Writes zstd-compressed NDJSON files grouped by hour and stream tag under
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
use std::io::Write;
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
    /// are guaranteed durable (complete frame, flushed and synced to disk).
    ///
    /// ADR-012 — returns the partition path the envelope was written
    /// to so the ingestion router can stamp `event_identity.archive_partition`
    /// without re-deriving the path. In-memory test impls return a
    /// synthetic `PathBuf` keyed by `stream_tag` so identity-index
    /// integration tests can still assert the round-trip.
    fn write(&self, stream_tag: &str, envelope: &Value) -> Result<PathBuf, ArchiveError>;
}

/// Zstd-compressed NDJSON archive. Rotates hourly per stream tag.
/// Each successful write ends a complete independently compressed frame.
pub struct NdJsonZstdArchive {
    root: PathBuf,
    level: i32,
    /// `(stream_tag, hour_bucket) -> open file`, bounded across old partitions.
    open: Mutex<std::collections::HashMap<(String, String), ArchiveFile>>,
}

const MAX_OPEN_ARCHIVE_FILES: usize = 32;

struct ArchiveFile {
    path: PathBuf,
    file: File,
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
        open: &mut std::collections::HashMap<(String, String), ArchiveFile>,
    ) -> Result<(), ArchiveError> {
        let bucket = format!("{}", ts.format("%Y%m%d%H"));
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
        // A new handle never reopens an old tail, including after eviction or
        // an I/O failure. create_new also arbitrates independent processes.
        let mut sequence = next_free_sequence(&dir, stream_tag)?;
        let (path, file) = loop {
            let path = dir.join(archive_filename(stream_tag, sequence));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    sequence = sequence
                        .checked_add(1)
                        .ok_or_else(|| std::io::Error::other("archive sequence exhausted"))?;
                }
                Err(error) => return Err(error.into()),
            }
        };
        if open.len() >= MAX_OPEN_ARCHIVE_FILES {
            // Every cached file already contains only durable completed
            // frames, so closing an old hour has no buffered work to lose.
            if let Some(oldest) = open.keys().min_by(|a, b| a.1.cmp(&b.1)).cloned() {
                open.remove(&oldest);
            }
        }
        open.insert(key, ArchiveFile { path, file });
        Ok(())
    }

    /// Flush every currently-open file. Use before graceful shutdown.
    pub fn flush(&self) -> Result<(), ArchiveError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?;
        let keys: Vec<_> = open.keys().cloned().collect();
        for key in keys {
            let result = open
                .get_mut(&key)
                .ok_or_else(|| ArchiveError::Internal("archive file disappeared".into()))?
                .file
                .sync_data();
            if let Err(error) = result {
                open.remove(&key);
                return Err(error.into());
            }
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

        let mut owned = envelope.clone();
        if let Value::Object(map) = &mut owned {
            map.insert(
                "_archived_at".into(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }
        let mut line = serde_json::to_vec(&owned)?;
        line.push(b'\n');
        // Finish compression before touching the file: no encoder lives
        // across calls, and codec/finalization errors are returned explicitly.
        let frame = zstd::encode_all(line.as_slice(), self.level)?;

        let bucket = format!("{}", occurred_at.format("%Y%m%d%H"));
        let key = (stream_tag.to_string(), bucket);
        let mut open = self
            .open
            .lock()
            .map_err(|_| ArchiveError::Internal("archive mutex poisoned".into()))?;
        self.ensure_open(stream_tag, occurred_at, &mut open)?;
        let af = open
            .get_mut(&key)
            .ok_or_else(|| ArchiveError::Internal("archive file disappeared".into()))?;
        let result = af
            .file
            .write_all(&frame)
            .and_then(|()| af.file.flush())
            .and_then(|()| af.file.sync_data());
        if let Err(error) = result {
            // A partial write has an unknown tail. Keep its bytes for
            // recovery, but never append another frame to that file.
            open.remove(&key);
            return Err(error.into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Read;
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

        archive
            .write("raw", &serde_json::json!({"occurred_at": ts.to_rfc3339()}))
            .unwrap();
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

    #[test]
    fn successful_writes_finish_each_frame_before_flush_or_drop() {
        let tmp = TempDir::new().unwrap();
        let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 1);
        let first = json!({"event_id":"first", "occurred_at":"2026-04-28T22:00:00Z"});
        let second = json!({"event_id":"second", "occurred_at":"2026-04-28T22:01:00Z"});
        let path = archive.write("raw", &first).unwrap();
        let rows = read_archive_file(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["event_id"], "first");
        assert!(rows[0]["_archived_at"].is_string());

        assert_eq!(archive.write("raw", &second).unwrap(), path);
        assert_eq!(read_archive_file(&path).unwrap().len(), 2);
        let bytes = std::fs::read(&path).unwrap();
        let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes.as_slice())
            .unwrap()
            .single_frame();
        let mut first_frame = String::new();
        decoder.read_to_string(&mut first_frame).unwrap();
        let remaining = decoder.finish();
        assert!(!remaining.is_empty(), "second envelope needs its own frame");
        assert_eq!(
            serde_json::from_str::<Value>(first_frame.trim()).unwrap()["event_id"],
            "first"
        );
        let second_frame = zstd::decode_all(remaining).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&second_frame).unwrap()["event_id"],
            "second"
        );
        archive.flush().unwrap();
        assert_eq!(archive.open_paths(), vec![path.clone()]);
        assert_eq!(archive.write("raw", &second).unwrap(), path);
    }

    #[test]
    fn old_hour_eviction_bounds_handles_and_reopens_clean_sequence() {
        let tmp = TempDir::new().unwrap();
        let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 1);
        let start = DateTime::parse_from_rfc3339("2026-04-28T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut paths = Vec::new();
        for index in 0..MAX_OPEN_ARCHIVE_FILES + 3 {
            let ts = start + chrono::Duration::hours(index as i64);
            paths.push(
                archive
                    .write(
                        "raw",
                        &json!({"event_id":index, "occurred_at":ts.to_rfc3339()}),
                    )
                    .unwrap(),
            );
            assert!(archive.open_paths().len() <= MAX_OPEN_ARCHIVE_FILES);
        }
        assert_eq!(archive.open_paths().len(), MAX_OPEN_ARCHIVE_FILES);
        assert!(!archive.open_paths().contains(&paths[0]));
        let reopened = archive
            .write(
                "raw",
                &json!({"event_id":"reopened", "occurred_at":start.to_rfc3339()}),
            )
            .unwrap();
        assert_ne!(reopened, paths[0]);
        paths.push(reopened);
        for path in paths {
            assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        }
        assert_eq!(archive.open_paths().len(), MAX_OPEN_ARCHIVE_FILES);
    }

    #[test]
    fn failed_write_retires_handle_and_preserves_previous_complete_frames() {
        let tmp = TempDir::new().unwrap();
        let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 1);
        let first = json!({"event_id":"first", "occurred_at":"2026-04-28T22:00:00Z"});
        let path = archive.write("raw", &first).unwrap();
        {
            let mut open = archive.open.lock().unwrap();
            let entry = open.values_mut().next().unwrap();
            entry.file = File::open(&path).unwrap();
        }
        assert!(matches!(
            archive.write("raw", &first),
            Err(ArchiveError::Io(_))
        ));
        assert!(archive.open_paths().is_empty());
        let reopened = archive.write("raw", &first).unwrap();
        assert_ne!(reopened, path);
        assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        assert_eq!(read_archive_file(&reopened).unwrap().len(), 1);
    }

    #[test]
    fn independent_writers_never_share_a_new_file() {
        let tmp = TempDir::new().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|index| {
                let root = tmp.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let archive = NdJsonZstdArchive::new(root, 1);
                    barrier.wait();
                    archive
                        .write(
                            "raw",
                            &json!({"event_id":index, "occurred_at":"2026-04-28T22:00:00Z"}),
                        )
                        .unwrap()
                })
            })
            .collect();
        let paths: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_ne!(paths[0], paths[1]);
        for path in paths {
            assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        }
    }

    #[test]
    fn archive_directory_error_is_returned_without_open_handle() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("not-a-directory");
        std::fs::write(&root, b"preserve").unwrap();
        let archive = NdJsonZstdArchive::new(root.clone(), 1);
        assert!(matches!(
            archive.write("raw", &json!({})),
            Err(ArchiveError::Io(_))
        ));
        assert!(archive.open_paths().is_empty());
        assert_eq!(std::fs::read(root).unwrap(), b"preserve");
    }
}
