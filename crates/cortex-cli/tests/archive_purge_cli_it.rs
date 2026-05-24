//! Phase12b §2.3 — `cortex-ops retention-archive-purge` CLI IT.
//!
//! Drives the binary end-to-end against a synthetic archive: writes
//! one zstd-encoded NDJSON parquet file in the partition layout the
//! storage walker expects, invokes the binary with `--before`, and
//! asserts the JSON report on stdout matches the deletion shape.
//!
//! Companion to the unit tests in
//! `cortex-storage::archive_purge::tests` — those cover the walker
//! semantics. This IT covers the CLI argument plumbing + exit-code
//! contract.
//!
//! ## Why `#[cfg(not(debug_assertions))]`
//!
//! The `cortex-ops` debug binary on Windows overflows the default
//! main-thread stack on startup because `Command` carries dozens of
//! large variants and the debug build does not stack-pack them. The
//! release binary has no such issue. Gating the IT on
//! `not(debug_assertions)` runs it under `cargo test --release` (CI
//! default) and skips it under plain `cargo test` (developer
//! iteration). The storage-layer unit tests in
//! `cortex-storage::archive_purge::tests` still cover the walker
//! exhaustively in either profile.

#![cfg(not(debug_assertions))]

use std::io::Write;
use std::path::Path;
use std::process::Command;

use cortex_core::events::{Context, Envelope, Kind, Stream, Turn};

const BIN: &str = env!("CARGO_BIN_EXE_cortex-ops");

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
            extras: Default::default(),
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

fn write_archive_file(home: &Path, rel_dir: &str, name: &str, envelopes: &[Envelope]) {
    let dir = home.join(rel_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
    for env in envelopes {
        let line = serde_json::to_string(env).unwrap();
        enc.write_all(line.as_bytes()).unwrap();
        enc.write_all(b"\n").unwrap();
    }
    enc.finish().unwrap();
}

#[test]
fn dry_run_against_old_archive_reports_one_deletion_no_filesystem_change() {
    let dir = tempfile::tempdir().unwrap();
    write_archive_file(
        dir.path(),
        "events/year=2025/month=12/day=01/hour=00",
        "raw-00000.parquet",
        &[envelope("E_OLD", "cortex", "2025-12-01T00:00:00Z")],
    );

    let output = Command::new(BIN)
        // Windows debug builds of `cortex-ops` overflow the default
        // main-thread stack because the `Command` enum carries huge
        // variants. Bumping `RUST_MIN_STACK` works around it; the
        // release binary does not need this knob.
        .env("RUST_MIN_STACK", "16777216")
        .args([
            "retention-archive-purge",
            "--before",
            "2026-04-01T00:00:00+00:00",
            "--dry-run",
            "--home",
        ])
        .arg(dir.path())
        .output()
        .expect("spawn cortex-ops");

    assert!(
        output.status.success(),
        "expected exit 0 on dry-run, got {:?}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("parse stdout: {e}\nstdout={stdout}"));
    assert_eq!(report["dry_run"], serde_json::Value::Bool(true));
    assert_eq!(report["files_deleted"], 1);
    assert_eq!(report["files_kept"], 0);
    // Filesystem actually unchanged.
    let target = dir
        .path()
        .join("events/year=2025/month=12/day=01/hour=00/raw-00000.parquet");
    assert!(target.exists(), "dry-run must not delete the file");
}

#[test]
fn live_run_deletes_old_files_and_keeps_recent_ones() {
    let dir = tempfile::tempdir().unwrap();
    write_archive_file(
        dir.path(),
        "events/year=2025/month=12/day=01/hour=00",
        "raw-00000.parquet",
        &[envelope("E_OLD", "cortex", "2025-12-01T00:00:00Z")],
    );
    write_archive_file(
        dir.path(),
        "events/year=2026/month=05/day=01/hour=00",
        "raw-00000.parquet",
        &[envelope("E_NEW", "cortex", "2026-05-01T00:00:00Z")],
    );
    let output = Command::new(BIN)
        // Windows debug builds of `cortex-ops` overflow the default
        // main-thread stack because the `Command` enum carries huge
        // variants. Bumping `RUST_MIN_STACK` works around it; the
        // release binary does not need this knob.
        .env("RUST_MIN_STACK", "16777216")
        .args([
            "retention-archive-purge",
            "--before",
            "2026-04-01T00:00:00+00:00",
            "--home",
        ])
        .arg(dir.path())
        .output()
        .expect("spawn cortex-ops");
    assert!(
        output.status.success(),
        "expected exit 0, got {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["files_deleted"], 1);
    assert_eq!(report["files_kept"], 1);

    let old = dir
        .path()
        .join("events/year=2025/month=12/day=01/hour=00/raw-00000.parquet");
    let new = dir
        .path()
        .join("events/year=2026/month=05/day=01/hour=00/raw-00000.parquet");
    assert!(!old.exists(), "old file must be deleted");
    assert!(new.exists(), "new file must survive");
}

#[test]
fn invalid_before_flag_exits_with_code_two() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(BIN)
        // Windows debug builds of `cortex-ops` overflow the default
        // main-thread stack because the `Command` enum carries huge
        // variants. Bumping `RUST_MIN_STACK` works around it; the
        // release binary does not need this knob.
        .env("RUST_MIN_STACK", "16777216")
        .args([
            "retention-archive-purge",
            "--before",
            "not-a-date",
            "--home",
        ])
        .arg(dir.path())
        .output()
        .expect("spawn cortex-ops");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--before"),
        "expected stderr to name the flag, got: {stderr}"
    );
}

#[test]
fn empty_archive_returns_zeroed_report_with_exit_zero() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(BIN)
        // Windows debug builds of `cortex-ops` overflow the default
        // main-thread stack because the `Command` enum carries huge
        // variants. Bumping `RUST_MIN_STACK` works around it; the
        // release binary does not need this knob.
        .env("RUST_MIN_STACK", "16777216")
        .args([
            "retention-archive-purge",
            "--before",
            "2026-04-01T00:00:00+00:00",
            "--home",
        ])
        .arg(dir.path())
        .output()
        .expect("spawn cortex-ops");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["files_deleted"], 0);
    assert_eq!(report["partitions_visited"], 0);
}

#[test]
fn repo_filter_pins_files_with_other_repos() {
    let dir = tempfile::tempdir().unwrap();
    write_archive_file(
        dir.path(),
        "events/year=2025/month=12/day=01/hour=00",
        "raw-00000.parquet",
        &[
            envelope("E_CORTEX", "cortex", "2025-12-01T00:00:00Z"),
            envelope("E_NEXUS", "nexus", "2025-12-01T00:00:00Z"),
        ],
    );
    let output = Command::new(BIN)
        // Windows debug builds of `cortex-ops` overflow the default
        // main-thread stack because the `Command` enum carries huge
        // variants. Bumping `RUST_MIN_STACK` works around it; the
        // release binary does not need this knob.
        .env("RUST_MIN_STACK", "16777216")
        .args([
            "retention-archive-purge",
            "--before",
            "2026-04-01T00:00:00+00:00",
            "--repo",
            "cortex",
            "--home",
        ])
        .arg(dir.path())
        .output()
        .expect("spawn cortex-ops");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["files_deleted"], 0);
    assert_eq!(report["files_kept"], 1);
    assert_eq!(report["repo_filter"], "cortex");
}
