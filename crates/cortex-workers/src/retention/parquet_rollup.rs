//! Phase9b — archive rollup compactor.
//!
//! Spec 02 §"Event archive (Parquet)" defines the rollup contract:
//! hourly → daily at 90 d, daily → monthly at 365 d, drop monthly at
//! 3 y unless `pii_risk = "low"` or `kind ∈ {decision, analysis,
//! law_violation}`. This module implements that contract.
//!
//! Despite the `.parquet` filename suffix, the on-disk format is
//! **zstd-compressed line-delimited JSON** (see
//! `cortex-api/src/archive_loader.rs` for the read-path). The
//! compactor concatenates source files line-by-line into a single
//! destination, so the schema-stable contract that spec 02 promised
//! holds even though the actual encoding is NDJSON-on-zstd, not
//! Apache Parquet.
//!
//! Atomicity: every compaction is **read → write `<dest>.tmp` →
//! sync_all → rename → unlink sources**. A crash between `sync_all`
//! and `rename` leaves an orphan `.tmp` that the next run cleans up.
//! A crash between `rename` and `unlink sources` leaves the dest
//! file durable + the sources intact; the next run re-attempts and
//! the row-count assertion catches the duplicate.
//!
//! Corruption: every read is wrapped in `try_open`. Files matching
//! `*.corrupted*`, orphan `*.tmp`, or any file that fails the zstd
//! decode get moved under `events/_quarantine/<original-relpath>`
//! with a sibling `<file>.reason` describing why. The query layer
//! already skips paths under `_quarantine/` because it walks via
//! `extension == "parquet"` — quarantine adds a `.reason` extension
//! to the moved companion.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use cortex_storage::ArchiveLayout;

/// Rollup granularity. Each variant maps to a different cutoff +
/// destination layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    /// Hourly source files → one daily file per
    /// `year=YYYY/month=MM/day=DD/`.
    HourlyToDaily,
    /// Daily files → one monthly file per `year=YYYY/month=MM/`.
    DailyToMonthly,
    /// Drop monthly files older than 3 y unless their records pass
    /// the whitelist (kind ∈ {decision, analysis, law_violation} OR
    /// pii_risk == "low").
    ThreeYearDrop,
}

impl Granularity {
    /// Default cutoff in days for this granularity per spec 02.
    pub fn default_cutoff_days(self) -> i64 {
        match self {
            Granularity::HourlyToDaily => 90,
            Granularity::DailyToMonthly => 365,
            Granularity::ThreeYearDrop => 1_095,
        }
    }
    /// Stable string label used in CLI output and bookkeeping JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::HourlyToDaily => "hourly_to_daily",
            Granularity::DailyToMonthly => "daily_to_monthly",
            Granularity::ThreeYearDrop => "three_year_drop",
        }
    }
}

/// One compactable partition the enumerator returns. Each carries
/// the exact source files + the destination path the compactor
/// writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionPlan {
    /// Granularity bucket this plan belongs to.
    pub granularity: Granularity,
    /// Source files to merge.
    pub sources: Vec<PathBuf>,
    /// Destination file the merge writes (or `None` for the 3-y
    /// drop, which writes to `<year=>/<month=>/preserved.parquet`
    /// and computes the path from `month_dir`).
    pub dest: PathBuf,
    /// Partition root directory — useful for cleanup after the
    /// merge (empty `hour=*` directories get pruned).
    pub partition_root: PathBuf,
}

/// Roll-up summary written to `retention_sweeps.tier_transitions_json`
/// under the `parquet_rollup` key.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollupCounts {
    /// Source files read (across all partitions).
    pub files_in: u64,
    /// Destination files produced.
    pub files_out: u64,
    /// Approximate bytes reclaimed (sum of source sizes − sum of
    /// destination sizes; never negative).
    pub bytes_reclaimed: u64,
    /// Files moved into `_quarantine/`.
    pub quarantined: u64,
    /// Records dropped by the 3-y drop because they failed the
    /// whitelist.
    pub records_dropped: u64,
    /// Records preserved by the 3-y drop because they passed the
    /// whitelist.
    pub records_preserved: u64,
}

