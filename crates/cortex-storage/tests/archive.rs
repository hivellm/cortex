//! Integration tests for `cortex_storage::archive`.

use chrono::{TimeZone, Utc};
use cortex_storage::{archive_filename, archive_partition};
use std::path::PathBuf;

#[test]
fn partition_has_expected_layout() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 17, 12, 34, 56).unwrap();
    let root = PathBuf::from("/tmp/cortex-data");
    let p = archive_partition(&root, ts);
    assert!(p.ends_with("events/year=2026/month=04/day=17/hour=12"));
}

#[test]
fn filename_zero_pads_sequence() {
    assert_eq!(archive_filename("raw", 0), "raw-00000.parquet");
    assert_eq!(archive_filename("bootstrap", 42), "bootstrap-00042.parquet");
}
