//! Identity-coverage report — per-backend `event_identity` gap
//! counters shared between `cortex-ops doctor-identity-coverage`
//! and the dashboard `/v1/dashboard/coverage` endpoint.
//!
//! Reference: ADR-012 §4.1 (the scan), ADR-014 / phase13f §2.3
//! (the dashboard view).
//!
//! The CLI binary in `cortex-cli/src/bin/cortex-ops/identity_coverage.rs`
//! runs the SQL scan and constructs a [`CoverageReport`]; the
//! dashboard handler then calls [`CoverageReport::view`] to project
//! a stable [`CoverageReportView`]. The trait
//! [`CoverageReportSource`] decouples the dashboard from the CLI
//! binary so the projection logic is testable without a SQLite
//! database.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Per-backend coverage counter pair: total rows where the column
/// was NULL plus a sample of orphan event_ids for the operator to
/// spot-check. Sample length is bounded by the caller's
/// `--sample-limit`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCoverage {
    /// Count of `event_identity` rows where the backend column was
    /// NULL.
    pub missing: u64,
    /// Bounded sample of `event_id` values that are missing the
    /// backend column.
    #[serde(default)]
    pub sample_event_ids: Vec<String>,
}

/// Full coverage report — one [`BackendCoverage`] entry per
/// backend column the scan covers (nexus / vectorizer / meili /
/// archive). The CLI binary serialises this verbatim; the
/// dashboard handler projects it via [`CoverageReport::view`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Resolved metadata DB path the scan read from.
    pub metadata_db: String,
    /// Total rows in `event_identity`.
    pub rows_total: u64,
    /// `nexus_id` column coverage.
    pub nexus: BackendCoverage,
    /// `vec_id` column coverage.
    pub vectorizer: BackendCoverage,
    /// `meili_id` column coverage.
    pub meili: BackendCoverage,
    /// `archive_partition` column coverage.
    pub archive: BackendCoverage,
    /// `true` when ANY backend column has at least one NULL row.
    /// The CLI exit code mirrors this — operator-visible failure
    /// at the cron level.
    pub failed: bool,
    /// Wall-clock latency of the scan in milliseconds.
    pub latency_ms: u64,
}

impl CoverageReport {
    /// Project the report into the dashboard view. Pure (same input
    /// ⇒ same output). The dashboard handler MUST call this rather
    /// than recompute state on the handler side (ADR-014 / phase13f
    /// §3.3).
    ///
    /// The view orders backends as `[nexus, vectorizer, meili,
    /// archive]` — the order the CLI scan walks columns and the
    /// order GUI tables render. `is_healthy` is the inverse of
    /// `failed`; carried on the view so handlers do not re-derive.
    pub fn view(&self) -> CoverageReportView {
        CoverageReportView {
            metadata_db: self.metadata_db.clone(),
            rows_total: self.rows_total,
            backends: vec![
                BackendCoverageEntry::from("nexus", &self.nexus),
                BackendCoverageEntry::from("vectorizer", &self.vectorizer),
                BackendCoverageEntry::from("meili", &self.meili),
                BackendCoverageEntry::from("archive", &self.archive),
            ],
            is_healthy: !self.failed,
            latency_ms: self.latency_ms,
        }
    }
}

/// Dashboard projection of [`CoverageReport`]. The handler renders
/// these without doing any additional state inference — see
/// ADR-014 / phase13f §2.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReportView {
    /// Resolved metadata DB path the scan read from.
    pub metadata_db: String,
    /// Total rows in `event_identity`.
    pub rows_total: u64,
    /// Per-backend gap entries in fixed order: nexus, vectorizer,
    /// meili, archive.
    pub backends: Vec<BackendCoverageEntry>,
    /// `true` when every backend has zero missing rows. Inverse of
    /// the domain-level `failed` flag; carried so handlers and GUI
    /// never recompute.
    pub is_healthy: bool,
    /// Wall-clock latency of the scan in milliseconds.
    pub latency_ms: u64,
}