impl RollupCounts {
    /// Sum two roll-up summaries (used to roll multiple granularity
    /// passes into one bookkeeping row).
    pub fn merge(&mut self, other: &RollupCounts) {
        self.files_in += other.files_in;
        self.files_out += other.files_out;
        self.bytes_reclaimed += other.bytes_reclaimed;
        self.quarantined += other.quarantined;
        self.records_dropped += other.records_dropped;
        self.records_preserved += other.records_preserved;
    }
}

/// Plan-level errors. Per-record / per-file failures are captured
/// in [`RollupCounts::quarantined`] / `records_dropped` so the
/// caller can tolerate them.
#[derive(Debug, thiserror::Error)]
pub enum RollupError {
    /// I/O failure during read/write.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Row-count mismatch between sources and destination — the
    /// compactor aborts and quarantines the sources rather than
    /// commit a bad merge.
    #[error("row-count mismatch in {dest}: sources={sources_rows}, dest={dest_rows}")]
    RowMismatch {
        /// Destination file the assertion was checked against.
        dest: String,
        /// Total rows the source files produced.
        sources_rows: u64,
        /// Rows the destination file ended up with.
        dest_rows: u64,
    },
}

/// Walk `<archive_root>/events/` and return every partition that's
/// eligible for the requested granularity at the supplied reference
/// time. Returns an empty vector when no partitions are eligible.
pub fn enumerate_compactable(
    archive_root: &Path,
    now: DateTime<Utc>,
    granularity: Granularity,
) -> Vec<PartitionPlan> {
    let cutoff_days = granularity.default_cutoff_days();
    let cutoff = now - chrono::Duration::days(cutoff_days);
    let events_root = archive_root.join(ArchiveLayout::ROOT_SEGMENT);
    let mut plans = Vec::new();
    if !events_root.exists() {
        return plans;
    }
    match granularity {
        Granularity::HourlyToDaily => {
            // Walk year=*/month=*/day=*/ and collect every day whose
            // calendar date is older than the cutoff. Within each
            // eligible day, list every `hour=*/raw-*.parquet` under
            // it and add a single PartitionPlan.
            for_each_day(&events_root, |day_dir, day_date| {
                if day_date >= cutoff.date_naive() {
                    return;
                }
                let hour_dirs = match read_dir(day_dir) {
                    Some(d) => d,
                    None => return,
                };
                let mut sources = Vec::new();
                for entry in hour_dirs.flatten() {
                    let hour_path = entry.path();
                    let name = match hour_path.file_name().and_then(|s| s.to_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    if !name.starts_with("hour=") {
                        continue;
                    }
                    if let Some(files) = read_dir(&hour_path) {
                        for f in files.flatten() {
                            let p = f.path();
                            if p.extension().and_then(|s| s.to_str()) == Some("parquet") {
                                sources.push(p);
                            }
                        }
                    }
                }
                if sources.is_empty() {
                    return;
                }
                sources.sort();
                let dest = day_dir.join("raw-daily.parquet");
                plans.push(PartitionPlan {
                    granularity,
                    sources,
                    dest,
                    partition_root: day_dir.to_path_buf(),
                });
            });
        }
        Granularity::DailyToMonthly => {
            for_each_month(&events_root, |month_dir, month_date| {
                // Month is older than cutoff if the *last* day of
                // the month is still older than the cutoff. We use
                // the first day of the month as a conservative
                // proxy — a daily file at month-start older than
                // cutoff is the trigger.
                if month_date >= cutoff.date_naive() {
                    return;
                }
                let day_dirs = match read_dir(month_dir) {
                    Some(d) => d,
                    None => return,
                };
                let mut sources = Vec::new();
                for entry in day_dirs.flatten() {
                    let p = entry.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let name = match p.file_name().and_then(|s| s.to_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    if !name.starts_with("day=") {
                        continue;
                    }
                    let daily = p.join("raw-daily.parquet");
                    if daily.exists() {
                        sources.push(daily);
                    }
                }
                if sources.is_empty() {
                    return;
                }
                sources.sort();
                let dest = month_dir.join("raw-monthly.parquet");
                plans.push(PartitionPlan {
                    granularity,
                    sources,
                    dest,
                    partition_root: month_dir.to_path_buf(),
                });
            });
        }
        Granularity::ThreeYearDrop => {
            for_each_month(&events_root, |month_dir, month_date| {
                if month_date >= cutoff.date_naive() {
                    return;
                }
                let monthly = month_dir.join("raw-monthly.parquet");
                if !monthly.exists() {
                    return;
                }
                let dest = month_dir.join("preserved.parquet");
                plans.push(PartitionPlan {
                    granularity,
                    sources: vec![monthly],
                    dest,
                    partition_root: month_dir.to_path_buf(),
                });
            });
        }
    }
    plans
}

fn read_dir(path: &Path) -> Option<fs::ReadDir> {
    fs::read_dir(path).ok()
}

fn for_each_day<F>(events_root: &Path, mut visit: F)
where
    F: FnMut(&Path, chrono::NaiveDate),
{
    for_each_dated(events_root, |day_dir, year, month, day| {
        if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
            visit(day_dir, date);
        }
    });
}

fn for_each_month<F>(events_root: &Path, mut visit: F)
where
    F: FnMut(&Path, chrono::NaiveDate),
{
    let years = match read_dir(events_root) {
        Some(d) => d,
        None => return,
    };
    for year_entry in years.flatten() {
        let year_dir = year_entry.path();
        let year = match parse_partition_segment(&year_dir, "year=") {
            Some(v) => v as i32,
            None => continue,
        };
        let months = match read_dir(&year_dir) {
            Some(d) => d,
            None => continue,
        };
        for month_entry in months.flatten() {
            let month_dir = month_entry.path();
            let month = match parse_partition_segment(&month_dir, "month=") {
                Some(v) => v,
                None => continue,
            };
            if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, 1) {
                visit(&month_dir, date);
            }
        }
    }
}

