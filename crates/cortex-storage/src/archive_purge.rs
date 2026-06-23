//! Phase12b — bulk Parquet archive purge.
//!
//! `purge_before(home, cutoff, dry_run)` walks
//! `${home}/events/year=YYYY/month=MM/day=DD/hour=HH/*.parquet` and
//! deletes every file whose **newest** envelope's `occurred_at` is
//! strictly older than `cutoff`. Files holding any envelope newer
//! than the cutoff are left in place — partial purge would orphan
//! the surviving rows from their indexed counterparts in
//! Vectorizer / Meili / Nexus.
//!
//! ## Why it's a bulk path
//!
//! `cortex-api`'s [`/v1/admin/forget`] surface deletes a single event
//! id at a time and is unusably slow at scale. Phase12b ships the
//! `--before <RFC3339>` bulk path operators were already running as
//! `rm -rf` in production. The cron seed (phase12b §3) wires
//! `retention.archive_purge` to a daily 03:00 UTC tick with a 365-day
//! retention default; operators tune cadence + retention via the
//! existing cron-edit surface without code changes.
//!
//! ## Live-frame guard
//!
//! The current-hour partition contains an actively-written file
//! whose tail frame may be half-flushed when the purge runs. The
//! envelope reader treats a partial frame as end-of-stream (mirrors
//! `cortex-api::admin_forget::is_live_partial_frame`) and stops
//! parsing, so the `newest_occurred_at` we observe reflects only
//! the cleanly-flushed prefix. Every file with a partial-frame tail
//! is preserved; a future run picks them up once the writer rolls
//! to the next hour.

use chrono::{DateTime, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use cortex_core::events::Envelope;

/// Failure modes raised by [`purge_before`].
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    /// I/O while walking the archive or removing a file.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Per-run summary the operator surface (cron + CLI) emits.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PurgeReport {
    /// Files actually deleted (or, in `dry_run`, files that would be).
    pub files_deleted: u64,
    /// Total bytes reclaimed across `files_deleted`.
    pub bytes_reclaimed: u64,
    /// Distinct hour-partition directories visited.
    pub partitions_visited: u64,
    /// Files skipped because the newest envelope was at or after `cutoff`.
    pub files_kept: u64,
    /// Files skipped because the tail frame was incomplete (live writer).
    pub files_partial: u64,
    /// Files skipped because the path could not be opened or parsed.
    pub files_unreadable: u64,
    /// Whether the run was a `--dry-run` (no deletions actually performed).
    pub dry_run: bool,
    /// `--repo` filter, when present. `None` means "all repos".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_filter: Option<String>,
    /// RFC-3339 cutoff the run honoured. Surfaced so a downstream
    /// `tier_transitions_json` audit row carries the input that
    /// produced these counters.
    pub cutoff: String,
}

/// Walk every Parquet file under `${home}/events/**` and delete the
/// ones whose newest envelope's `occurred_at` is strictly older than
/// `cutoff`. When `dry_run` is true, deletions are tallied but not
/// performed.
///
/// `repo_filter` (when `Some`) restricts deletion to envelopes whose
/// `context.repo` matches; files that contain at least one envelope
/// for any other repo are left in place even if every other envelope
/// is old. v1 honours the conservative shape: a file is deletable
/// only when **every** envelope inside it would be deleted.
pub fn purge_before(
    home: &Path,
    cutoff: DateTime<Utc>,
    dry_run: bool,
    repo_filter: Option<&str>,
) -> Result<PurgeReport, PurgeError> {
    let mut report = PurgeReport {
        dry_run,
        repo_filter: repo_filter.map(String::from),
        cutoff: cutoff.to_rfc3339(),
        ..Default::default()
    };
    let archive_root = home.join("events");
    if !archive_root.exists() {
        // No archive yet — empty result is success.
        return Ok(report);
    }
    purge_dir(&archive_root, cutoff, dry_run, repo_filter, &mut report)?;
    Ok(report)
}

