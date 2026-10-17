//! Envelope sinks — phase11i §1.5.
//!
//! Two sinks, each implementing the [`Emitter`] trait:
//!
//! - [`StdoutEmitter`] — writes one canonical-JSON envelope per
//!   line to stdout. Used by tests and by `cortex-claude-archive
//!   bootstrap --sink stdout` for piping into `jq` / external
//!   tooling.
//! - [`ArchiveEmitter`] — writes zstd-compressed NDJSON to
//!   `<CORTEX_ARCHIVE_ROOT>/events/year=YYYY/month=MM/day=DD/hour=HH/<tag>-NNNNN.parquet`,
//!   matching the format `cortex-ingestion`'s `NdJsonZstdArchive`
//!   produces and `cortex-api`'s `archive_loader` consumes at
//!   boot. Picks the next free sequence number so a crash mid-
//!   write never corrupts a previously-flushed file.
//!
//! A future §1.5 follow-up adds a `SynapEmitter` that publishes
//! to `cortex.events.bootstrap` directly. For now the archive
//! sink is the canonical path: cortex-api's archive_loader
//! re-reads it at boot and seeds the keyword lane, so envelopes
//! become queryable on the next daemon restart without needing
//! the live worker pipeline up.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use cortex_core::{Context, Envelope, Kind, Stream};
use cortex_storage::{archive_filename, archive_partition};
use serde_json::Value;
use thiserror::Error;

use super::mapper::{EnvelopeKind, MappedEnvelope};

/// Stream tag used by every archive file written from this crate.
/// `cortex-api`'s archive_loader matches on the prefix when it
/// walks `<archive_root>/events/.../*.parquet`, so any tag that
/// starts with `raw-` or `bootstrap-` is picked up. We use
/// `bootstrap-claude` so the operator can grep the partition for
/// claude-archive output specifically.
pub const STREAM_TAG: &str = "bootstrap-claude";

/// Errors raised by the emitter layer.
#[derive(Debug, Error)]
pub enum EmitError {
    /// Filesystem or zstd I/O failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialisation failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Canonical payload hashing failed before any archive bytes were written.
    #[error("canonical payload: {0}")]
    Canonical(#[from] cortex_core::CanonicalJsonError),
    /// Internal bug (poisoned mutex, etc.).
    #[error("internal: {0}")]
    Internal(String),
}

/// Abstract sink. Implementations: [`StdoutEmitter`],
/// [`ArchiveEmitter`].
pub trait Emitter: Send + Sync {
    /// Persist one envelope. Must not return until the bytes are
    /// durable (flushed + fsynced for the archive sink; written
    /// to stdout for the stdout sink).
    fn emit(&self, envelope: &MappedEnvelope) -> Result<(), EmitError>;

    /// Flush every open writer. The CLI calls this on graceful
    /// shutdown; the archive sink uses it to close the current
    /// hourly partition.
    fn flush(&self) -> Result<(), EmitError>;
}

/// Stdout sink. One canonical-JSON line per envelope. Cheap; used
/// by tests, piping into `jq`, and the `--sink stdout` CLI flag.
pub struct StdoutEmitter {
    inner: Mutex<()>,
}

impl StdoutEmitter {
    /// Build a stdout sink. The internal mutex serialises writes so
    /// concurrent emitters do not interleave bytes mid-line.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(()),
        }
    }
}