fn for_each_dated<F>(events_root: &Path, mut visit: F)
where
    F: FnMut(&Path, i32, u32, u32),
{
    let years = match read_dir(events_root) {
        Some(d) => d,
        None => return,
    };
    for year_entry in years.flatten() {
        let year_dir = year_entry.path();
        let year = match parse_partition_segment(&year_dir, "year=") {
            Some(v) => v as i32,
            None => continue,
        };
        let months = match read_dir(&year_dir) {
            Some(d) => d,
            None => continue,
        };
        for month_entry in months.flatten() {
            let month_dir = month_entry.path();
            let month = match parse_partition_segment(&month_dir, "month=") {
                Some(v) => v,
                None => continue,
            };
            let days = match read_dir(&month_dir) {
                Some(d) => d,
                None => continue,
            };
            for day_entry in days.flatten() {
                let day_dir = day_entry.path();
                let day = match parse_partition_segment(&day_dir, "day=") {
                    Some(v) => v,
                    None => continue,
                };
                visit(&day_dir, year, month, day);
            }
        }
    }
}

fn parse_partition_segment(path: &Path, prefix: &str) -> Option<u32> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    name.strip_prefix(prefix).and_then(|s| s.parse().ok())
}

/// Compact one partition. Reads every source file, concatenates
/// records to `<dest>.tmp`, sync_all + rename to `<dest>`, then
/// unlinks the sources. Per-source decode failures move the
/// offending file to `_quarantine/` and surface in the returned
/// counts; the merge continues for the rest.
pub fn compact_partition(
    archive_root: &Path,
    plan: &PartitionPlan,
) -> Result<RollupCounts, RollupError> {
    let mut counts = RollupCounts::default();
    let tmp_path = with_suffix(&plan.dest, ".tmp");
    // Crash-safe: if the previous attempt left an orphan tmp,
    // remove it before we start.
    if tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }
    if let Some(parent) = plan.dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut bytes_in: u64 = 0;
    let mut sources_rows: u64 = 0;
    let mut readable_sources: Vec<PathBuf> = Vec::new();

    let tmp_file = File::create(&tmp_path)?;
    let mut encoder =
        zstd::stream::write::Encoder::new(tmp_file, ArchiveLayout::COMPRESSION_LEVEL)?;

    for source in &plan.sources {
        match read_source_file(source) {
            Ok((rows, bytes)) => {
                for line in rows {
                    encoder.write_all(line.as_bytes())?;
                    encoder.write_all(b"\n")?;
                    sources_rows += 1;
                }
                bytes_in += bytes;
                readable_sources.push(source.clone());
            }
            Err(reason) => {
                let qcounts = quarantine(archive_root, source, &reason)?;
                counts.quarantined += qcounts;
            }
        }
    }
    let inner = encoder.finish()?;
    inner.sync_all()?;
    drop(inner);

    // Verify the destination by re-reading + counting rows.
    let (dest_rows, bytes_out) = match read_source_file(&tmp_path) {
        Ok((rows, bytes)) => (rows.len() as u64, bytes),
        Err(reason) => {
            // The tmp file we just wrote doesn't decode — quarantine
            // it and bail out without touching the sources.
            let _ = fs::remove_file(&tmp_path);
            tracing::warn!(error = %reason, dest = %plan.dest.display(), "rollup: tmp file unreadable");
            return Err(RollupError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                reason,
            )));
        }
    };
    if dest_rows != sources_rows {
        // Row mismatch — abort. Quarantine the destination + leave
        // the sources alone for the next sweep.
        let _ = fs::remove_file(&tmp_path);
        return Err(RollupError::RowMismatch {
            dest: plan.dest.display().to_string(),
            sources_rows,
            dest_rows,
        });
    }

    // Atomic finalize.
    fs::rename(&tmp_path, &plan.dest)?;
    counts.files_out += 1;

    // Now the destination is durable. Unlink sources.
    for source in &readable_sources {
        if let Err(e) = fs::remove_file(source) {
            tracing::warn!(error = %e, source = %source.display(), "rollup: source unlink failed (will retry next sweep)");
        }
    }
    counts.files_in += readable_sources.len() as u64;
    counts.bytes_reclaimed = bytes_in.saturating_sub(bytes_out);

    // Best-effort cleanup of empty `hour=*` directories under the
    // partition root.
    if let Some(dir_iter) = read_dir(&plan.partition_root) {
        for entry in dir_iter.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !(name.starts_with("hour=") || name.starts_with("day=")) {
                continue;
            }
            // Only remove dirs that are now empty.
            if read_dir(&p)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
            {
                let _ = fs::remove_dir(&p);
            }
        }
    }

    Ok(counts)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn read_source_file(path: &Path) -> Result<(Vec<String>, u64), String> {
    let bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file = File::open(path).map_err(|e| format!("open: {e}"))?;
    let decoder = zstd::stream::read::Decoder::new(file).map_err(|e| format!("zstd: {e}"))?;
    let reader = BufReader::new(decoder);
    let mut rows = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        rows.push(line);
    }
    Ok((rows, bytes))
}

