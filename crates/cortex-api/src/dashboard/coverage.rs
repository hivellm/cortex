//! `/v1/dashboard/coverage` — surfaces per-backend identity
//! coverage gaps (ADR-012 §4.1 + ADR-014 / phase13f §3.3).
//!
//! Reads the `event_identity` table via
//! [`cortex_workers::coverage::collect_coverage`] (the same scanner
//! `cortex-ops doctor-identity-coverage` runs) and projects the
//! result through [`CoverageReport::view`] so the handler stays a
//! pure typed reader — no fallback strings, no handler-side
//! derivations.
//!
//! When the dashboard has no metadata store wired (cold dev boot),
//! the handler returns the all-zero `CoverageReportView` with an
//! empty `metadata_db` string — the GUI distinguishes "not wired"
//! from "scanned and clean" by checking that field.

use std::time::Instant;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cortex_workers::coverage::{collect_coverage, CoverageReport, CoverageReportView};

use super::DashboardState;

/// Hard cap on the orphan-event sample size carried back on each
/// response. Matches the CLI default — large enough for an
/// operator to spot-check, small enough that a million-row gap
/// does not blow up the JSON payload.
pub const DEFAULT_SAMPLE_LIMIT: usize = 50;

pub(super) async fn coverage(State(state): State<DashboardState>) -> Response {
    let Some(metadata) = state.metadata.as_ref() else {
        return Json(unscanned_view()).into_response();
    };
    let started = Instant::now();
    let report_res = {
        let guard = match metadata.lock() {
            Ok(g) => g,
            Err(_) => return Json(unscanned_view()).into_response(),
        };
        let conn = guard.conn();
        // `MetadataStore` does not expose its on-disk path; pass an
        // empty marker so the view's `metadata_db` field stays
        // honestly blank. The CLI (`cortex-ops
        // doctor-identity-coverage`) remains the authoritative
        // surface when the path matters.
        collect_coverage(conn, std::path::Path::new(""), DEFAULT_SAMPLE_LIMIT)
    };
    match report_res {
        Ok(mut report) => {
            report.latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            Json(report.view()).into_response()
        }
        Err(_) => Json(unscanned_view()).into_response(),
    }
}

/// View returned when no scan happened — empty metadata_db marks
/// the "not wired" state so the GUI does not confuse it with
/// "scanned and clean". All-zero counters are factual: the scanner
/// produced no observations.
fn unscanned_view() -> CoverageReportView {
    CoverageReport::default().view()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unscanned_view_carries_empty_metadata_db_marker() {
        let v = unscanned_view();
        assert_eq!(v.metadata_db, "");
        assert_eq!(v.rows_total, 0);
        assert_eq!(v.latency_ms, 0);
    }

    #[test]
    fn unscanned_view_renders_four_backend_entries_in_fixed_order() {
        let v = unscanned_view();
        assert_eq!(v.backends.len(), 4);
        let names: Vec<&str> = v.backends.iter().map(|b| b.backend.as_str()).collect();
        assert_eq!(names, vec!["nexus", "vectorizer", "meili", "archive"]);
        assert!(v.is_healthy);
    }

    #[test]
    fn unscanned_view_backends_carry_zero_counters() {
        let v = unscanned_view();
        for b in &v.backends {
            assert_eq!(b.missing, 0);
            assert!(b.sample_event_ids.is_empty());
        }
    }
}