impl Default for StdoutEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Emitter for StdoutEmitter {
    fn emit(&self, envelope: &MappedEnvelope) -> Result<(), EmitError> {
        let _guard = self
            .inner
            .lock()
            .map_err(|_| EmitError::Internal("stdout mutex poisoned".into()))?;
        let line = serde_json::to_string(&envelope_to_value(envelope)?)?;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(line.as_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        Ok(())
    }

    fn flush(&self) -> Result<(), EmitError> {
        Ok(())
    }
}

/// Archive sink. Writes zstd-compressed NDJSON to one hourly file
/// per partition under `<root>/events/year=…/…/<tag>-NNNNN.parquet`.
/// Each emit appends one complete, synced frame; at most 32 handles are cached.
/// The file format matches what cortex-ingestion's
/// `NdJsonZstdArchive` writes and what cortex-api's
/// `archive_loader` reads.
pub struct ArchiveEmitter {
    root: PathBuf,
    /// zstd compression level. `0` defers to zstd's default; `3`
    /// is a reasonable balance for log-heavy NDJSON.
    level: i32,
    /// Stream tag prefix every file gets — usually [`STREAM_TAG`]
    /// but the constructor leaves it overridable so tests can
    /// segregate fixture output.
    stream_tag: String,
    /// `(stream_tag, hour_bucket) → open file`, bounded across old partitions.
    open: Mutex<HashMap<(String, String), ArchiveFile>>,
}

const MAX_OPEN_ARCHIVE_FILES: usize = 32;

struct ArchiveFile {
    /// Path retained with the handle for diagnostics and tests.
    #[allow(dead_code)]
    path: PathBuf,
    file: File,
}

impl ArchiveEmitter {
    /// Build a new archive sink rooted at `root`. The directory
    /// tree is created lazily on the first emit. `level=3` is a
    /// reasonable default; tests can pass `1` for speed.
    pub fn new(root: impl Into<PathBuf>, level: i32) -> Self {
        Self {
            root: root.into(),
            level,
            stream_tag: STREAM_TAG.to_string(),
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Override the stream tag prefix. Tests use this to keep
    /// fixture archives separate from production output.
    pub fn with_stream_tag(mut self, tag: impl Into<String>) -> Self {
        self.stream_tag = tag.into();
        self
    }

    fn ensure_open(
        &self,
        bucket: &str,
        ts: DateTime<Utc>,
        open: &mut HashMap<(String, String), ArchiveFile>,
    ) -> Result<(), EmitError> {
        let key = (self.stream_tag.clone(), bucket.to_string());
        if open.contains_key(&key) {
            return Ok(());
        }
        let dir = archive_partition(&self.root, ts);
        std::fs::create_dir_all(&dir)?;
        let mut sequence = next_free_sequence(&dir, &self.stream_tag)?;
        let (path, file) = loop {
            let path = dir.join(archive_filename(&self.stream_tag, sequence));
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
            // Each prior emit already synced a completed frame, so eviction
            // cannot strand encoder state or discard buffered envelopes.
            if let Some(oldest) = open.keys().min_by(|a, b| a.1.cmp(&b.1)).cloned() {
                open.remove(&oldest);
            }
        }
        open.insert(key, ArchiveFile { path, file });
        Ok(())
    }
}

impl Emitter for ArchiveEmitter {
    fn emit(&self, envelope: &MappedEnvelope) -> Result<(), EmitError> {
        let mut line = serde_json::to_vec(&envelope_to_value(envelope)?)?;
        line.push(b'\n');
        // encode_all returns only after the frame has finished. No Drop-time
        // finalization can hide codec errors or leave an hour-long frame open.
        let frame = zstd::encode_all(line.as_slice(), self.level)?;
        let bucket = format!("{}", envelope.occurred_at.format("%Y%m%d%H"));
        let key = (self.stream_tag.clone(), bucket.clone());
        let mut open = self
            .open
            .lock()
            .map_err(|_| EmitError::Internal("archive mutex poisoned".into()))?;
        self.ensure_open(&bucket, envelope.occurred_at, &mut open)?;
        let entry = open
            .get_mut(&key)
            .ok_or_else(|| EmitError::Internal("ensure_open did not insert".into()))?;
        let result = entry
            .file
            .write_all(&frame)
            .and_then(|()| entry.file.flush())
            .and_then(|()| entry.file.sync_data());
        if let Err(error) = result {
            // Never append again after an unacknowledged/possibly partial tail.
            open.remove(&key);
            return Err(error.into());
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), EmitError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| EmitError::Internal("archive mutex poisoned".into()))?;
        // Preserve flush's existing close-and-rotate behavior, but surface
        // synchronization errors rather than relying on AutoFinish Drop.
        for (_, mut entry) in open.drain() {
            entry.file.flush()?;
            entry.file.sync_data()?;
        }
        Ok(())
    }
}

/// Walk the partition dir and pick a sequence number that does
/// not clash with any existing file. Mirrors the rule in
/// `cortex-ingestion::archive::next_free_sequence` so a hard-
/// killed prior run cannot corrupt an existing zstd tail.
fn next_free_sequence(dir: &Path, stream_tag: &str) -> Result<u32, EmitError> {
    let prefix = format!("{stream_tag}-");
    let mut highest: Option<u32> = None;
    let read = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(EmitError::Io(e)),
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(rest) = name.strip_prefix(&prefix) {
            // Sequence is the digit run before any `.` — the
            // archive_filename helper ships `<tag>-NNNNN.parquet`.
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                highest = Some(highest.map_or(n, |h| h.max(n)));
            }
        }
    }
    match highest {
        Some(highest) => highest
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("archive sequence exhausted").into()),
        None => Ok(0),
    }
}