fn purge_dir(
    dir: &Path,
    cutoff: DateTime<Utc>,
    dry_run: bool,
    repo_filter: Option<&str>,
    report: &mut PurgeReport,
) -> Result<(), PurgeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    let mut had_subdir = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            had_subdir = true;
            purge_dir(&path, cutoff, dry_run, repo_filter, report)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("parquet") {
            files.push(path);
        }
    }
    // Only count this dir as a partition when it directly held
    // parquet files — intermediate `year=YYYY/month=MM` levels are
    // not partitions in the metric we surface.
    if !files.is_empty() && !had_subdir {
        report.partitions_visited = report.partitions_visited.saturating_add(1);
    }
    for path in files {
        match classify_file(&path, cutoff, repo_filter)? {
            FileVerdict::Delete => {
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                report.bytes_reclaimed = report.bytes_reclaimed.saturating_add(bytes);
                report.files_deleted = report.files_deleted.saturating_add(1);
                if !dry_run {
                    if let Err(e) = std::fs::remove_file(&path) {
                        // The CLI surface (phase12b §2.2) maps this
                        // to exit code 2 via `PurgeReport` carrying
                        // a non-zero `files_unreadable` count.
                        report.files_deleted = report.files_deleted.saturating_sub(1);
                        report.bytes_reclaimed = report.bytes_reclaimed.saturating_sub(bytes);
                        report.files_unreadable = report.files_unreadable.saturating_add(1);
                        // cortex-storage stays dependency-light (no
                        // `tracing`); the CLI surface logs the
                        // structured event from the per-file delete
                        // result it walks. Stderr keeps the bug
                        // visible during ad-hoc CLI runs.
                        eprintln!("archive_purge: remove_file {} failed: {e}", path.display());
                    }
                }
            }
            FileVerdict::Keep => report.files_kept = report.files_kept.saturating_add(1),
            FileVerdict::Partial => report.files_partial = report.files_partial.saturating_add(1),
            FileVerdict::Unreadable => {
                report.files_unreadable = report.files_unreadable.saturating_add(1)
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileVerdict {
    Delete,
    Keep,
    /// Tail frame is half-flushed by the live writer — skip until the
    /// next run when the writer has rolled to a new hour.
    Partial,
    /// Could not open / decode at all — record but skip.
    Unreadable,
}

fn classify_file(
    path: &Path,
    cutoff: DateTime<Utc>,
    repo_filter: Option<&str>,
) -> Result<FileVerdict, PurgeError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(FileVerdict::Unreadable),
    };
    let decoder = match zstd::stream::read::Decoder::new(file) {
        Ok(d) => d,
        Err(_) => return Ok(FileVerdict::Unreadable),
    };
    let reader = BufReader::new(decoder);
    let mut newest: Option<DateTime<Utc>> = None;
    let mut saw_any = false;
    let mut saw_other_repo = false;
    let mut saw_partial_tail = false;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) if is_live_partial_frame(&e) => {
                saw_partial_tail = true;
                break;
            }
            Err(_) => return Ok(FileVerdict::Unreadable),
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let env: Envelope = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => continue,
        };
        saw_any = true;
        if let Some(filter) = repo_filter {
            let env_repo = env.context.repo.as_deref().unwrap_or("");
            if env_repo != filter {
                saw_other_repo = true;
            }
        }
        if let Ok(ts) = DateTime::parse_from_rfc3339(&env.occurred_at) {
            let utc = ts.with_timezone(&Utc);
            newest = Some(match newest {
                None => utc,
                Some(prev) => prev.max(utc),
            });
        }
    }
    if saw_partial_tail {
        return Ok(FileVerdict::Partial);
    }
    if !saw_any {
        return Ok(FileVerdict::Unreadable);
    }
    if saw_other_repo {
        // v1 conservative: any non-matching repo in the file pins
        // the file. A future rev can rewrite the file with the
        // surviving rows; that needs a Parquet writer round-trip and
        // is out of scope here.
        return Ok(FileVerdict::Keep);
    }
    match newest {
        Some(ts) if ts < cutoff => Ok(FileVerdict::Delete),
        _ => Ok(FileVerdict::Keep),
    }
}

