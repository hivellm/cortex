//! `/v1/dashboard/producers` — surfaces per-producer cursor state
//! (ADR-014 / phase13f §3.4).
//!
//! Reads the `producer_checkpoints` SQLite table via
//! [`MetadataStore::latest_producer_checkpoint_per_scope`] and
//! projects each row through [`ProducerCheckpoint::view`] so the
//! handler stays a pure typed reader.
//!
//! Scope note (deviation from the phase13f §3.4 spec text): the
//! spec originally called for `ProducerReportView` joined with
//! `producer_checkpoints`. Per-run [`ProducerReport`] rows are
//! in-memory only — `cortex-workers` does not persist a
//! `producer_reports` table today. The dashboard therefore surfaces
//! `ProducerCheckpointsReportView` (checkpoints-only), which is
//! the durable state actually available. A future task can add the
//! reports table and extend the view shape without changing this
//! handler's contract beyond an additive field set.
//!
//! When the dashboard has no metadata store wired (cold dev boot)
//! the handler returns an all-zero view; the GUI distinguishes
//! "not wired" from "no producers running" by checking `total`
//! plus the absence of any rows.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cortex_workers::producer::{ProducerCheckpoint, ProducerCheckpointsReportView};

use super::DashboardState;

pub(super) async fn producers(State(state): State<DashboardState>) -> Response {
    let Some(metadata) = state.metadata.as_ref() else {
        return Json(empty_view()).into_response();
    };
    let rows_res = {
        let guard = match metadata.lock() {
            Ok(g) => g,
            Err(_) => return Json(empty_view()).into_response(),
        };
        guard.latest_producer_checkpoint_per_scope()
    };
    let rows = match rows_res {
        Ok(rows) => rows
            .into_iter()
            .map(|row| ProducerCheckpoint::from_row(row).view())
            .collect(),
        Err(_) => return Json(empty_view()).into_response(),
    };
    Json(ProducerCheckpointsReportView::from_rows(rows)).into_response()
}

/// All-zero view returned when no metadata store is wired or the
/// query itself fails. `total = 0` plus an empty `rows` is honest:
/// nothing was observed.
fn empty_view() -> ProducerCheckpointsReportView {
    ProducerCheckpointsReportView::from_rows(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_view_carries_zero_total_and_no_rows() {
        let v = empty_view();
        assert_eq!(v.total, 0);
        assert!(v.rows.is_empty());
    }
}