/// Serialize through the canonical envelope type consumed by archive indexing.
/// Source IDs and payloads stay intact. The importer cannot establish a missing
/// repository, original platform, or tool outcome from its own filesystem.
pub fn envelope_to_value(envelope: &MappedEnvelope) -> Result<Value, EmitError> {
    let kind = match envelope.kind {
        EnvelopeKind::Turn => Kind::Turn,
        EnvelopeKind::ToolCall => Kind::ToolCall,
        EnvelopeKind::AgentCall => Kind::AgentCall,
    };
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    Ok(serde_json::to_value(Envelope {
        event_id: envelope.event_id.clone(),
        schema_version: "1".into(),
        occurred_at: envelope.occurred_at.to_rfc3339(),
        ingested_at: None,
        session_id: envelope.session_id.clone(),
        stream: Stream::Bootstrap,
        tool: "claude-code".into(),
        model: envelope.model.clone(),
        kind,
        context: Context {
            repo: envelope.repo_slug.clone(),
            branch: envelope.git_branch.clone(),
            commit: None,
            cwd: envelope.cwd.clone(),
            user: None,
            platform: platform.into(),
            ide: None,
            extras: std::collections::BTreeMap::from([(
                "claude_archive".into(),
                serde_json::json!({"platform_origin":"import_host", "source_platform":"unknown"}),
            )]),
        },
        payload: envelope.payload.clone(),
        redactions: Vec::new(),
        content_hash: cortex_core::content_hash(&envelope.payload)?.to_string(),
        parent_event_id: envelope.parent_event_id.clone(),
        class_level: None,
        class_compartments: None,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::archive::read_archive_file;
    use chrono::TimeZone;
    use serde_json::json;
    use std::io::Read;
    use tempfile::tempdir;

    fn fixture(kind: EnvelopeKind, ts: &str) -> MappedEnvelope {
        MappedEnvelope {
            event_id: "01TEST".into(),
            kind,
            occurred_at: chrono::DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            session_id: "sess".into(),
            cwd: Some("/repo".into()),
            git_branch: Some("main".into()),
            model: Some("claude-opus-4-7".into()),
            repo_slug: Some("cortex".into()),
            parent_event_id: None,
            payload: json!({"user_message": "hi", "assistant_message": "hello"}),
        }
    }

    #[test]
    fn envelope_to_value_emits_canonical_shape() {
        let v = envelope_to_value(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z")).unwrap();
        assert_eq!(v["kind"], json!("turn"));
        assert_eq!(v["tool"], json!("claude-code"));
        assert_eq!(v["stream"], json!("bootstrap"));
        assert_eq!(v["session_id"], json!("sess"));
        assert_eq!(v["context"]["repo"], json!("cortex"));
        assert_eq!(v["model"], json!("claude-opus-4-7"));
        let parsed: Envelope = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.stream, Stream::Bootstrap);
        assert_eq!(
            parsed.context.extras["claude_archive"]["platform_origin"],
            "import_host"
        );
        assert_eq!(
            parsed.content_hash,
            cortex_core::content_hash(&parsed.payload)
                .unwrap()
                .to_string()
        );
    }

    #[test]
    fn tool_call_kind_maps_to_tool_call_tag() {
        let v =
            envelope_to_value(&fixture(EnvelopeKind::ToolCall, "2026-04-20T17:48:02Z")).unwrap();
        assert_eq!(v["kind"], json!("tool_call"));
    }

    #[test]
    fn agent_call_kind_maps_to_agent_call_tag() {
        let v =
            envelope_to_value(&fixture(EnvelopeKind::AgentCall, "2026-04-20T17:48:02Z")).unwrap();
        assert_eq!(v["kind"], json!("agent_call"));
    }

    #[test]
    fn unknown_source_repository_and_outcome_are_not_inferred() {
        let mut mapped = fixture(EnvelopeKind::ToolCall, "2026-04-20T17:48:02Z");
        mapped.repo_slug = None;
        mapped.cwd = Some("E:/other-project/app".into());
        mapped.payload = json!({"tool_name":"Bash", "input":{"command":"true"}});
        let value = envelope_to_value(&mapped).unwrap();
        let parsed: Envelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed.context.repo, None);
        assert_eq!(parsed.context.cwd, mapped.cwd);
        assert_eq!(parsed.event_id, mapped.event_id);
        assert_eq!(parsed.payload, mapped.payload);
        assert!(value["payload"].get("outcome").is_none());
    }

    #[test]
    fn emitted_turn_is_projected_by_incremental_index_before_and_after_restart() {
        use cortex_storage::archive_index::ArchiveIndex;

        fn project(envelope: &Envelope) -> Option<Value> {
            let turn: cortex_core::Turn = serde_json::from_value(envelope.payload.clone()).ok()?;
            Some(
                json!({"id":envelope.event_id, "repo":envelope.context.repo, "text":turn.user_message}),
            )
        }

        fn settle(index: &mut ArchiveIndex) -> Vec<Value> {
            for _ in 0..64 {
                let report = index.poll(project).unwrap();
                assert!(report.errors.is_empty(), "{report:?}");
                if report.discovery_complete
                    && report.pending_files == 0
                    && !report.historical_backfill
                {
                    return index.snapshot().unwrap();
                }
            }
            panic!("incremental archive did not settle");
        }

        let directory = tempdir().unwrap();
        let archive = directory.path().join("archive");
        let metadata = directory.path().join("metadata.sqlite");
        let emitter = ArchiveEmitter::new(&archive, 1);
        let first = fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z");
        emitter.emit(&first).unwrap();
        let mut index = ArchiveIndex::open(&archive, &metadata).unwrap();
        let initial = settle(&mut index);
        assert_eq!(
            initial,
            vec![json!({"id":first.event_id, "repo":"cortex", "text":"hi"})]
        );
        drop(index);

        let mut second = first.clone();
        second.event_id = "01SECOND".into();
        second.payload["user_message"] = json!("after restart");
        emitter.emit(&second).unwrap();
        let mut restarted = ArchiveIndex::open(&archive, &metadata).unwrap();
        let resumed = settle(&mut restarted);
        assert_eq!(resumed.len(), 2);
        assert!(resumed.iter().any(|value| value["id"] == first.event_id));
        assert!(resumed
            .iter()
            .any(|value| value["id"] == second.event_id && value["text"] == "after restart"));
    }

    #[test]
    fn archive_emitter_creates_partition_dir_on_first_emit() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path().to_path_buf(), 1);
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"))
            .unwrap();
        emitter.flush().unwrap();
        // archive_partition splits into year=YYYY/month=MM/day=DD/hour=HH.
        let p = dir
            .path()
            .join("events")
            .join("year=2026")
            .join("month=04")
            .join("day=20")
            .join("hour=17");
        assert!(p.exists(), "partition dir was not created: {}", p.display());
        let files: Vec<_> = std::fs::read_dir(&p).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let name = files[0].file_name().to_string_lossy().to_string();
        assert!(name.starts_with("bootstrap-claude-"), "name: {name}");
    }

    #[test]
    fn next_free_sequence_picks_zero_for_empty_dir() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        assert_eq!(
            next_free_sequence(dir.path(), "bootstrap-claude").unwrap(),
            0
        );
    }

    #[test]
    fn next_free_sequence_skips_used_numbers() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bootstrap-claude-00000.parquet"), b"").unwrap();
        std::fs::write(dir.path().join("bootstrap-claude-00007.parquet"), b"").unwrap();
        assert_eq!(
            next_free_sequence(dir.path(), "bootstrap-claude").unwrap(),
            8
        );
    }

    #[test]
    fn second_emit_in_same_hour_uses_same_file() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path().to_path_buf(), 1);
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"))
            .unwrap();
        emitter
            .emit(&fixture(EnvelopeKind::ToolCall, "2026-04-20T17:50:00Z"))
            .unwrap();
        emitter.flush().unwrap();
        let p = dir.path().join("events/year=2026/month=04/day=20/hour=17");
        let count = std::fs::read_dir(&p).unwrap().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn second_emit_in_different_hour_opens_new_file() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path().to_path_buf(), 1);
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"))
            .unwrap();
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T18:00:00Z"))
            .unwrap();
        emitter.flush().unwrap();
        let h17 = dir.path().join("events/year=2026/month=04/day=20/hour=17");
        let h18 = dir.path().join("events/year=2026/month=04/day=20/hour=18");
        assert_eq!(std::fs::read_dir(&h17).unwrap().count(), 1);
        assert_eq!(std::fs::read_dir(&h18).unwrap().count(), 1);
    }

    #[test]
    fn flush_closes_files_so_next_emit_picks_next_sequence() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path().to_path_buf(), 1);
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"))
            .unwrap();
        emitter.flush().unwrap();
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:49:00Z"))
            .unwrap();
        emitter.flush().unwrap();
        let p = dir.path().join("events/year=2026/month=04/day=20/hour=17");
        let files: Vec<_> = std::fs::read_dir(&p).unwrap().flatten().collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn stdout_emitter_does_not_panic_on_repeated_emit() {
        // Cannot capture stdout in a unit test cheaply; smoke-test
        // that the trait impl runs without panicking. The format
        // round-trip is covered by envelope_to_value tests.
        let emitter = StdoutEmitter::new();
        emitter
            .emit(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"))
            .unwrap();
        emitter.flush().unwrap();
    }

    #[test]
    fn bucket_format_matches_partition_helper() {
        // Cross-check the in-emitter bucket key against the
        // partition helper so a future change to either format
        // surfaces here.
        let ts = Utc.with_ymd_and_hms(2026, 4, 20, 17, 48, 2).unwrap();
        let bucket = format!("{}", ts.format("%Y%m%d%H"));
        assert_eq!(bucket, "2026042017");
    }

    #[test]
    fn emit_finishes_each_frame_before_flush_or_drop() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path(), 1);
        let mut first = fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z");
        first.event_id = "first".into();
        emitter.emit(&first).unwrap();
        let path = emitter
            .open
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .path
            .clone();
        assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        let mut second = fixture(EnvelopeKind::ToolCall, "2026-04-20T17:49:02Z");
        second.event_id = "second".into();
        emitter.emit(&second).unwrap();
        assert_eq!(emitter.open.lock().unwrap().len(), 1);
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
        assert_eq!(
            serde_json::from_slice::<Value>(&zstd::decode_all(remaining).unwrap()).unwrap()
                ["event_id"],
            "second"
        );
    }

    #[test]
    fn old_hour_eviction_bounds_handles_without_losing_frames() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path(), 1);
        let first = fixture(EnvelopeKind::Turn, "2026-04-20T00:00:00Z");
        let mut paths = Vec::new();
        for index in 0..MAX_OPEN_ARCHIVE_FILES + 3 {
            let mut envelope = first.clone();
            envelope.occurred_at += chrono::Duration::hours(index as i64);
            emitter.emit(&envelope).unwrap();
            let key = (
                STREAM_TAG.to_string(),
                envelope.occurred_at.format("%Y%m%d%H").to_string(),
            );
            let open = emitter.open.lock().unwrap();
            assert!(open.len() <= MAX_OPEN_ARCHIVE_FILES);
            paths.push(open.get(&key).unwrap().path.clone());
        }
        let first_key = (STREAM_TAG.to_string(), "2026042000".to_string());
        assert!(!emitter.open.lock().unwrap().contains_key(&first_key));
        emitter.emit(&first).unwrap();
        let reopened = emitter
            .open
            .lock()
            .unwrap()
            .get(&first_key)
            .unwrap()
            .path
            .clone();
        assert_ne!(reopened, paths[0]);
        paths.push(reopened);
        for path in paths {
            assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        }
        assert_eq!(emitter.open.lock().unwrap().len(), MAX_OPEN_ARCHIVE_FILES);
    }

    #[test]
    fn failed_emit_retires_file_instead_of_appending_after_unknown_tail() {
        let dir = tempdir().unwrap();
        let emitter = ArchiveEmitter::new(dir.path(), 1);
        let envelope = fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z");
        emitter.emit(&envelope).unwrap();
        let path = {
            let mut open = emitter.open.lock().unwrap();
            let entry = open.values_mut().next().unwrap();
            entry.file = File::open(&entry.path).unwrap();
            entry.path.clone()
        };
        assert!(matches!(emitter.emit(&envelope), Err(EmitError::Io(_))));
        assert!(emitter.open.lock().unwrap().is_empty());
        emitter.emit(&envelope).unwrap();
        let reopened = emitter
            .open
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .path
            .clone();
        assert_ne!(reopened, path);
        assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        assert_eq!(read_archive_file(&reopened).unwrap().len(), 1);
    }

    #[test]
    fn existing_corrupt_tail_is_preserved_and_new_emit_uses_fresh_sequence() {
        let dir = tempdir().unwrap();
        let envelope = fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z");
        let partition = archive_partition(dir.path(), envelope.occurred_at);
        std::fs::create_dir_all(&partition).unwrap();
        let corrupt = partition.join(archive_filename(STREAM_TAG, 0));
        std::fs::write(&corrupt, b"unknown old tail").unwrap();
        let emitter = ArchiveEmitter::new(dir.path(), 1);
        emitter.emit(&envelope).unwrap();
        let path = emitter
            .open
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .path
            .clone();
        assert_ne!(path, corrupt);
        assert_eq!(read_archive_file(&path).unwrap().len(), 1);
        assert_eq!(std::fs::read(&corrupt).unwrap(), b"unknown old tail");
    }

    #[test]
    fn sequence_exhaustion_returns_error_instead_of_wrapping() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path()
                .join(format!("{STREAM_TAG}-{}.parquet", u32::MAX)),
            b"",
        )
        .unwrap();
        assert!(matches!(
            next_free_sequence(dir.path(), STREAM_TAG),
            Err(EmitError::Io(_))
        ));
    }
}
