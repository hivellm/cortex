//! phase11v §6 — `retention_sweeps` bookkeeping helper.
//!
//! Every `cortex-ops <sweep>` binary records one row per invocation
//! through this helper so the dashboard's
//! `Bytes reclaimed last 30 d` panel — and any operator querying
//! `retention_sweeps` directly — sees a faithful per-sweep history.
//!
//! Two legacy handlers (`retention-sweep` and `rollup`) already use
//! the two-step `start_retention_sweep` / `finish_retention_sweep`
//! API because they need the running-row advisory lock. Every other
//! sweep relies on the `cortex-workers::retention::scheduler`
//! per-job semaphore for serialisation and only needs the
//! single-row write — that's what `note_sweep_completion` is for.

use std::path::Path;

use chrono::{DateTime, Utc};
use cortex_storage::{MetadataError, MetadataStore};

/// Per-sweep counters serialised into `tier_transitions_json` on the
/// completed row. Keeping the schema flat keeps the dashboard parser
/// (`gui/src/views/Retention.tsx::sweepBytesReclaimed`) untouched —
/// it walks the per-stage object for `bytes_reclaimed` and falls
/// back to `0` when absent, so a sweep with no byte counter still
/// publishes an honest row without distorting the sparkline.
#[derive(Default, Clone, serde::Serialize)]
pub struct SweepStageStats {
    /// Bytes the sweep reclaimed (Meili doc shrink, archive copy
    /// drop, vector demotion, …). Zero when the sweep does not
    /// emit a byte counter.
    pub bytes_reclaimed: u64,
    /// Records demoted (typically vector tier transitions).
    pub records_demoted: u64,
    /// Records dropped (hard purge — older-than-N retention).
    pub records_dropped: u64,
    /// First line of stderr / `last_error` when the sweep failed.
    /// Truncated to 256 chars by the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Free-form per-sweep extras the dashboard surfaces in the
    /// detail view (`pruned`, `quarantined`, `cohort_counts`, …).
    /// Empty by default.
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub extras: serde_json::Map<String, serde_json::Value>,
}

/// Render the `tier_transitions_json` payload that the new helper
/// writes. Single key (`sweep_name`) at the top so the dashboard
/// per-card `stages[id]` lookup matches the existing key-based
/// projection convention.
pub fn render_stage_json(sweep_name: &str, stats: &SweepStageStats) -> String {
    let mut top = serde_json::Map::new();
    let payload = serde_json::to_value(stats).unwrap_or(serde_json::Value::Null);
    top.insert(sweep_name.to_string(), payload);
    serde_json::Value::Object(top).to_string()
}

/// Open the metadata store at `metadata_path` and write one
/// completed-state row. Best-effort: on store-open or write
/// failure the helper logs and returns the underlying error so the
/// caller can decide whether to fail the sweep or just emit a
/// stderr note. The recommendation is to log and CONTINUE — a
/// missed bookkeeping row is preferable to a sweep that ran
/// successfully against the live backend but is reported as a
/// failure because the audit write tripped.
pub fn note_sweep_completion(
    metadata_path: &Path,
    sweep_name: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    status: &str,
    stats: SweepStageStats,
) -> Result<(), MetadataError> {
    let store = MetadataStore::open(metadata_path)?;
    let sweep_id = cortex_workers::retention::new_sweep_id();
    let json = render_stage_json(sweep_name, &stats);
    store.note_completed_retention_sweep(
        &sweep_id,
        started_at,
        finished_at,
        stats.records_demoted,
        stats.records_dropped,
        &json,
        status,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_stage_json_emits_single_top_level_key_matching_sweep_name() {
        let stats = SweepStageStats {
            bytes_reclaimed: 4096,
            records_demoted: 2,
            records_dropped: 0,
            last_error: None,
            extras: Default::default(),
        };
        let json = render_stage_json("meili_prune", &stats);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("meili_prune"));
        assert_eq!(obj["meili_prune"]["bytes_reclaimed"], 4096);
        assert_eq!(obj["meili_prune"]["records_demoted"], 2);
    }

    #[test]
    fn note_sweep_completion_writes_one_row_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.sqlite");
        // First-time open creates the schema (apply_phase9a_schema is
        // called inside MetadataStore::open).
        let now = Utc::now();
        note_sweep_completion(
            &path,
            "meili_prune",
            now,
            now + chrono::Duration::seconds(1),
            "success",
            SweepStageStats {
                bytes_reclaimed: 1024,
                ..Default::default()
            },
        )
        .unwrap();
        note_sweep_completion(
            &path,
            "pii_enforce",
            now + chrono::Duration::seconds(2),
            now + chrono::Duration::seconds(3),
            "success",
            SweepStageStats::default(),
        )
        .unwrap();

        let store = MetadataStore::open(&path).unwrap();
        let rows = store.list_recent_sweeps(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.status == "success"));
    }
}
