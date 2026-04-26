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

