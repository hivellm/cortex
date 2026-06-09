//! `/v1/dashboard/canary` — surfaces the last 24 h of pre-thinking
//! health canary ticks (phase15f §3.4).
//!
//! Returns the most-recent `limit` rows from `canary_runs` plus a
//! convenience `last_failure` field so the GUI can render both a
//! sparkline and a "most recent failure" banner without client-side
//! filtering.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cortex_storage::metadata::CanaryRun;
use serde::Serialize;

use super::DashboardState;

/// Response body for `GET /v1/dashboard/canary`.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CanaryReport {
    /// Up to `LIMIT` most-recent canary run rows (newest first).
    pub runs: Vec<CanaryRunView>,
    /// The most-recent error tick within `runs`, or `null` when every
    /// tick in the window was `"ok"`.
    pub last_failure: Option<CanaryRunView>,
}

/// One canary tick row projected for the API surface.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CanaryRunView {
    /// RFC-3339 timestamp of the tick.
    pub ts: String,
    /// `"ok"` or `"error"`.
    pub status: String,
    /// Round-trip latency in milliseconds. `null` on hard errors.
    pub latency_ms: Option<i64>,
    /// Error detail. `null` on success.
    pub error_message: Option<String>,
}

impl From<CanaryRun> for CanaryRunView {
    fn from(r: CanaryRun) -> Self {
        Self {
            ts: r.ts,
            status: r.status,
            latency_ms: r.latency_ms,
            error_message: r.error_message,
        }
    }
}

const LIMIT: usize = 144; // 24 h at 10-min granularity; plenty for sparkline

pub(super) async fn canary(State(state): State<DashboardState>) -> Response {
    let Some(metadata) = state.metadata.as_ref() else {
        return Json(CanaryReport {
            runs: vec![],
            last_failure: None,
        })
        .into_response();
    };
    let rows_res = {
        let guard = match metadata.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.list_canary_runs(LIMIT)
    };
    let runs: Vec<CanaryRunView> = match rows_res {
        Ok(rows) => rows.into_iter().map(CanaryRunView::from).collect(),
        Err(_) => {
            return Json(CanaryReport {
                runs: vec![],
                last_failure: None,
            })
            .into_response();
        }
    };
    let last_failure = runs.iter().find(|r| r.status != "ok").cloned();
    Json(CanaryReport { runs, last_failure }).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_run_view_from_storage_row_maps_all_fields() {
        let row = CanaryRun {
            ts: "2026-06-09T12:00:00Z".to_string(),
            status: "ok".to_string(),
            latency_ms: Some(42),
            error_message: None,
        };
        let v = CanaryRunView::from(row);
        assert_eq!(v.ts, "2026-06-09T12:00:00Z");
        assert_eq!(v.status, "ok");
        assert_eq!(v.latency_ms, Some(42));
        assert!(v.error_message.is_none());
    }

    #[test]
    fn canary_run_view_error_row_maps_error_message() {
        let row = CanaryRun {
            ts: "2026-06-09T12:01:00Z".to_string(),
            status: "error".to_string(),
            latency_ms: None,
            error_message: Some("http 500".to_string()),
        };
        let v = CanaryRunView::from(row);
        assert_eq!(v.status, "error");
        assert!(v.latency_ms.is_none());
        assert_eq!(v.error_message.as_deref(), Some("http 500"));
    }
}
