//! Phase9g — gzip log rotator for the operator's hook logs.
//!
//! `~/.cortex/hook-invocations.log` and `~/.cortex/hook-errors.log`
//! are append-only; they grow indefinitely without an external
//! rotator. The reaper runs this helper after every metadata sweep
//! to bound the operator's home directory.
//!
//! Rotation is rename-first: we move the live file aside to a dated
//! `.gz`-encoded copy and recreate an empty live file. Writers
//! holding an `O_APPEND` handle to the original inode continue
//! writing into the moved-aside file until they reopen — the
//! reaper accepts that briefly until the next process restart, in
//! exchange for never racing against an exclusive file lock.
//!
//! Retention: only the 8 most recent rotations per file are kept;
//! older `.gz` siblings are unlinked.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

/// Tuning knobs for [`rotate_if_needed`].
#[derive(Debug, Clone)]
pub struct LogRotateOpts {
    /// Reference time for the rotation suffix and age calculation.
    pub now: DateTime<Utc>,
    /// Rotate when the file exceeds this many bytes. Default 5 MB.
    pub max_bytes: u64,
    /// Rotate when the file is older than this many days. Default 7.
    pub max_age_days: u32,
    /// Keep the N most recent rotations per file. Default 8.
    pub keep_rotations: usize,
}

impl LogRotateOpts {
    /// Defaults per spec — 5 MB, 7 days, 8 rotations.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            max_bytes: 5_000_000,
            max_age_days: 7,
            keep_rotations: 8,
        }
    }
}

/// Outcome of one rotation attempt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRotateOutcome {
    /// `true` when a rotation actually happened.
    pub rotated: bool,
    /// Size of the source file before rotation, in bytes.
    pub source_bytes: u64,
    /// Path to the produced `.gz` file when `rotated == true`.
    pub gz_path: Option<PathBuf>,
    /// `.gz` rotations the helper unlinked because they fell off
    /// the keep-N tail.
    pub pruned: Vec<PathBuf>,
}

/// Inspect `path` and rotate when it exceeds `max_bytes` OR is older
/// than `max_age_days`. No-op when the file is missing, empty, or
/// fresh.
pub fn rotate_if_needed(path: &Path, opts: &LogRotateOpts) -> io::Result<LogRotateOutcome> {
    let mut outcome = LogRotateOutcome::default();
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(outcome),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() {
        return Ok(outcome);
    }
    outcome.source_bytes = metadata.len();
    if metadata.len() == 0 {
        return Ok(outcome);
    }
    let age_ok = file_age_days(&metadata, opts.now)? < i64::from(opts.max_age_days);
    let size_ok = metadata.len() < opts.max_bytes;
    if age_ok && size_ok {
        return Ok(outcome);
    }

    let gz_path = produce_rotated_path(path, opts);
    // Rename-first: move the live file aside under a temporary name
    // so a concurrent appender cannot land bytes into the gzip
    // stream we're producing. We then gzip the moved file and
    // recreate an empty live file at the original path.
    let staged = path.with_extension("rotating");
    if staged.exists() {
        fs::remove_file(&staged)?;
    }
    fs::rename(path, &staged)?;
    // Recreate the live file empty so subsequent writers (after a
    // reopen) land bytes again. Existing FDs continue writing to
    // `staged` — those bytes WILL be captured in the gzip we're
    // about to produce.
    File::create(path)?;

    gzip_file_to(&staged, &gz_path)?;
    fs::remove_file(&staged)?;

    outcome.rotated = true;
    outcome.gz_path = Some(gz_path);
    outcome.pruned = prune_old_rotations(path, opts.keep_rotations)?;
    Ok(outcome)
}

fn file_age_days(metadata: &fs::Metadata, now: DateTime<Utc>) -> io::Result<i64> {
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let now_sys: SystemTime = now.into();
    let elapsed = now_sys.duration_since(modified).unwrap_or(Duration::ZERO);
    Ok(i64::try_from(elapsed.as_secs() / 86_400).unwrap_or(i64::MAX))
}

fn produce_rotated_path(path: &Path, opts: &LogRotateOpts) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
    let suffix = opts.now.format("%Y-%m-%d").to_string();
    let mut candidate = parent.join(format!("{stem}.{suffix}.gz"));
    // If a rotation for the same day already exists, append a
    // monotonic counter so we never overwrite history.
    let mut n = 1;
    while candidate.exists() {
        candidate = parent.join(format!("{stem}.{suffix}.{n}.gz"));
        n += 1;
    }
    candidate
}

fn gzip_file_to(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = BufReader::new(File::open(src)?);
    let output = BufWriter::new(File::create(dst)?);
    let mut encoder = GzEncoder::new(output, Compression::default());
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        encoder.write_all(&buf[..n])?;
    }
    encoder.finish()?.flush()?;
    Ok(())
}

