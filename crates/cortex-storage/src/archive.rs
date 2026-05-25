//! Parquet event archive partition layout.
//!
//! The actual Parquet writer lives in `cortex-core` (spec 04); this module
//! owns only the **layout** so every writer / reader hits the same paths.

use chrono::{DateTime, Datelike, Timelike, Utc};
use std::path::{Path, PathBuf};

/// Root-relative layout rules for the Parquet event archive.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLayout;

impl ArchiveLayout {
    /// Root directory under the Cortex data root.
    pub const ROOT_SEGMENT: &'static str = "events";

    /// Compression codec used when writing new files.
    pub const COMPRESSION: &'static str = "zstd";

    /// Zstd compression level.
    pub const COMPRESSION_LEVEL: i32 = 6;

    /// Rotation granularity (hourly; merges to daily at 90 days).
    pub const ROTATION: ArchiveRotation = ArchiveRotation::Hourly;
}

/// Rotation granularity for archive files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveRotation {
    /// One file per hour.
    Hourly,
    /// One file per day (applied after the 90-day rollup).
    Daily,
    /// One file per month (applied after the 365-day rollup).
    Monthly,
}

/// Compute the archive directory for a given timestamp + stream tag.
///
/// Produces `<data_root>/events/year=YYYY/month=MM/day=DD/hour=HH/`.
pub fn archive_partition(data_root: &Path, ts: DateTime<Utc>) -> PathBuf {
    data_root
        .join(ArchiveLayout::ROOT_SEGMENT)
        .join(format!("year={:04}", ts.year()))
        .join(format!("month={:02}", ts.month()))
        .join(format!("day={:02}", ts.day()))
        .join(format!("hour={:02}", ts.hour()))
}

/// Compute the archive file name for `<stream>-<sequence>.parquet`.
///
/// `stream` is typically `"raw"` or `"bootstrap"`; `sequence` is an
/// 0-padded counter that the writer rotates when files reach a size cap.
pub fn archive_filename(stream_tag: &str, sequence: u32) -> String {
    format!("{stream_tag}-{sequence:05}.parquet")
}

// ----------------------------------------------------------------------
// Phase11p §0 — envelope retrieval helpers.
//
// The keyword-lane bootstrap loader in `cortex-api::archive_loader`
// projects each envelope onto a `LaneHit` row; the consolidator's
// live read path needs full envelopes (kind + payload). Walking the
// same parquet hierarchy with a per-line predicate keeps both paths
// on the same on-disk format without duplicating the zstd / NDJSON
// decode. Lives in `cortex-storage` (not `cortex-api`) because
// `cortex-workers::consolidator::source` calls these directly and
// `cortex-workers` does not depend on `cortex-api`.
// ----------------------------------------------------------------------

use cortex_core::events::Envelope;
use std::fs::File;
use std::io::{BufRead, BufReader};

/// Failure modes raised by the envelope walker.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// I/O while reading or decompressing.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Walk the archive under `archive_root`, decode every envelope,
/// and pass each to `predicate`. When `predicate` returns `true`
/// the envelope is appended to the result set; when it returns
/// `false` the envelope is dropped. Returns the matched set
/// sorted by `occurred_at` (RFC-3339 lexicographic comparison
/// matches chronological order for our timestamps).
pub fn walk_envelopes<F>(archive_root: &Path, mut predicate: F) -> Result<Vec<Envelope>, ScanError>
where
    F: FnMut(&Envelope) -> bool,
{
    let mut out: Vec<Envelope> = Vec::new();
    walk_envelopes_dir(archive_root, &mut predicate, &mut out)?;
    out.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at));
    Ok(out)
}

fn walk_envelopes_dir<F>(
    dir: &Path,
    predicate: &mut F,
    out: &mut Vec<Envelope>,
) -> Result<(), ScanError>
where
    F: FnMut(&Envelope) -> bool,
{
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_envelopes_dir(&path, predicate, out)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        // Mirror the keyword loader's defensive shape: corrupt zstd
        // frames are bypassed rather than aborting the whole walk
        // (silent data loss already documented in
        // `docs/analysis/adapter/01-tool-call-archive-loss.md`).
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let decoder = match zstd::stream::read::Decoder::new(file) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let reader = BufReader::new(decoder);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(env) = serde_json::from_str::<Envelope>(trimmed) {
                if predicate(&env) {
                    out.push(env);
                }
            }
        }
    }
    Ok(())
}

/// Phase11p §0.1 — collect every envelope whose `session_id`
/// matches, sorted by `occurred_at`. Empty result is `Ok(vec![])`,
/// never an error.
pub fn scan_envelopes_by_session(
    archive_root: &Path,
    session_id: &str,
) -> Result<Vec<Envelope>, ScanError> {
    walk_envelopes(archive_root, |env| env.session_id == session_id)
}

