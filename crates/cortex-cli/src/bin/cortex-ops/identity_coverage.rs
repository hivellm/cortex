//! ADR-012 §4.1 — `cortex-ops doctor-identity-coverage` — walks
//! the `event_identity` table once and reports per-backend
//! coverage gaps. Replaces the per-call cross-backend fan-out
//! `doctor_consistency` absorbs today with a single indexed scan.
//!
//! Per-row check:
//!
//! - Count rows whose `nexus_id` is NULL → `nexus_missing`.
//! - Count rows whose `vec_id` is NULL → `vec_missing`.
//! - Count rows whose `meili_id` is NULL → `meili_missing`.
//! - Count rows whose `archive_partition` is NULL → `archive_missing`.
//!
//! A row stamped by ALL 4 projections (the happy path) does not
//! contribute to any counter. Any non-zero counter is a coverage
//! gap the operator should resolve — either a projection bug or
//! a worker that dropped the event mid-batch.
//!
//! Exit code `2` when at least one backend has any missing column
//! so cron wrappers escalate visibly; otherwise `0`.
//!
//! Budget: indexed scan of 100k rows finishes well under 10 s on
//! the running stack (`event_id` is the PK, the secondary
//! UNIQUE partial indexes back the reverse lookups but the scan
//! itself is a full table read of compact rows). The 100k bench
//! gate lands in §4.2.

use std::path::PathBuf;
use std::process::ExitCode;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use super::helpers::resolve_metadata_db_path;

