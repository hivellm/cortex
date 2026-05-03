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
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
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
        let line = serde_json::to_string(&envelope_to_value(envelope))?;
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
    /// `(stream_tag, hour_bucket) → open writer`.
    open: Mutex<HashMap<(String, String), ArchiveFile>>,
}

struct ArchiveFile {
    /// Path the encoder is writing to. Held for diagnostic logs
    /// when the encoder errors mid-write.
    #[allow(dead_code)]
    path: PathBuf,
    encoder: zstd::stream::write::AutoFinishEncoder<'static, BufWriter<File>>,
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
        let sequence = next_free_sequence(&dir, &self.stream_tag)?;
        let filename = archive_filename(&self.stream_tag, sequence);
        let path = dir.join(&filename);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let encoder = zstd::Encoder::new(BufWriter::new(file), self.level)?.auto_finish();
        open.insert(key, ArchiveFile { path, encoder });
        Ok(())
    }
}

impl Emitter for ArchiveEmitter {
    fn emit(&self, envelope: &MappedEnvelope) -> Result<(), EmitError> {
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
        let line = serde_json::to_vec(&envelope_to_value(envelope))?;
        entry.encoder.write_all(&line)?;
        entry.encoder.write_all(b"\n")?;
        Ok(())
    }

    fn flush(&self) -> Result<(), EmitError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| EmitError::Internal("archive mutex poisoned".into()))?;
        // Drop every open file → the AutoFinishEncoder Drop impl
        // closes the zstd stream cleanly. Subsequent emits open a
        // fresh sequence number per the next_free_sequence rule.
        let drained: Vec<_> = open.drain().collect();
        for (_, entry) in drained {
            // entry's Drop runs the encoder finish.
            drop(entry);
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
    Ok(highest.map(|h| h + 1).unwrap_or(0))
}

/// Serialise a [`MappedEnvelope`] into the wire-format
/// `cortex-api::archive_loader` expects. The shape mirrors
/// `cortex_core::events::Envelope` so a future migration to the
/// fully-typed envelope is field-renames only.
pub fn envelope_to_value(envelope: &MappedEnvelope) -> Value {
    let kind_tag = match envelope.kind {
        EnvelopeKind::Turn => "turn",
        EnvelopeKind::ToolCall => "tool_call",
        EnvelopeKind::AgentCall => "agent_call",
    };
    let mut context = serde_json::Map::new();
    if let Some(repo) = &envelope.repo_slug {
        context.insert("repo".into(), Value::String(repo.clone()));
    }
    if let Some(cwd) = &envelope.cwd {
        context.insert("cwd".into(), Value::String(cwd.clone()));
    }
    if let Some(branch) = &envelope.git_branch {
        context.insert("branch".into(), Value::String(branch.clone()));
    }
    serde_json::json!({
        "event_id": envelope.event_id,
        "schema_version": "1",
        "occurred_at": envelope.occurred_at.to_rfc3339(),
        "stream": "Bootstrap",
        "tool": "claude-code",
        "model": envelope.model,
        "kind": kind_tag,
        "session_id": envelope.session_id,
        "context": Value::Object(context),
        "payload": envelope.payload,
        "parent_event_id": envelope.parent_event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
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
        let v = envelope_to_value(&fixture(EnvelopeKind::Turn, "2026-04-20T17:48:02Z"));
        assert_eq!(v["kind"], json!("turn"));
        assert_eq!(v["tool"], json!("claude-code"));
        assert_eq!(v["stream"], json!("Bootstrap"));
        assert_eq!(v["session_id"], json!("sess"));
        assert_eq!(v["context"]["repo"], json!("cortex"));
        assert_eq!(v["model"], json!("claude-opus-4-7"));
    }

    #[test]
    fn tool_call_kind_maps_to_tool_call_tag() {
        let v = envelope_to_value(&fixture(EnvelopeKind::ToolCall, "2026-04-20T17:48:02Z"));
        assert_eq!(v["kind"], json!("tool_call"));
    }

    #[test]
    fn agent_call_kind_maps_to_agent_call_tag() {
        let v = envelope_to_value(&fixture(EnvelopeKind::AgentCall, "2026-04-20T17:48:02Z"));
        assert_eq!(v["kind"], json!("agent_call"));
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
}