/// Phase11p ad-hoc — collect every distinct `session_id` present in
/// the archive (skipping envelopes that carry an empty session id).
/// Used by the consolidator's corpus back-fill pass when the metadata
/// SQLite has no `sessions` rows but the archive on disk has years of
/// envelopes from older runs.
pub fn enumerate_session_ids(archive_root: &Path) -> Result<Vec<String>, ScanError> {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    walk_envelopes(archive_root, |env| {
        let sid = env.session_id.trim();
        if !sid.is_empty() {
            seen.insert(sid.to_string());
        }
        false
    })?;
    Ok(seen.into_iter().collect())
}

/// Phase11p §0.2 — find the single envelope whose `event_id`
/// matches. Envelope ids are globally unique ULIDs so the first
/// match is the only match; the walk short-circuits on hit.
/// Returns `Ok(None)` when no envelope carries the id.
pub fn scan_envelope_by_event_id(
    archive_root: &Path,
    event_id: &str,
) -> Result<Option<Envelope>, ScanError> {
    let mut found: Option<Envelope> = None;
    walk_envelopes(archive_root, |env| {
        if found.is_some() {
            return false;
        }
        if env.event_id == event_id {
            found = Some(env.clone());
        }
        false
    })?;
    Ok(found)
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use cortex_core::events::{Context, Kind, Stream, Turn};
    use std::collections::BTreeMap;
    use std::io::Write;

    fn envelope_with(event_id: &str, session_id: &str, occurred_at: &str) -> Envelope {
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: occurred_at.to_string(),
            ingested_at: None,
            session_id: session_id.to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: Some("cortex".to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".to_string(),
                ide: None,
                extras: BTreeMap::new(),
            },
            payload: serde_json::to_value(Turn {
                user_message: "x".to_string(),
                assistant_message: None,
                tokens: None,
                tool_call_event_ids: Vec::new(),
            })
            .unwrap(),
            redactions: Vec::new(),
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            parent_event_id: None,
        }
    }

    fn write_archive_file(root: &Path, rel_dir: &str, envelopes: &[Envelope]) -> PathBuf {
        let dir = root.join(rel_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw-00000.parquet");
        let file = File::create(&path).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
        for env in envelopes {
            let line = serde_json::to_string(env).unwrap();
            enc.write_all(line.as_bytes()).unwrap();
            enc.write_all(b"\n").unwrap();
        }
        enc.finish().unwrap();
        path
    }

    #[test]
    fn scan_envelopes_by_session_zero_match_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=19",
            &[envelope_with("E1", "OTHER", "2026-04-26T19:04:00Z")],
        );
        let got = scan_envelopes_by_session(dir.path(), "WANTED").unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn scan_envelopes_by_session_orders_by_occurred_at() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=19",
            &[
                envelope_with("E5", "S1", "2026-04-26T19:05:00Z"),
                envelope_with("E0", "S1", "2026-04-26T19:00:00Z"),
                envelope_with("E2", "S1", "2026-04-26T19:02:00Z"),
            ],
        );
        let got = scan_envelopes_by_session(dir.path(), "S1").unwrap();
        let ids: Vec<&str> = got.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["E0", "E2", "E5"]);
    }

    #[test]
    fn scan_envelopes_by_session_unions_across_hour_partitions() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=18",
            &[envelope_with("E18", "S2", "2026-04-26T18:30:00Z")],
        );
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=19",
            &[envelope_with("E19", "S2", "2026-04-26T19:30:00Z")],
        );
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=20",
            &[envelope_with("E_OTHER", "S99", "2026-04-26T20:30:00Z")],
        );
        let got = scan_envelopes_by_session(dir.path(), "S2").unwrap();
        let ids: Vec<&str> = got.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["E18", "E19"]);
    }

    #[test]
    fn scan_envelope_by_event_id_hit_returns_full_envelope() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=19",
            &[
                envelope_with("E_A", "S1", "2026-04-26T19:00:00Z"),
                envelope_with("E_B", "S1", "2026-04-26T19:01:00Z"),
            ],
        );
        let got = scan_envelope_by_event_id(dir.path(), "E_B").unwrap();
        assert!(got.is_some());
        let env = got.unwrap();
        assert_eq!(env.event_id, "E_B");
        assert_eq!(env.session_id, "S1");
    }

    #[test]
    fn scan_envelope_by_event_id_miss_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        write_archive_file(
            dir.path(),
            "events/year=2026/month=04/day=26/hour=19",
            &[envelope_with("E_A", "S1", "2026-04-26T19:00:00Z")],
        );
        let got = scan_envelope_by_event_id(dir.path(), "NOT_THERE").unwrap();
        assert!(got.is_none());
    }
}
