//! Append-only overflow WAL.
//!
//! Spec 10 §Asynchronous publisher: when `cortex-core` is unreachable
//! after retries, dropped events spill to `~/.cortex/overflow.wal`.
//! On daemon startup the WAL is replayed; the file is recreated empty
//! after a successful drain so it never grows unbounded.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use thiserror::Error;

/// Failure modes raised by the WAL.
#[derive(Debug, Error)]
pub enum WalError {
    /// Filesystem failure.
    #[error("wal io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON encode/decode failure.
    #[error("wal json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Append-only WAL backed by a single newline-delimited JSON file.
#[derive(Debug)]
pub struct OverflowWal {
    path: PathBuf,
    handle: Mutex<Option<File>>,
}

impl OverflowWal {
    /// Open or create the WAL at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, WalError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        Ok(Self {
            path,
            handle: Mutex::new(Some(f)),
        })
    }

    /// Path the WAL persists at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one envelope JSON. Each call flushes — at-least-once
    /// durability when the daemon process dies between calls.
    pub fn append(&self, value: &Value) -> Result<(), WalError> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        let mut guard = self
            .handle
            .lock()
            .expect("wal mutex poisoned");
        let file = guard
            .as_mut()
            .ok_or_else(|| WalError::Io(std::io::Error::other("wal closed")))?;
        file.write_all(&line)?;
        file.sync_data()?;
        Ok(())
    }

    /// Current on-disk size in bytes — surfaced via the
    /// `overflow.wal_bytes` gauge.
    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    /// Read every persisted envelope into memory and truncate the
    /// file. Used at daemon startup to replay drops from the previous
    /// run.
    pub fn drain(&self) -> Result<Vec<Value>, WalError> {
        let mut guard = self
            .handle
            .lock()
            .expect("wal mutex poisoned");
        // Drop the live handle so we can reopen for read+truncate.
        guard.take();

        let mut entries: Vec<Value> = Vec::new();
        if self.path.exists() {
            let f = File::open(&self.path)?;
            for line in BufReader::new(f).lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(v) => entries.push(v),
                    Err(e) => {
                        tracing::warn!(error = %e, "wal entry parse failed; skipping");
                    }
                }
            }
        }
        // Truncate atomically by rewriting an empty file.
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        f.sync_all()?;
        // Reopen append handle.
        let appender = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&self.path)?;
        *guard = Some(appender);
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appends_grow_the_file_and_drain_returns_entries_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let wal = OverflowWal::open(tmp.path().join("overflow.wal")).unwrap();
        wal.append(&json!({ "event_id": "1" })).unwrap();
        wal.append(&json!({ "event_id": "2" })).unwrap();
        assert!(wal.size_bytes() > 0);
        let drained = wal.drain().unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0]["event_id"], "1");
        assert_eq!(drained[1]["event_id"], "2");
        assert_eq!(wal.size_bytes(), 0);
    }

    #[test]
    fn drain_on_empty_wal_returns_empty_vec() {
        let tmp = tempfile::tempdir().unwrap();
        let wal = OverflowWal::open(tmp.path().join("overflow.wal")).unwrap();
        let drained = wal.drain().unwrap();
        assert!(drained.is_empty());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("overflow.wal");
        std::fs::write(&path, "{\"event_id\":\"good\"}\nnot-json\n{\"event_id\":\"after\"}\n").unwrap();
        let wal = OverflowWal::open(&path).unwrap();
        let drained = wal.drain().unwrap();
        let ids: Vec<&str> = drained.iter().map(|v| v["event_id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["good", "after"]);
    }
}
