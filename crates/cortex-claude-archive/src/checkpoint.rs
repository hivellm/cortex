//! Atomic resume checkpoint — phase11i §1.6.
//!
//! Tracks `(project_dir, session_id, last_record_uuid,
//! last_byte_offset)` per ingested file so a crashed bootstrap can
//! pick up where it left off. The CLI writes this every 5 s
//! during a long bootstrap; the watcher daemon writes on every
//! flush.
//!
//! Storage is a single JSON file at the configured path, written
//! via the temp-file + rename atomic pattern. Concurrent writers
//! are not supported (the daemon is single-process); concurrent
//! READS are fine — the file is replaced atomically so partial
//! reads cannot observe a half-written body.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One per (project_dir, session_id) pair. The CLI / watcher
/// updates this each time it advances past a record; on resume
/// the reader fast-forwards to the next byte after
/// `last_byte_offset`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    /// Stable session id. Always matches `Envelope.session_id`.
    pub session_id: String,
    /// Last consumed UUID — the next record's `parentUuid` chain
    /// re-resolves around this anchor.
    pub last_record_uuid: String,
    /// Byte offset into the JSONL file. The reader seeks here on
    /// resume; the next line consumed is the one starting at
    /// `last_byte_offset`.
    pub last_byte_offset: u64,
    /// Epoch milliseconds at the moment of the write. Lets the
    /// admin CLI report "checkpoint is N hours stale".
    pub written_at_ms: i64,
}

/// In-memory map of `(project_dir, session_path) → Checkpoint`.
/// Keys are joined `<project_dir>::<session_filename>` so the
/// CLI can resume per-file (one project may carry hundreds of
/// `.jsonl` files).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CheckpointStoreState {
    /// Schema version. Bumped on incompatible changes. Old
    /// checkpoint files with a different version are ignored
    /// (treated as "no checkpoint") rather than blocking the
    /// daemon.
    pub schema: u32,
    /// One entry per per-session file. Key format:
    /// `<project_dir>::<session_filename>`.
    pub entries: BTreeMap<String, Checkpoint>,
}