/// Phase12c §2 — single-source-of-truth helper for "this zstd error
/// means the writer is mid-flush, treat as end-of-stream".
///
/// zstd raises one of three messages when the frame the writer is
/// currently flushing is incomplete; the archive walker (this
/// module), the `/v1/admin/forget` envelope walker
/// (`cortex-api::admin_forget`), and any future purger must agree on
/// the predicate so the live current-hour file is treated
/// identically across surfaces. Adding a fourth call site means
/// importing this helper, not copy-pasting the message list.
pub fn is_live_partial_frame(err: &std::io::Error) -> bool {
    let msg = err.to_string();
    msg.contains("incomplete frame")
        || msg.contains("Unknown frame")
        || msg.contains("frame descriptor")
}

#[cfg(test)]
mod is_live_partial_frame_tests {
    //! Phase12c §2.5 — pin the predicate's classification table so
    //! adding a fourth zstd error kind is a deliberate code change.
    //! The four scenarios match the operational shapes the live
    //! current-hour writer produces, plus the everyday I/O errors
    //! that must NOT be classified as "live writer".
    use super::is_live_partial_frame;

    #[test]
    fn complete_frame_error_is_not_partial() {
        // A "real" I/O error (UnexpectedEof, not zstd-frame-shaped)
        // must NOT be treated as a live-writer marker — that would
        // mask actual corruption.
        let err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected end of file");
        assert!(!is_live_partial_frame(&err));
    }

    #[test]
    fn incomplete_frame_message_is_partial() {
        // The shape zstd raises mid-flush when the trailing frame's
        // body bytes are present but the checksum is missing.
        let err = std::io::Error::new(std::io::ErrorKind::InvalidData, "incomplete frame");
        assert!(is_live_partial_frame(&err));
    }

    #[test]
    fn unknown_frame_message_is_partial() {
        // zstd raises this when the magic bytes do not parse as a
        // known frame header — happens when the writer has only
        // flushed the first byte or two of the next frame.
        let err = std::io::Error::new(std::io::ErrorKind::InvalidData, "Unknown frame magic bytes");
        assert!(is_live_partial_frame(&err));
    }

    #[test]
    fn frame_descriptor_message_is_partial() {
        // The third operational shape — a malformed frame descriptor.
        // Same root cause: the writer is mid-flush.
        let err = std::io::Error::new(std::io::ErrorKind::InvalidData, "Wrong frame descriptor");
        assert!(is_live_partial_frame(&err));
    }

    #[test]
    fn permission_denied_is_not_partial() {
        // Sanity — a permission error is operational, never a
        // "live writer" signal.
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        assert!(!is_live_partial_frame(&err));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Kind, Stream, Turn};
    use std::collections::BTreeMap;
    use std::io::Write;