/// One row in [`CoverageReportView::backends`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCoverageEntry {
    /// Backend identifier (`nexus` / `vectorizer` / `meili` /
    /// `archive`).
    pub backend: String,
    /// Count of rows where the backend column was NULL.
    pub missing: u64,
    /// Bounded sample of `event_id` values missing the backend
    /// column.
    pub sample_event_ids: Vec<String>,
}

impl BackendCoverageEntry {
    /// Build an entry from a label + the domain pair.
    pub fn from(backend: &str, c: &BackendCoverage) -> Self {
        Self {
            backend: backend.to_string(),
            missing: c.missing,
            sample_event_ids: c.sample_event_ids.clone(),
        }
    }
}

/// Source the dashboard reads from. The CLI binary's scanner
/// implements this; tests substitute a fixture so the projection
/// logic is exercised without a SQLite database.
pub trait CoverageReportSource: Send + Sync {
    /// Return the latest report. Implementations may run the scan
    /// on demand or read a cached row — the dashboard handler does
    /// not care.
    fn collect(&self) -> anyhow::Result<CoverageReport>;
}

/// Single-pass scanner that walks `event_identity` and counts per-
/// backend NULL columns + an orphan sample bounded by
/// `sample_limit`. Shared by `cortex-ops doctor-identity-coverage`
/// and the `/v1/dashboard/coverage` endpoint (ADR-014 / phase13f
/// §3.3).
///
/// `latency_ms` on the returned [`CoverageReport`] is left at zero —
/// callers that need the wall-clock latency stamp it themselves
/// (the CLI prints it, the dashboard reports it on the view).
pub fn collect_coverage(
    conn: &Connection,
    db_path: &Path,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(failed: bool) -> CoverageReport {
        CoverageReport {
            metadata_db: "/var/lib/cortex/metadata.sqlite".into(),
            rows_total: 1024,
            nexus: BackendCoverage {
                missing: if failed { 3 } else { 0 },
                sample_event_ids: if failed {
                    vec!["01A".into(), "01B".into()]
                } else {
                    Vec::new()
                },
            },
            vectorizer: BackendCoverage::default(),
            meili: BackendCoverage::default(),
            archive: BackendCoverage::default(),
            failed,
            latency_ms: 42,
        }
    }

    #[test]
    fn report_round_trips_via_serde() {
        let r = sample_report(true);
        let j = serde_json::to_string(&r).unwrap();
        let p: CoverageReport = serde_json::from_str(&j).unwrap();
        assert_eq!(p, r);
    }

    #[test]
    fn view_projects_backends_in_fixed_order() {
        let r = sample_report(true);
        let v = r.view();
        assert_eq!(v.backends.len(), 4);
        assert_eq!(v.backends[0].backend, "nexus");
        assert_eq!(v.backends[1].backend, "vectorizer");
        assert_eq!(v.backends[2].backend, "meili");
        assert_eq!(v.backends[3].backend, "archive");
        assert_eq!(v.backends[0].missing, 3);
        assert_eq!(v.backends[0].sample_event_ids, vec!["01A", "01B"]);
    }

    #[test]
    fn view_is_healthy_is_inverse_of_failed() {
        assert!(sample_report(false).view().is_healthy);
        assert!(!sample_report(true).view().is_healthy);
    }

    #[test]
    fn view_is_a_pure_projection() {
        let r = sample_report(true);
        assert_eq!(r.view(), r.view());
    }

    #[test]
    fn view_round_trips_via_serde() {
        let v = sample_report(true).view();
        let j = serde_json::to_string(&v).unwrap();
        let p: CoverageReportView = serde_json::from_str(&j).unwrap();
        assert_eq!(p, v);
    }

    struct StubSource(CoverageReport);

    impl CoverageReportSource for StubSource {
        fn collect(&self) -> anyhow::Result<CoverageReport> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn source_trait_is_object_safe_and_returns_report() {
        let src: Box<dyn CoverageReportSource> = Box::new(StubSource(sample_report(false)));
        let report = src.collect().unwrap();
        assert!(!report.failed);
        assert_eq!(report.rows_total, 1024);
    }
}