/// One per-backend coverage counter pair: total rows where the
/// column was NULL plus a sample of orphan event_ids for the
/// operator to spot-check. The sample is capped by
/// `--sample-limit` so a million-row gap does not blow up the
/// report.
#[derive(Debug, Clone, Default, Serialize)]
struct BackendCoverage {
    missing: u64,
    sample_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct CoverageReport {
    metadata_db: String,
    rows_total: u64,
    nexus: BackendCoverage,
    vectorizer: BackendCoverage,
    meili: BackendCoverage,
    archive: BackendCoverage,
    /// True when ANY backend column has at least one NULL row.
    /// The CLI exit code mirrors this — operator-visible failure
    /// at the cron level.
    failed: bool,
    /// Wall-clock latency of the scan in milliseconds.
    latency_ms: u64,
}

pub(super) fn doctor_identity_coverage(
    metadata_db: Option<String>,
    sample_limit: usize,
    json_output: bool,
) -> ExitCode {
    let db_path = metadata_db
        .map(PathBuf::from)
        .unwrap_or_else(resolve_metadata_db_path);

    if !db_path.exists() {
        eprintln!(
            "doctor-identity-coverage: metadata DB {} not found",
            db_path.display()
        );
        return ExitCode::FAILURE;
    }

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "doctor-identity-coverage: open metadata DB {}: {e}",
                db_path.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let started = std::time::Instant::now();
    let report = match collect_coverage(&conn, &db_path, sample_limit) {
        Ok(mut r) => {
            r.latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            r
        }
        Err(e) => {
            eprintln!("doctor-identity-coverage: scan failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json_output {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-identity-coverage: serialise: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render_text(&report);
    }

    if report.failed {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect_coverage(
    conn: &Connection,
    db_path: &std::path::Path,
    sample_limit: usize,
) -> rusqlite::Result<CoverageReport> {
    let mut report = CoverageReport {
        metadata_db: db_path.display().to_string(),
        ..CoverageReport::default()
    };

    report.rows_total = conn
        .query_row("SELECT COUNT(*) FROM event_identity", [], |r| {
            r.get::<_, i64>(0)
        })?
        .max(0) as u64;

    for (label, column) in [
        ("nexus", "nexus_id"),
        ("vectorizer", "vec_id"),
        ("meili", "meili_id"),
        ("archive", "archive_partition"),
    ] {
        let missing_count = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM event_identity WHERE {column} IS NULL"),
                [],
                |r| r.get::<_, i64>(0),
            )?
            .max(0) as u64;
        let mut sample: Vec<String> = Vec::new();
        if missing_count > 0 && sample_limit > 0 {
            let mut stmt = conn.prepare(&format!(
                "SELECT event_id FROM event_identity WHERE {column} IS NULL \
                 ORDER BY event_id ASC LIMIT ?1"
            ))?;
            let rows = stmt.query_map([sample_limit as i64], |r| r.get::<_, String>(0))?;
            for row in rows.flatten() {
                sample.push(row);
            }
        }
        let coverage = BackendCoverage {
            missing: missing_count,
            sample_event_ids: sample,
        };
        match label {
            "nexus" => report.nexus = coverage,
            "vectorizer" => report.vectorizer = coverage,
            "meili" => report.meili = coverage,
            "archive" => report.archive = coverage,
            _ => unreachable!(),
        }
    }

    report.failed = report.nexus.missing > 0
        || report.vectorizer.missing > 0
        || report.meili.missing > 0
        || report.archive.missing > 0;
    Ok(report)
}

fn render_text(r: &CoverageReport) {
    println!("cortex-ops doctor-identity-coverage");
    println!("metadata_db:  {}", r.metadata_db);
    println!("rows_total:   {}", r.rows_total);
    println!("latency_ms:   {}", r.latency_ms);
    println!();
    println!("per-backend coverage gaps:");
    println!("  nexus_missing:      {}", r.nexus.missing);
    println!("  vectorizer_missing: {}", r.vectorizer.missing);
    println!("  meili_missing:      {}", r.meili.missing);
    println!("  archive_missing:    {}", r.archive.missing);
    if r.failed {
        println!();
        println!("FAILED — at least one backend has missing identity rows.");
        for (label, cov) in [
            ("nexus", &r.nexus),
            ("vectorizer", &r.vectorizer),
            ("meili", &r.meili),
            ("archive", &r.archive),
        ] {
            if cov.missing == 0 || cov.sample_event_ids.is_empty() {
                continue;
            }
            println!();
            println!("  {label} missing — sample event_ids:");
            for id in &cov.sample_event_ids {
                println!("    {id}");
            }
        }
    } else {
        println!();
        println!("OK — every row carries all four backend ids.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_storage::{apply_phase13d_schema, Backend, IdentityIndex as _, SqliteIdentityIndex};

    fn open() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_phase13d_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn empty_table_reports_zero_rows_and_no_failure() {
        let conn = open();
        let report = collect_coverage(&conn, std::path::Path::new(":memory:"), 50).unwrap();
        assert_eq!(report.rows_total, 0);
        assert_eq!(report.nexus.missing, 0);
        assert_eq!(report.vectorizer.missing, 0);
        assert_eq!(report.meili.missing, 0);
        assert_eq!(report.archive.missing, 0);
        assert!(!report.failed);
    }

    #[test]
    fn fully_stamped_row_reports_zero_gaps() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("EVT", Backend::Nexus, "node-1")
            .unwrap();
        idx.upsert_identity("EVT", Backend::Vectorizer, "vec-1")
            .unwrap();
        idx.upsert_identity("EVT", Backend::Meili, "doc-1").unwrap();
        idx.upsert_identity(
            "EVT",
            Backend::Archive,
            "events/year=2026/month=05/raw-00000.parquet",
        )
        .unwrap();

        let report = collect_coverage(&conn, std::path::Path::new(":memory:"), 50).unwrap();
        assert_eq!(report.rows_total, 1);
        assert_eq!(report.nexus.missing, 0);
        assert_eq!(report.vectorizer.missing, 0);
        assert_eq!(report.meili.missing, 0);
        assert_eq!(report.archive.missing, 0);
        assert!(!report.failed);
    }

    #[test]
    fn partial_stamp_surfaces_per_backend_gap_with_sample() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        // EVT_A: only archive stamped → 3 columns NULL.
        idx.upsert_identity(
            "EVT_A",
            Backend::Archive,
            "events/year=2026/month=05/raw-00000.parquet",
        )
        .unwrap();
        // EVT_B: archive + meili stamped → 2 columns NULL.
        idx.upsert_identity(
            "EVT_B",
            Backend::Archive,
            "events/year=2026/month=05/raw-00000.parquet",
        )
        .unwrap();
        idx.upsert_identity("EVT_B", Backend::Meili, "EVT_B")
            .unwrap();

        let report = collect_coverage(&conn, std::path::Path::new(":memory:"), 50).unwrap();
        assert_eq!(report.rows_total, 2);
        // Both rows lack nexus_id + vec_id.
        assert_eq!(report.nexus.missing, 2);
        assert_eq!(report.vectorizer.missing, 2);
        // EVT_A also lacks meili_id; EVT_B has it.
        assert_eq!(report.meili.missing, 1);
        // Both rows have archive_partition.
        assert_eq!(report.archive.missing, 0);
        // Failure surfaces.
        assert!(report.failed);
        // Sample lists are sorted (EVT_A before EVT_B).
        assert_eq!(report.nexus.sample_event_ids, vec!["EVT_A", "EVT_B"]);
        assert_eq!(report.vectorizer.sample_event_ids, vec!["EVT_A", "EVT_B"]);
        assert_eq!(report.meili.sample_event_ids, vec!["EVT_A"]);
        assert!(report.archive.sample_event_ids.is_empty());
    }

    // ADR-012 §4.2 — budget gate. The legacy doctor_consistency
    // fanned out per-backend HTTP probes; over 100k events that
    // took minutes. The identity-driven scan reads a single SQLite
    // table with an indexed COUNT + 4 sampled SELECTs, each
    // bounded by the partial UNIQUE indexes the migration
    // installs. The 10-second budget below is the spec-04
    // ADR-012 §4.2 contract: a regression that bumps scan latency
    // above 10 s on the test machine MUST fail in CI before it
    // ships. Run is gated behind `CORTEX_DOCTOR_BENCH=1` so a
    // routine `cargo test` does not spend the seeding cost on
    // every iteration (the seed alone takes ~1-2 s on Windows
    // even at this row count); CI flips the gate in the bench
    // workflow.
    #[test]
    fn scan_100k_rows_finishes_under_10s_budget() {
        if !cortex_config::Config::load()
            .map(|c| c.doctor.bench)
            .unwrap_or(false)
        {
            eprintln!("skipping: set CORTEX_DOCTOR_BENCH=true to run the 100k budget gate");
            return;
        }
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        // 100 000 rows — every projection stamped, no coverage gap.
        // Wrap the seed in a single transaction so SQLite does not
        // fsync per row; the bench measures the SCAN, not the
        // ingest. The doctor's production input is also produced
        // by per-event commits inside the workers' own batch
        // transactions, so the on-disk shape lines up.
        conn.execute_batch("BEGIN TRANSACTION").unwrap();
        for i in 0..100_000u32 {
            let event_id = format!("EVT_{i:06}");
            idx.upsert_identity(&event_id, Backend::Nexus, &format!("node-{i}"))
                .unwrap();
            idx.upsert_identity(&event_id, Backend::Vectorizer, &format!("vec-{i}"))
                .unwrap();
            idx.upsert_identity(&event_id, Backend::Meili, &event_id)
                .unwrap();
            idx.upsert_identity(
                &event_id,
                Backend::Archive,
                &format!("events/year=2026/month=05/raw-{:05}.parquet", i / 10_000),
            )
            .unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();

        let started = std::time::Instant::now();
        let report = collect_coverage(&conn, std::path::Path::new(":memory:"), 0).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(report.rows_total, 100_000);
        assert!(!report.failed, "every row should be fully stamped");
        assert!(
            elapsed.as_secs() < 10,
            "ADR-012 §4.2 budget gate: 100k-row scan must finish under 10s, got {elapsed:?}"
        );
    }

    #[test]
    fn sample_limit_caps_orphan_listing() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        // Insert 5 rows each missing nexus_id.
        for i in 0..5 {
            idx.upsert_identity(
                &format!("EVT_{i:02}"),
                Backend::Vectorizer,
                &format!("vec-{i:02}"),
            )
            .unwrap();
        }
        let report = collect_coverage(&conn, std::path::Path::new(":memory:"), 3).unwrap();
        assert_eq!(report.nexus.missing, 5);
        // Sample respects the cap.
        assert_eq!(report.nexus.sample_event_ids.len(), 3);
        assert_eq!(
            report.nexus.sample_event_ids,
            vec!["EVT_00", "EVT_01", "EVT_02"]
        );
    }
}