/// Current schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Errors raised by the checkpoint store.
#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("io error on checkpoint {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("malformed checkpoint at {path}: {source}")]
    Malformed {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Disk-backed checkpoint store. The `path` is held by the
/// struct; `load` reads it on construction (or returns an empty
/// state when the file does not exist), and `save_atomic` writes
/// via temp-file + rename so a crash mid-write never leaves the
/// file half-written.
#[derive(Debug, Clone)]
pub struct CheckpointStore {
    path: PathBuf,
    state: CheckpointStoreState,
}

impl CheckpointStore {
    /// Open or create the store at `path`. Missing files yield an
    /// empty state with the current schema. Malformed files
    /// surface `CheckpointError::Malformed`; the caller can
    /// either log + reset or abort.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, CheckpointError> {
        let path: PathBuf = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                state: CheckpointStoreState {
                    schema: SCHEMA_VERSION,
                    entries: BTreeMap::new(),
                },
            });
        }
        let raw = fs::read(&path).map_err(|source| CheckpointError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let state: CheckpointStoreState =
            serde_json::from_slice(&raw).map_err(|source| CheckpointError::Malformed {
                path: path.display().to_string(),
                source,
            })?;
        // Forward-incompatible schema bumps reset the store
        // instead of crashing the daemon — old checkpoints become
        // a re-bootstrap, never a hard fail.
        let state = if state.schema != SCHEMA_VERSION {
            tracing::warn!(
                path = %path.display(),
                expected = SCHEMA_VERSION,
                got = state.schema,
                "checkpoint schema mismatch; resetting to empty",
            );
            CheckpointStoreState {
                schema: SCHEMA_VERSION,
                entries: BTreeMap::new(),
            }
        } else {
            state
        };
        Ok(Self { path, state })
    }

    /// Look up a checkpoint by `(project_dir, session_filename)`.
    /// Returns `None` when no entry exists yet.
    pub fn get(&self, project_dir: &str, session_filename: &str) -> Option<&Checkpoint> {
        let key = make_key(project_dir, session_filename);
        self.state.entries.get(&key)
    }

    /// Insert or replace a checkpoint. Caller invokes
    /// [`Self::save_atomic`] to persist.
    pub fn put(&mut self, project_dir: &str, session_filename: &str, cp: Checkpoint) {
        let key = make_key(project_dir, session_filename);
        self.state.entries.insert(key, cp);
    }

    /// Remove a checkpoint (e.g. after a session finalises and we
    /// want to reclaim the slot).
    pub fn remove(&mut self, project_dir: &str, session_filename: &str) -> bool {
        let key = make_key(project_dir, session_filename);
        self.state.entries.remove(&key).is_some()
    }

    /// Total number of tracked checkpoints. Used by the watcher's
    /// `/healthz` probe.
    pub fn len(&self) -> usize {
        self.state.entries.len()
    }

    /// `true` when the store carries no checkpoints.
    pub fn is_empty(&self) -> bool {
        self.state.entries.is_empty()
    }

    /// Persist the store via temp-file + rename. The temp file is
    /// always written next to the final path so the rename stays
    /// on the same filesystem (atomic on Unix; close-enough on
    /// Windows where the rename overwrites).
    pub fn save_atomic(&self) -> Result<(), CheckpointError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| CheckpointError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }
        let tmp_path = with_extension(&self.path, "tmp");
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|source| {
            CheckpointError::Malformed {
                path: self.path.display().to_string(),
                source,
            }
        })?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .map_err(|source| CheckpointError::Io {
                path: tmp_path.display().to_string(),
                source,
            })?;
        file.write_all(&bytes).map_err(|source| CheckpointError::Io {
            path: tmp_path.display().to_string(),
            source,
        })?;
        file.sync_all().map_err(|source| CheckpointError::Io {
            path: tmp_path.display().to_string(),
            source,
        })?;
        drop(file);
        fs::rename(&tmp_path, &self.path).map_err(|source| CheckpointError::Io {
            path: self.path.display().to_string(),
            source,
        })?;
        Ok(())
    }

    /// Path the store was opened against. Surfaces in the
    /// watcher's `/healthz` for diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn make_key(project_dir: &str, session_filename: &str) -> String {
    format!("{project_dir}::{session_filename}")
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut out = path.to_path_buf();
    let new_ext = match path.extension().and_then(|s| s.to_str()) {
        Some(existing) => format!("{existing}.{ext}"),
        None => ext.to_string(),
    };
    out.set_extension(new_ext);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cp(uuid: &str, offset: u64) -> Checkpoint {
        Checkpoint {
            session_id: format!("sess-{uuid}"),
            last_record_uuid: uuid.to_string(),
            last_byte_offset: offset,
            written_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn missing_file_yields_empty_store() {
        let dir = tempdir().unwrap();
        let store = CheckpointStore::load(dir.path().join("never-existed.json")).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn put_then_get_round_trips() {
        let dir = tempdir().unwrap();
        let mut store = CheckpointStore::load(dir.path().join("c.json")).unwrap();
        store.put("e--HiveLLM-Cortex", "abc.jsonl", cp("u1", 1024));
        let got = store.get("e--HiveLLM-Cortex", "abc.jsonl").unwrap();
        assert_eq!(got.last_record_uuid, "u1");
        assert_eq!(got.last_byte_offset, 1024);
    }

    #[test]
    fn save_atomic_writes_then_load_recovers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        {
            let mut store = CheckpointStore::load(&path).unwrap();
            store.put("p", "f.jsonl", cp("u1", 64));
            store.save_atomic().unwrap();
        }
        let store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("p", "f.jsonl").unwrap().last_byte_offset, 64);
    }

    #[test]
    fn schema_mismatch_resets_to_empty_with_warning() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        let alien = serde_json::json!({
            "schema": 999,
            "entries": {"foo::bar.jsonl": {
                "session_id": "x",
                "last_record_uuid": "y",
                "last_byte_offset": 0,
                "written_at_ms": 0,
            }},
        });
        fs::write(&path, alien.to_string()).unwrap();
        let store = CheckpointStore::load(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn malformed_json_surfaces_as_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        fs::write(&path, "{ not json").unwrap();
        let err = CheckpointStore::load(&path).unwrap_err();
        assert!(matches!(err, CheckpointError::Malformed { .. }));
    }

    #[test]
    fn remove_returns_true_when_entry_existed() {
        let dir = tempdir().unwrap();
        let mut store = CheckpointStore::load(dir.path().join("c.json")).unwrap();
        store.put("p", "f.jsonl", cp("u1", 0));
        assert!(store.remove("p", "f.jsonl"));
        assert!(!store.remove("p", "f.jsonl"));
    }

    #[test]
    fn save_atomic_is_idempotent_on_repeat_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        let mut store = CheckpointStore::load(&path).unwrap();
        store.put("p", "f.jsonl", cp("u1", 0));
        store.save_atomic().unwrap();
        store.put("p", "f.jsonl", cp("u2", 64));
        store.save_atomic().unwrap();
        let store2 = CheckpointStore::load(&path).unwrap();
        assert_eq!(store2.get("p", "f.jsonl").unwrap().last_record_uuid, "u2");
        assert_eq!(store2.get("p", "f.jsonl").unwrap().last_byte_offset, 64);
    }
}