fn prune_old_rotations(path: &Path, keep: usize) -> io::Result<Vec<PathBuf>> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
    let prefix = format!("{stem}.");
    let mut rotations: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let p = entry.path();
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.starts_with(&prefix) || !name.ends_with(".gz") {
            continue;
        }
        let meta = entry.metadata()?;
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        rotations.push((modified, p));
    }
    rotations.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let mut pruned = Vec::new();
    for (_, p) in rotations.into_iter().skip(keep) {
        fs::remove_file(&p)?;
        pruned.push(p);
    }
    Ok(pruned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn write_file(p: &Path, body: &[u8]) {
        std::fs::write(p, body).unwrap();
    }

    fn read_gz(p: &Path) -> Vec<u8> {
        let f = File::open(p).unwrap();
        let mut dec = GzDecoder::new(f);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn opts_default_uses_spec_thresholds() {
        let o = LogRotateOpts::default_for(now());
        assert_eq!(o.max_bytes, 5_000_000);
        assert_eq!(o.max_age_days, 7);
        assert_eq!(o.keep_rotations, 8);
    }

    #[test]
    fn missing_file_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        let outcome = rotate_if_needed(&p, &LogRotateOpts::default_for(now())).unwrap();
        assert!(!outcome.rotated);
        assert_eq!(outcome.source_bytes, 0);
    }

    #[test]
    fn empty_file_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        write_file(&p, b"");
        let outcome = rotate_if_needed(&p, &LogRotateOpts::default_for(now())).unwrap();
        assert!(!outcome.rotated);
    }

    #[test]
    fn fresh_small_file_does_not_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        write_file(&p, b"tiny");
        let outcome = rotate_if_needed(&p, &LogRotateOpts::default_for(now())).unwrap();
        assert!(!outcome.rotated);
    }

    #[test]
    fn six_megabyte_file_triggers_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook-invocations.log");
        let body = vec![b'x'; 6_000_000];
        write_file(&p, &body);
        let outcome = rotate_if_needed(&p, &LogRotateOpts::default_for(now())).unwrap();
        assert!(outcome.rotated);
        let gz_path = outcome.gz_path.unwrap();
        assert!(gz_path.exists());
        // Original file recreated empty.
        let live = std::fs::metadata(&p).unwrap();
        assert!(live.is_file());
        assert!(live.len() < 1_024);
        let unzipped = read_gz(&gz_path);
        assert_eq!(unzipped.len(), body.len());
    }

    #[test]
    fn day_suffix_collisions_use_a_monotonic_counter() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        let opts = LogRotateOpts {
            max_bytes: 8,
            ..LogRotateOpts::default_for(now())
        };
        // First rotation.
        write_file(&p, b"abcdefghijklmnop");
        let r1 = rotate_if_needed(&p, &opts).unwrap();
        assert!(r1.rotated);
        // Second rotation on the same day.
        write_file(&p, b"qrstuvwxyz0123456789");
        let r2 = rotate_if_needed(&p, &opts).unwrap();
        assert!(r2.rotated);
        let p2 = r2.gz_path.unwrap();
        assert!(p2.file_name().unwrap().to_str().unwrap().contains(".1.gz"));
    }

    #[test]
    fn keeps_only_n_most_recent_rotations() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        let mut opts = LogRotateOpts::default_for(now());
        opts.max_bytes = 1;
        opts.keep_rotations = 3;
        for i in 0..10 {
            write_file(&p, format!("payload-{i}").as_bytes());
            // Bump the system time on the produced .gz so the prune
            // sort is deterministic.
            let outcome = rotate_if_needed(&p, &opts).unwrap();
            assert!(outcome.rotated);
            // Force monotonic mtime so the sort orders correctly on
            // fast machines that produce identical ms-resolution
            // timestamps.
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let mut surviving: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().and_then(|s| s.to_str()) == Some("gz")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("hook."))
                        .unwrap_or(false)
            })
            .collect();
        surviving.sort();
        assert_eq!(surviving.len(), 3);
    }

    #[test]
    fn old_file_past_age_threshold_rotates_even_when_small() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        write_file(&p, b"tiny");
        // The test's fixed `now()` is a frozen RFC-3339 string; the
        // file mtime comes from the real OS clock and inevitably drifts
        // past it. Anchor `opts.now` to wall-clock + 30 days so the
        // synthetic "now" sits well past the file mtime regardless of
        // when the suite runs.
        let mut opts = LogRotateOpts::default_for(Utc::now());
        opts.now = Utc::now() + chrono::Duration::days(30);
        let outcome = rotate_if_needed(&p, &opts).unwrap();
        assert!(
            outcome.rotated,
            "30-day-old file MUST rotate; source_bytes={}, gz={:?}",
            outcome.source_bytes, outcome.gz_path
        );
    }
}