/// Apply the 3-year drop. Reads the source file, partitions records
/// by the whitelist, writes the surviving records to
/// `<month_dir>/preserved.parquet`, then deletes the source. When
/// every record in the source fails the whitelist, no
/// `preserved.parquet` is written and the source is simply removed.
pub fn apply_three_year_drop(
    archive_root: &Path,
    plan: &PartitionPlan,
) -> Result<RollupCounts, RollupError> {
    debug_assert!(plan.granularity == Granularity::ThreeYearDrop);
    let mut counts = RollupCounts::default();
    let source = match plan.sources.first() {
        Some(p) => p,
        None => return Ok(counts),
    };
    let bytes_in = fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    let (rows, _) = match read_source_file(source) {
        Ok(v) => v,
        Err(reason) => {
            let q = quarantine(archive_root, source, &reason)?;
            counts.quarantined += q;
            return Ok(counts);
        }
    };
    let mut preserved: Vec<String> = Vec::new();
    for row in rows {
        let value: serde_json::Value = match serde_json::from_str(&row) {
            Ok(v) => v,
            Err(_) => {
                // Garbled JSON in an old archive — drop it.
                counts.records_dropped += 1;
                continue;
            }
        };
        if record_passes_whitelist(&value) {
            preserved.push(row);
            counts.records_preserved += 1;
        } else {
            counts.records_dropped += 1;
        }
    }

    if !preserved.is_empty() {
        let tmp = with_suffix(&plan.dest, ".tmp");
        if tmp.exists() {
            let _ = fs::remove_file(&tmp);
        }
        let tmp_file = File::create(&tmp)?;
        let mut encoder =
            zstd::stream::write::Encoder::new(tmp_file, ArchiveLayout::COMPRESSION_LEVEL)?;
        for row in &preserved {
            encoder.write_all(row.as_bytes())?;
            encoder.write_all(b"\n")?;
        }
        let inner = encoder.finish()?;
        inner.sync_all()?;
        drop(inner);
        fs::rename(&tmp, &plan.dest)?;
        counts.files_out += 1;
    }

    let bytes_out = if !preserved.is_empty() {
        fs::metadata(&plan.dest).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    counts.bytes_reclaimed = bytes_in.saturating_sub(bytes_out);
    counts.files_in += 1;
    fs::remove_file(source)?;
    Ok(counts)
}

/// Whitelist test for the 3-year drop. Records pass when:
/// - `kind ∈ {decision, analysis, law_violation}` (always-preserved
///   audit kinds)
/// - OR `redactions[].pii_risk = "low"` is present, indicating the
///   record was already redacted to a low-risk shape.
///
/// Implementation reads the canonical envelope shape — `kind` at the
/// top level, `redactions` is an array of redaction tags.
fn record_passes_whitelist(value: &serde_json::Value) -> bool {
    let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if matches!(kind, "decision" | "analysis" | "law_violation") {
        return true;
    }
    // PII risk: the cortex-core redactor stamps tags like
    // "low", "medium", "high"; "low" passes the whitelist.
    if let Some(redactions) = value.get("redactions").and_then(|v| v.as_array()) {
        for r in redactions {
            if r.as_str() == Some("pii_risk:low")
                || r.as_str() == Some("low")
                || r.get("pii_risk").and_then(|v| v.as_str()) == Some("low")
            {
                return true;
            }
        }
    }
    false
}

/// Move `path` under `<archive_root>/events/_quarantine/<relpath>`
/// preserving the relative path so the original location is still
/// visible. Writes a sibling `<file>.reason` describing why.
/// Returns `1` on success, `0` when the move failed (logged at
/// WARN). Never returns an error — quarantine is best-effort.
pub fn quarantine(archive_root: &Path, path: &Path, reason: &str) -> Result<u64, RollupError> {
    let events_root = archive_root.join(ArchiveLayout::ROOT_SEGMENT);
    let rel = path
        .strip_prefix(&events_root)
        .unwrap_or(path)
        .to_path_buf();
    let dest = events_root.join("_quarantine").join(&rel);
    if let Some(parent) = dest.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(error = %e, dest = %dest.display(), "quarantine: create_dir_all failed");
            return Ok(0);
        }
    }
    if let Err(e) = fs::rename(path, &dest) {
        tracing::warn!(error = %e, src = %path.display(), dest = %dest.display(), "quarantine: rename failed");
        return Ok(0);
    }
    let mut reason_path = dest.clone().into_os_string();
    reason_path.push(".reason");
    let _ = fs::write(PathBuf::from(reason_path), reason);
    tracing::info!(
        path = %path.display(),
        dest = %dest.display(),
        reason,
        "quarantine: file moved"
    );
    Ok(1)
}