    fn envelope(event_id: &str, repo: &str, occurred_at: &str) -> Envelope {
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: occurred_at.to_string(),
            ingested_at: None,
            session_id: "01HSESS00000000000000000000".to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: Some(repo.to_string()),
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
            class_level: None,
            class_compartments: None,
        }
    }

    fn write_archive(home: &Path, rel_dir: &str, name: &str, envelopes: &[Envelope]) -> PathBuf {
        let dir = home.join(rel_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
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
    fn purge_before_no_archive_returns_zeroed_report() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        // Don't create `events/` at all.
        let report = purge_before(dir.path(), cutoff, false, None).unwrap();
        assert_eq!(report.files_deleted, 0);
        assert_eq!(report.bytes_reclaimed, 0);
        assert_eq!(report.partitions_visited, 0);
    }

    #[test]
    fn purge_before_deletes_every_file_when_all_old() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-04-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        write_archive(
            dir.path(),
            "events/year=2025/month=12/day=01/hour=00",
            "raw-00000.parquet",
            &[envelope("E_OLD_A", "cortex", "2025-12-01T00:00:00Z")],
        );
        write_archive(
            dir.path(),
            "events/year=2026/month=01/day=15/hour=10",
            "raw-00000.parquet",
            &[envelope("E_OLD_B", "cortex", "2026-01-15T10:00:00Z")],
        );

        let report = purge_before(dir.path(), cutoff, false, None).unwrap();
        assert_eq!(report.files_deleted, 2);
        assert_eq!(report.partitions_visited, 2);
        assert_eq!(report.files_kept, 0);
        assert!(report.bytes_reclaimed > 0);
        // Files actually gone.
        let leftover: Vec<_> = walkdir(&dir.path().join("events"))
            .into_iter()
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("parquet"))
            .collect();
        assert!(
            leftover.is_empty(),
            "expected zero parquet files, got {leftover:?}"
        );
    }

    #[test]
    fn purge_before_mixed_keeps_files_with_recent_envelopes() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-04-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let old_path = write_archive(
            dir.path(),
            "events/year=2025/month=12/day=01/hour=00",
            "raw-00000.parquet",
            &[envelope("E_OLD", "cortex", "2025-12-01T00:00:00Z")],
        );
        let mixed_path = write_archive(
            dir.path(),
            "events/year=2026/month=05/day=01/hour=00",
            "raw-00000.parquet",
            &[
                envelope("E_OLD_2", "cortex", "2025-11-15T00:00:00Z"),
                envelope("E_NEW", "cortex", "2026-05-01T00:00:00Z"),
            ],
        );
        let report = purge_before(dir.path(), cutoff, false, None).unwrap();
        assert_eq!(report.files_deleted, 1);
        assert_eq!(report.files_kept, 1);
        assert!(!old_path.exists(), "old file should have been deleted");
        assert!(mixed_path.exists(), "mixed file should be retained");
    }

    #[test]
    fn purge_before_partial_frame_guard_keeps_live_writer_file() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-04-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        // Build a file with a valid envelope, then truncate the
        // closing zstd footer so the reader hits "incomplete frame"
        // mid-stream — the same shape a live current-hour writer
        // produces between flushes.
        let path = write_archive(
            dir.path(),
            "events/year=2025/month=12/day=01/hour=00",
            "raw-00000.parquet",
            &[envelope("E_OLD", "cortex", "2025-12-01T00:00:00Z")],
        );
        let bytes = std::fs::read(&path).unwrap();
        // Drop the trailing 4 bytes — corrupts the zstd frame
        // checksum so the decoder raises a "incomplete frame"-class
        // error mid-line.
        std::fs::write(&path, &bytes[..bytes.len().saturating_sub(4)]).unwrap();
        let report = purge_before(dir.path(), cutoff, false, None).unwrap();
        // The exact verdict depends on whether the truncation hit
        // before or after we read any complete envelope. Either
        // partial or unreadable is acceptable — the invariant is
        // "the file is NOT deleted".
        assert_eq!(report.files_deleted, 0);
        assert!(report.files_partial + report.files_unreadable >= 1);
        assert!(path.exists(), "live-writer-shape file must be preserved");
    }

    #[test]
    fn purge_before_dry_run_does_not_delete() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-04-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let path = write_archive(
            dir.path(),
            "events/year=2025/month=12/day=01/hour=00",
            "raw-00000.parquet",
            &[envelope("E_OLD", "cortex", "2025-12-01T00:00:00Z")],
        );
        let report = purge_before(dir.path(), cutoff, true, None).unwrap();
        assert_eq!(report.files_deleted, 1, "dry-run still tallies");
        assert!(report.dry_run);
        assert!(path.exists(), "dry-run must not actually delete");
    }

    #[test]
    fn purge_before_repo_filter_pins_files_with_other_repos() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = DateTime::parse_from_rfc3339("2026-04-01T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        // File mixes cortex + nexus envelopes — repo-filtered purge
        // for "cortex" must keep the file because deleting it would
        // also drop the nexus rows.
        let path = write_archive(
            dir.path(),
            "events/year=2025/month=12/day=01/hour=00",
            "raw-00000.parquet",
            &[
                envelope("E_CORTEX", "cortex", "2025-12-01T00:00:00Z"),
                envelope("E_NEXUS", "nexus", "2025-12-01T00:00:00Z"),
            ],
        );
        let report = purge_before(dir.path(), cutoff, false, Some("cortex")).unwrap();
        assert_eq!(report.files_deleted, 0);
        assert_eq!(report.files_kept, 1);
        assert!(
            path.exists(),
            "mixed-repo file must be preserved when filter is set"
        );
        assert_eq!(report.repo_filter.as_deref(), Some("cortex"));
    }

    fn walkdir(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if !root.exists() {
            return out;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out
    }
}