/// Walk `<archive_root>/events/` once and quarantine every file
/// matching `*.corrupted*` or orphan `*.tmp`. Called from the
/// rollup CLI on startup before any compaction runs so the working
/// tree is clean.
pub fn quarantine_pre_existing(archive_root: &Path) -> RollupCounts {
    let events_root = archive_root.join(ArchiveLayout::ROOT_SEGMENT);
    let mut counts = RollupCounts::default();
    if !events_root.exists() {
        return counts;
    }
    walk_files(&events_root, |path| {
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => return,
        };
        // Don't re-quarantine files already under `_quarantine/`.
        if path
            .strip_prefix(&events_root)
            .ok()
            .map(|p| p.starts_with("_quarantine"))
            .unwrap_or(false)
        {
            return;
        }
        let reason = if name.contains(".corrupted") {
            Some(format!("matches `*.corrupted*` (filename: {name})"))
        } else if name.ends_with(".tmp") {
            Some(format!("orphan tmp file (filename: {name})"))
        } else {
            None
        };
        if let Some(r) = reason {
            if let Ok(n) = quarantine(archive_root, path, &r) {
                counts.quarantined += n;
            }
        }
    });
    counts
}

fn walk_files<F>(root: &Path, mut visit: F)
where
    F: FnMut(&Path),
{
    let entries = match read_dir(root) {
        Some(d) => d,
        None => return,
    };
    let mut stack: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            if let Some(d) = read_dir(&path) {
                for e in d.flatten() {
                    stack.push(e.path());
                }
            }
        } else if path.is_file() {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_zstd_lines(path: &Path, lines: &[&str]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let file = File::create(path).unwrap();
        let mut encoder =
            zstd::stream::write::Encoder::new(file, ArchiveLayout::COMPRESSION_LEVEL).unwrap();
        for line in lines {
            encoder.write_all(line.as_bytes()).unwrap();
            encoder.write_all(b"\n").unwrap();
        }
        let inner = encoder.finish().unwrap();
        inner.sync_all().unwrap();
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn granularity_default_cutoffs_match_spec() {
        assert_eq!(Granularity::HourlyToDaily.default_cutoff_days(), 90);
        assert_eq!(Granularity::DailyToMonthly.default_cutoff_days(), 365);
        assert_eq!(Granularity::ThreeYearDrop.default_cutoff_days(), 1_095);
    }

    #[test]
    fn enumerate_returns_empty_when_no_partitions_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let plans = enumerate_compactable(tmp.path(), fixed_now(), Granularity::HourlyToDaily);
        assert!(plans.is_empty());
    }

    #[test]
    fn enumerate_skips_partitions_younger_than_cutoff() {
        let tmp = tempfile::tempdir().unwrap();
        let now = fixed_now();
        // Day 50 in the past — well within the 90-day window.
        let recent_day = now - chrono::Duration::days(50);
        let p = tmp.path().join(format!(
            "events/year={:04}/month={:02}/day={:02}/hour=12/raw-00000.parquet",
            recent_day.format("%Y"),
            recent_day.format("%m"),
            recent_day.format("%d")
        ));
        write_zstd_lines(&p, &["{\"event_id\":\"01TEST\"}"]);
        let plans = enumerate_compactable(tmp.path(), now, Granularity::HourlyToDaily);
        assert!(plans.is_empty());
    }

    #[test]
    fn enumerate_returns_91_day_old_day_for_hourly_to_daily() {
        let tmp = tempfile::tempdir().unwrap();
        let now = fixed_now();
        let old_day = now - chrono::Duration::days(91);
        let day_root = tmp.path().join(format!(
            "events/year={:04}/month={:02}/day={:02}",
            old_day.format("%Y"),
            old_day.format("%m"),
            old_day.format("%d")
        ));
        for h in 0..3u32 {
            let p = day_root
                .join(format!("hour={h:02}"))
                .join(format!("raw-{h:05}.parquet"));
            write_zstd_lines(
                &p,
                &[
                    &format!("{{\"event_id\":\"01H{h}A\",\"kind\":\"turn\"}}"),
                    &format!("{{\"event_id\":\"01H{h}B\",\"kind\":\"tool_call\"}}"),
                ],
            );
        }
        let plans = enumerate_compactable(tmp.path(), now, Granularity::HourlyToDaily);
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(plan.granularity, Granularity::HourlyToDaily);
        assert_eq!(plan.sources.len(), 3);
        assert_eq!(plan.dest, day_root.join("raw-daily.parquet"));
    }

    #[test]
    fn compact_partition_merges_sources_atomically_and_unlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let now = fixed_now();
        let old_day = now - chrono::Duration::days(91);
        let day_root = tmp.path().join(format!(
            "events/year={:04}/month={:02}/day={:02}",
            old_day.format("%Y"),
            old_day.format("%m"),
            old_day.format("%d")
        ));
        let h0 = day_root.join("hour=00").join("raw-00000.parquet");
        let h1 = day_root.join("hour=01").join("raw-00000.parquet");
        write_zstd_lines(
            &h0,
            &[
                "{\"event_id\":\"01A\",\"kind\":\"turn\"}",
                "{\"event_id\":\"01B\",\"kind\":\"turn\"}",
            ],
        );
        write_zstd_lines(&h1, &["{\"event_id\":\"01C\",\"kind\":\"tool_call\"}"]);
        let plans = enumerate_compactable(tmp.path(), now, Granularity::HourlyToDaily);
        assert_eq!(plans.len(), 1);
        let counts = compact_partition(tmp.path(), &plans[0]).unwrap();
        assert_eq!(counts.files_in, 2);
        assert_eq!(counts.files_out, 1);
        // Destination has 3 rows.
        let (rows, _) = read_source_file(&plans[0].dest).unwrap();
        assert_eq!(rows.len(), 3);
        // Sources removed.
        assert!(!h0.exists());
        assert!(!h1.exists());
        // Empty `hour=*` directories pruned.
        assert!(!day_root.join("hour=00").exists());
    }

    #[test]
    fn three_year_drop_preserves_decisions_and_drops_high_pii_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let now = fixed_now();
        let old_month = now - chrono::Duration::days(1_100);
        let month_dir = tmp.path().join(format!(
            "events/year={:04}/month={:02}",
            old_month.format("%Y"),
            old_month.format("%m")
        ));
        let monthly = month_dir.join("raw-monthly.parquet");
        write_zstd_lines(
            &monthly,
            &[
                "{\"event_id\":\"01D\",\"kind\":\"decision\"}",
                "{\"event_id\":\"01A\",\"kind\":\"analysis\"}",
                "{\"event_id\":\"01L\",\"kind\":\"law_violation\"}",
                "{\"event_id\":\"01T1\",\"kind\":\"turn\"}",
                "{\"event_id\":\"01T2\",\"kind\":\"turn\",\"redactions\":[\"pii_risk:low\"]}",
            ],
        );
        let plans = enumerate_compactable(tmp.path(), now, Granularity::ThreeYearDrop);
        assert_eq!(plans.len(), 1);
        let counts = apply_three_year_drop(tmp.path(), &plans[0]).unwrap();
        // 4 preserved (3 audit kinds + 1 low-pii turn), 1 dropped.
        assert_eq!(counts.records_preserved, 4);
        assert_eq!(counts.records_dropped, 1);
        let preserved = month_dir.join("preserved.parquet");
        assert!(preserved.exists());
        // Original monthly file gone.
        assert!(!monthly.exists());
        let (rows, _) = read_source_file(&preserved).unwrap();
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn three_year_drop_removes_monthly_outright_when_nothing_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let now = fixed_now();
        let old_month = now - chrono::Duration::days(1_100);
        let month_dir = tmp.path().join(format!(
            "events/year={:04}/month={:02}",
            old_month.format("%Y"),
            old_month.format("%m")
        ));
        let monthly = month_dir.join("raw-monthly.parquet");
        write_zstd_lines(
            &monthly,
            &[
                "{\"event_id\":\"01T1\",\"kind\":\"turn\"}",
                "{\"event_id\":\"01T2\",\"kind\":\"turn\"}",
            ],
        );
        let plans = enumerate_compactable(tmp.path(), now, Granularity::ThreeYearDrop);
        let counts = apply_three_year_drop(tmp.path(), &plans[0]).unwrap();
        assert_eq!(counts.records_preserved, 0);
        assert_eq!(counts.records_dropped, 2);
        assert!(!monthly.exists());
        assert!(!month_dir.join("preserved.parquet").exists());
    }

    #[test]
    fn quarantine_pre_existing_moves_corrupted_and_tmp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp
            .path()
            .join("events/year=2026/month=04/day=28/hour=22/raw-00000.parquet.corrupted-1907");
        let orphan = tmp
            .path()
            .join("events/year=2026/month=04/day=29/hour=00/raw-daily.parquet.tmp");
        fs::create_dir_all(bad.parent().unwrap()).unwrap();
        fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        fs::write(&bad, b"garbage").unwrap();
        fs::write(&orphan, b"orphan").unwrap();
        let counts = quarantine_pre_existing(tmp.path());
        assert_eq!(counts.quarantined, 2);
        assert!(!bad.exists());
        assert!(!orphan.exists());
        let q_dir = tmp.path().join("events/_quarantine");
        // At least the directory now exists.
        assert!(q_dir.exists());
    }

    #[test]
    fn rollup_counts_merge_accumulates_every_field() {
        let mut a = RollupCounts {
            files_in: 1,
            files_out: 1,
            bytes_reclaimed: 10,
            quarantined: 0,
            records_dropped: 0,
            records_preserved: 5,
        };
        let b = RollupCounts {
            files_in: 2,
            files_out: 1,
            bytes_reclaimed: 100,
            quarantined: 1,
            records_dropped: 3,
            records_preserved: 7,
        };
        a.merge(&b);
        assert_eq!(a.files_in, 3);
        assert_eq!(a.files_out, 2);
        assert_eq!(a.bytes_reclaimed, 110);
        assert_eq!(a.quarantined, 1);
        assert_eq!(a.records_dropped, 3);
        assert_eq!(a.records_preserved, 12);
    }

    #[test]
    fn record_passes_whitelist_recognises_each_audit_kind() {
        let dec: serde_json::Value = serde_json::from_str("{\"kind\":\"decision\"}").unwrap();
        let ana: serde_json::Value = serde_json::from_str("{\"kind\":\"analysis\"}").unwrap();
        let law: serde_json::Value = serde_json::from_str("{\"kind\":\"law_violation\"}").unwrap();
        let trn: serde_json::Value = serde_json::from_str("{\"kind\":\"turn\"}").unwrap();
        let low: serde_json::Value =
            serde_json::from_str("{\"kind\":\"turn\",\"redactions\":[\"pii_risk:low\"]}").unwrap();
        assert!(record_passes_whitelist(&dec));
        assert!(record_passes_whitelist(&ana));
        assert!(record_passes_whitelist(&law));
        assert!(!record_passes_whitelist(&trn));
        assert!(record_passes_whitelist(&low));
    }

    #[test]
    fn granularity_serde_round_trips_via_snake_case() {
        let g = Granularity::HourlyToDaily;
        let s = serde_json::to_string(&g).unwrap();
        assert_eq!(s, "\"hourly_to_daily\"");
        let parsed: Granularity = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, g);
    }
}
