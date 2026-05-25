use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use super::DashboardState;

/// Query params for `/v1/retention/sweeps`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct RetentionSweepsQuery {
    /// Maximum rows to return. Defaults to 50, capped at 500.
    pub limit: Option<usize>,
    /// Optional RFC-3339 lower bound on `started_at`.
    pub since: Option<String>,
}

/// One row in the `/v1/retention/sweeps` response.
#[derive(Debug, Clone, Serialize)]
struct RetentionSweepBody {
    sweep_id: String,
    started_at: String,
    finished_at: Option<String>,
    status: String,
    records_demoted: u64,
    records_dropped: u64,
    /// Per-stage counters parsed from `tier_transitions_json`. Keys
    /// are stage names (`sweep`, `parquet_rollup`, `cas_vacuum`,
    /// `pii_enforce`, `turn_digest`, `meili_prune`, `metadata_reap`,
    /// …) and values carry the stage's own JSON shape so the GUI can
    /// render breakdowns without the API having to know each stage's
    /// schema. Falls back to an empty object when the source row's
    /// JSON is missing or malformed.
    stages: serde_json::Value,
}

/// One archive partition bucket — bytes by age window.
#[derive(Debug, Clone, Default, Serialize)]
struct ArchiveBuckets {
    /// Bytes in archive files modified in the last 30 days.
    le_30d: u64,
    /// Bytes in files modified between 30 and 365 days ago.
    #[serde(rename = "30d_to_365d")]
    between_30d_365d: u64,
    /// Bytes in files modified more than 365 days ago.
    gt_365d: u64,
    /// Total bytes scanned.
    total: u64,
    /// Archive root that produced these counters.
    root: String,
    /// `false` when the archive root could not be resolved or read;
    /// in that case the counters are zeros.
    available: bool,
}

/// CAS store totals.
#[derive(Debug, Clone, Default, Serialize)]
struct CasTotals {
    /// Number of `cas_blobs` rows.
    rows: u64,
    /// Sum of `size` (uncompressed bytes).
    bytes: u64,
    /// `false` when the CAS DB could not be opened (e.g. cold dev
    /// boot). Counters are zeros.
    available: bool,
    /// Path the totals were read from.
    path: String,
}

/// One scheduled run row. Phase13a §4.3 reshape (ADR-014):
///
/// - `next_run` is the schedule projection. `None` when the cron
///   row is missing from the metadata store, `Some("disabled")`
///   when the row exists with `enabled = 0`, otherwise the
///   RFC-3339 timestamp of the next fire. The wire-level `null`
///   replaces the legacy handler-side missing-state string so
///   the dashboard never claims a state it has no evidence for.
/// - `last_run` / `last_status` are sourced from
///   `retention_sweeps` (the trait-level write that ADR-009
///   guarantees), not from `cron_jobs.last_run_at` /
///   `last_status` (the child-process scheduler's bookkeeping).
///   The former is authoritative for what a sweep actually did;
///   the latter only records that the CLI exited 0.
#[derive(Debug, Clone, Serialize)]
struct ScheduledRun {
    /// Sweep type (`tier_sweep`, `parquet_rollup`, `cas_vacuum`,
    /// `pii_enforce`, `turn_digest`, `meili_prune`,
    /// `metadata_reap`, `consolidator_nightly`,
    /// `consolidation_prune`, `memory_consolidate`).
    sweep: String,
    /// RFC-3339 timestamp of the next scheduled run, `"disabled"`
    /// when the row exists but `enabled = 0`, or `null` when the
    /// cron row is missing from the metadata store. The GUI
    /// surfaces `null` as an empty cell rather than inventing a
    /// string the handler could not justify.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_run: Option<String>,
    /// RFC-3339 timestamp of the most recent run, or `null` when
    /// the sweep has never executed. Sourced from
    /// `retention_sweeps.finished_at` (most-recent row per sweep
    /// name) per ADR-009 §4.3.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run: Option<String>,
    /// Status of the most recent run (`success`, `failed`,
    /// `abandoned`), or `null` when the sweep has never executed.
    /// Sourced from `retention_sweeps.status` per ADR-009 §4.3.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    /// Consecutive failure count from the last successful run.
    /// Zero is the steady state; values > 0 surface in the GUI as a
    /// failing-streak warning pill. Still sourced from `cron_jobs`
    /// because retention_sweeps does not track streaks today; this
    /// migrates to a sweep-level counter in Phase B.
    #[serde(skip_serializing_if = "is_zero_u32")]
    failure_streak: u32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Phase13a §4.3 — extract the sweep `name` from a serialised
/// `SweepReport` payload. The supervisor writes the full report
/// JSON into `retention_sweeps.tier_transitions_json`; this helper
/// pulls the `name` field so the dashboard can key by sweep slug
/// without re-parsing the entire report.
fn parse_sweep_name(json: Option<&str>) -> Option<String> {
    let raw = json?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// `/v1/retention/state` body.
#[derive(Debug, Clone, Serialize)]
struct RetentionStateBody {
    /// Per-Vectorizer-collection size. Empty until a live SDK probe
    /// is wired through `DashboardState`; the GUI handles `[]`
    /// honestly with an "unavailable" pill.
    collections: Vec<serde_json::Value>,
    /// Parquet archive bytes by age bucket.
    archive_bytes: ArchiveBuckets,
    /// Meilisearch index document counts. Empty when no live probe
    /// is available; same honest-empty semantics as `collections`.
    meili_indexes: Vec<serde_json::Value>,
    /// CAS store totals (rows + bytes).
    cas: CasTotals,
    /// Per-sweep schedule + last-run snapshot. Phase13a §4.3
    /// reads cron-row schedule from `cron_jobs` and the last-run
    /// truth from `retention_sweeps`. Missing rows surface as
    /// wire-level `null` per ADR-014 — the dashboard refuses to
    /// invent state literals.
    next_runs: Vec<ScheduledRun>,
}

/// Map a `cron_jobs.name` (e.g. `retention.sweep`) to the public
/// sweep slug the GUI consumes (e.g. `tier_sweep`). Returns `None`
/// when the cron row is not part of the retention surface so callers
/// can ignore unrelated rows without polluting the dashboard.
fn cron_name_to_sweep_slug(name: &str) -> Option<&'static str> {
    match name {
        "retention.sweep" => Some("tier_sweep"),
        "retention.rollup" => Some("parquet_rollup"),
        "retention.cas_vacuum" => Some("cas_vacuum"),
        "retention.pii_enforce" => Some("pii_enforce"),
        "retention.turn_digest" => Some("turn_digest"),
        "retention.meili_prune" => Some("meili_prune"),
        "retention.metadata_reap" => Some("metadata_reap"),
        "retention.consolidator_nightly" => Some("consolidator_nightly"),
        "retention.consolidation_prune" => Some("consolidation_prune"),
        "retention.memory_consolidate" => Some("memory_consolidate"),
        "retention.tool_call_digest" => Some("tool_call_digest"),
        "retention.sessions_backfill" => Some("sessions_backfill"),
        _ => None,
    }
}

/// Canonical sweep slug list. Keeps the dashboard response stable
/// even when the metadata store is unavailable or the cron registry
/// is mid-seed. Every slug appears once with the right defaults.
pub(super) const RETENTION_SWEEP_SLUGS: &[&str] = &[
    "tier_sweep",
    "parquet_rollup",
    "cas_vacuum",
    "pii_enforce",
    "turn_digest",
    "tool_call_digest",
    "meili_prune",
    "metadata_reap",
    "consolidator_nightly",
    "consolidation_prune",
    "memory_consolidate",
    "sessions_backfill",
];

/// `GET /v1/retention/sweeps` — recent retention sweeps + per-stage
/// breakdown. The response is the rows from `retention_sweeps`
/// (newest first) merged with the JSON inside
/// `tier_transitions_json` so the GUI can render per-stage counters.
pub(super) async fn retention_sweeps(
    State(state): State<DashboardState>,
    Query(params): Query<RetentionSweepsQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(50).min(500);
    let since_filter = params
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_rfc3339());

    let metadata = match &state.metadata {
        Some(m) => m,
        None => {
            // Honest empty when no metadata DB is configured (cold dev
            // boot). Keeps the GUI's empty-state branch usable.
            return (StatusCode::OK, Json(Vec::<RetentionSweepBody>::new())).into_response();
        }
    };

    let rows = {
        let guard = match metadata.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match guard.list_recent_sweeps(limit) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error=%e, "retention/sweeps: list_recent_sweeps failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": "list_recent_sweeps", "detail": e.to_string()}),
                    ),
                )
                    .into_response();
            }
        }
    };

    let body: Vec<RetentionSweepBody> = rows
        .into_iter()
        .filter(|r| {
            since_filter
                .as_deref()
                .map_or(true, |s| r.started_at.as_str() >= s)
        })
        .map(|r| {
            let stages = r
                .tier_transitions_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            RetentionSweepBody {
                sweep_id: r.sweep_id,
                started_at: r.started_at,
                finished_at: r.finished_at,
                status: r.status,
                records_demoted: r.records_demoted,
                records_dropped: r.records_dropped,
                stages,
            }
        })
        .collect();

    (StatusCode::OK, Json(body)).into_response()
}

/// `GET /v1/retention/state` — compact "current state" envelope the
/// Retention tab's header cards consume.
pub(super) async fn retention_state(State(state): State<DashboardState>) -> Response {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let archive_root = cfg.ingestion.archive_root.clone().unwrap_or_else(|| {
        home_path().map_or_else(
            || ".cortex/archive".to_string(),
            |h| h.join(".cortex/archive").display().to_string(),
        )
    });
    let archive_bytes = scan_archive_age_buckets(std::path::Path::new(&archive_root));

    let cas_path = cfg.ingestion.cas_db.clone().unwrap_or_else(|| {
        home_path().map_or_else(
            || ".cortex/cas.sqlite".to_string(),
            |h| h.join(".cortex/cas.sqlite").display().to_string(),
        )
    });
    let cas = scan_cas_totals(std::path::Path::new(&cas_path));

    // Phase13a §4.3 — live-read of `cron_jobs` for schedule
    // projection, `retention_sweeps` for last-run / last-status
    // truth. Missing cron rows surface as wire-level `null`
    // (`Option::None`); disabled rows surface as
    // `Some("disabled")`. Handler-side missing-state literals are
    // gone — see ADR-014 + the CI grep gate.
    type CronRows = std::collections::HashMap<&'static str, cortex_storage::CronJob>;
    type LastPerSweep = std::collections::HashMap<String, (Option<String>, String)>;
    let (cron_rows, last_per_sweep): (CronRows, LastPerSweep) = match &state.metadata {
        Some(handle) => {
            let guard = match handle.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let cron = match guard.list_cron_jobs() {
                Ok(rows) => rows
                    .into_iter()
                    .filter_map(|j| cron_name_to_sweep_slug(&j.name).map(|s| (s, j)))
                    .collect(),
                Err(err) => {
                    tracing::warn!(error = %err, "retention/state: list_cron_jobs failed");
                    std::collections::HashMap::new()
                }
            };
            let last = match guard.list_recent_sweeps(500) {
                Ok(rows) => {
                    let mut out: std::collections::HashMap<String, (Option<String>, String)> =
                        std::collections::HashMap::new();
                    // Rows arrive newest-first; first hit per name wins.
                    for row in rows {
                        let name = parse_sweep_name(row.tier_transitions_json.as_deref())
                            .unwrap_or_else(|| row.sweep_id.clone());
                        out.entry(name)
                            .or_insert_with(|| (row.finished_at.clone(), row.status.clone()));
                    }
                    out
                }
                Err(err) => {
                    tracing::warn!(error = %err, "retention/state: list_recent_sweeps failed");
                    std::collections::HashMap::new()
                }
            };
            (cron, last)
        }
        None => (
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ),
    };

    let next_runs: Vec<ScheduledRun> = RETENTION_SWEEP_SLUGS
        .iter()
        .map(|slug| {
            let (last_run, last_status) = last_per_sweep
                .get(*slug)
                .map(|(run, status)| (run.clone(), Some(status.clone())))
                .unwrap_or_else(|| (None, None));
            match cron_rows.get(*slug) {
                Some(job) => ScheduledRun {
                    sweep: (*slug).to_string(),
                    next_run: if !job.enabled {
                        Some("disabled".to_string())
                    } else {
                        job.next_run_at.clone()
                    },
                    last_run: last_run.or_else(|| job.last_run_at.clone()),
                    last_status: last_status.or_else(|| job.last_status.clone()),
                    failure_streak: job.failure_streak,
                },
                None => ScheduledRun {
                    sweep: (*slug).to_string(),
                    next_run: None,
                    last_run,
                    last_status,
                    failure_streak: 0,
                },
            }
        })
        .collect();

    let body = RetentionStateBody {
        collections: Vec::new(),
        archive_bytes,
        meili_indexes: Vec::new(),
        cas,
        next_runs,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Scan `root` for Parquet / NDJSON archive files and bucket their
/// bytes by file-mtime age. Honest defaults when the root is
/// missing / unreadable.
fn scan_archive_age_buckets(root: &std::path::Path) -> ArchiveBuckets {
    let mut out = ArchiveBuckets {
        root: root.display().to_string(),
        ..ArchiveBuckets::default()
    };
    if !root.exists() {
        return out;
    }
    out.available = true;
    let now = std::time::SystemTime::now();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            // Only count canonical archive files. Skip lockfiles,
            // *.tmp, *.corrupted*, and the README.
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let archive_file = name.ends_with(".parquet")
                || name.ends_with(".ndjson")
                || name.ends_with(".ndjson.zst")
                || name.ends_with(".ndjson.zstd");
            if !archive_file {
                continue;
            }
            let size = metadata.len();
            let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            let age_days = now.duration_since(modified).unwrap_or_default().as_secs() / 86_400;
            out.total = out.total.saturating_add(size);
            if age_days <= 30 {
                out.le_30d = out.le_30d.saturating_add(size);
            } else if age_days <= 365 {
                out.between_30d_365d = out.between_30d_365d.saturating_add(size);
            } else {
                out.gt_365d = out.gt_365d.saturating_add(size);
            }
        }
    }
    out
}

/// Read `cas_blobs` totals from the SQLite file at `path`. Honest
/// empty when the file is missing or unreadable.
fn scan_cas_totals(path: &std::path::Path) -> CasTotals {
    let mut out = CasTotals {
        path: path.display().to_string(),
        ..CasTotals::default()
    };
    if !path.exists() {
        return out;
    }
    match cortex_storage::CasStore::open(path) {
        Ok(store) => {
            out.available = true;
            out.rows = store.total_blob_count().unwrap_or(0);
            // Sum `size` directly — `CasStore` does not expose a
            // bytes total today, so query the column.
            let bytes: i64 = store
                .conn()
                .query_row("SELECT COALESCE(SUM(size), 0) FROM cas_blobs", [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            out.bytes = bytes.max(0) as u64;
        }
        Err(e) => {
            tracing::warn!(path=%path.display(), error=%e, "cas store open failed");
        }
    }
    out
}

fn home_path() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum_extra::extract::Query;
    use serde_json::Value;
    use std::sync::Arc;

    fn make_state(metadata: Option<cortex_storage::MetadataStore>) -> DashboardState {
        let lane = crate::lanes::MemoryKeywordLane::new();
        DashboardState {
            lane: Arc::new(lane),
            nexus: None,
            analyzer: Arc::new(crate::analyzer::Analyzer::from_env()),
            tasks: Arc::new(crate::tasks_loader::MultiTaskLoader::new(vec![
                crate::tasks_loader::TaskLoader::new(std::path::PathBuf::from(
                    "__tests_no_rulebook__",
                )),
            ])),
            metadata: metadata.map(|m| Arc::new(std::sync::Mutex::new(m))),
            loader_metrics: Arc::new(crate::LoaderMetrics::new()),
            events_bus: crate::dashboard_watcher::DashboardEventBus::new(),
        }
    }

    fn seed_three_sweeps(store: &cortex_storage::MetadataStore) {
        let now = chrono::Utc::now();
        store.start_retention_sweep("01SWEEP", now, 0).unwrap();
        store
            .finish_retention_sweep(
                "01SWEEP",
                now,
                12,
                0,
                r#"{"sweep":{"turn:fp32->pq":12}}"#,
                "success",
            )
            .unwrap();
        let t2 = now + chrono::Duration::seconds(1);
        store.start_retention_sweep("01ROLLUP", t2, 0).unwrap();
        store
            .finish_retention_sweep(
                "01ROLLUP",
                t2,
                0,
                3,
                r#"{"parquet_rollup":{"merged":4,"dropped":3}}"#,
                "success",
            )
            .unwrap();
        let t3 = now + chrono::Duration::seconds(2);
        store.start_retention_sweep("01CASVAC", t3, 0).unwrap();
        store
            .finish_retention_sweep(
                "01CASVAC",
                t3,
                0,
                7,
                r#"{"cas_vacuum":{"blobs_dropped":7,"bytes_reclaimed":4096}}"#,
                "success",
            )
            .unwrap();
    }

    #[tokio::test]
    async fn retention_sweeps_returns_per_stage_counters() {
        let store = cortex_storage::MetadataStore::open_in_memory().unwrap();
        seed_three_sweeps(&store);
        let state = make_state(Some(store));
        let resp = retention_sweeps(
            State(state),
            Query(RetentionSweepsQuery {
                limit: Some(10),
                since: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let arr = v.as_array().expect("array body");
        assert_eq!(arr.len(), 3);
        for row in arr {
            assert!(row.get("stages").and_then(|s| s.as_object()).is_some());
            assert!(row["sweep_id"].is_string());
            assert!(row["status"].is_string());
        }
        assert_eq!(arr[0]["sweep_id"], "01CASVAC");
        assert!(
            arr[0]["stages"]["cas_vacuum"]["blobs_dropped"]
                .as_u64()
                .unwrap()
                >= 7
        );
    }

    #[tokio::test]
    async fn retention_sweeps_honours_since_filter() {
        let store = cortex_storage::MetadataStore::open_in_memory().unwrap();
        seed_three_sweeps(&store);
        let state = make_state(Some(store));
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let resp = retention_sweeps(
            State(state),
            Query(RetentionSweepsQuery {
                limit: Some(10),
                since: Some(future),
            }),
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn retention_sweeps_falls_back_to_empty_without_metadata() {
        let state = make_state(None);
        let resp = retention_sweeps(State(state), Query(RetentionSweepsQuery::default())).await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn scan_archive_age_buckets_classifies_files_by_mtime() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            let p = dir.path().join(format!("event-{i}.parquet"));
            std::fs::write(&p, vec![0u8; 1024 * (i as usize + 1)]).unwrap();
        }
        std::fs::write(dir.path().join("scratch.tmp"), b"ignore me").unwrap();
        std::fs::write(dir.path().join("README.md"), b"docs").unwrap();
        let buckets = scan_archive_age_buckets(dir.path());
        assert!(buckets.available);
        assert!(buckets.le_30d > 0, "expected fresh bytes in le_30d");
        assert_eq!(buckets.gt_365d, 0);
        assert_eq!(
            buckets.total,
            buckets.le_30d + buckets.between_30d_365d + buckets.gt_365d
        );
    }

    #[test]
    fn scan_archive_returns_unavailable_when_root_missing() {
        let buckets = scan_archive_age_buckets(std::path::Path::new(
            "/no/such/path/for/cortex/archive/test",
        ));
        assert!(!buckets.available);
        assert_eq!(buckets.total, 0);
    }

    #[tokio::test]
    async fn retention_state_reports_archive_bucket_for_fresh_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..15 {
            std::fs::write(
                dir.path().join(format!("hour-{i:02}.parquet")),
                vec![0u8; 256],
            )
            .unwrap();
        }
        std::env::set_var("CORTEX_ARCHIVE_ROOT", dir.path());
        std::env::set_var(
            "CORTEX_CAS_DB",
            dir.path().join("__nope__cas.sqlite").as_os_str(),
        );
        let state = make_state(None);
        let resp = retention_state(State(state)).await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        std::env::remove_var("CORTEX_ARCHIVE_ROOT");
        std::env::remove_var("CORTEX_CAS_DB");
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["archive_bytes"]["available"].as_bool().unwrap());
        assert!(v["archive_bytes"]["le_30d"].as_u64().unwrap() > 0);
        assert_eq!(v["archive_bytes"]["gt_365d"].as_u64().unwrap(), 0);
        let next = v["next_runs"].as_array().unwrap();
        assert_eq!(next.len(), RETENTION_SWEEP_SLUGS.len());
        // Phase13a §4.3: missing cron rows surface as wire-level
        // `null` (`Option::None` → skip-serialize). The legacy
        // string sentinel is gone; the GUI renders the empty cell.
        assert!(next.iter().all(|r| r.get("next_run").is_none()));
        assert!(next.iter().all(|r| r.get("last_run").is_none()));
        assert!(next.iter().any(|r| r["sweep"] == "consolidator_nightly"));
    }

    #[tokio::test]
    async fn retention_state_reads_live_cron_jobs_when_metadata_present() {
        use cortex_storage::MetadataStore;
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("metadata.sqlite");
        let store = MetadataStore::open(&db_path).expect("metadata opens");
        cortex_storage::apply_phase9k_schema(store.conn()).expect("phase9k schema");

        store.conn().execute(
            "INSERT INTO cron_jobs (name, schedule, command, enabled, last_run_at, next_run_at, last_status, failure_streak)
                  VALUES ('retention.sweep', '0 3 * * *', 'cortex-ops retention-sweep', 1,
                          '2026-05-05T03:00:11+00:00', '2026-05-06T03:00:00+00:00',
                          'success', 0)",
            [],
        ).unwrap();
        store.conn().execute(
            "INSERT INTO cron_jobs (name, schedule, command, enabled, last_run_at, next_run_at, last_status, failure_streak)
                  VALUES ('retention.consolidation_prune', '0 3 * * *', 'cortex-ops consolidation-prune', 1,
                          '2026-05-05T03:00:11+00:00', '2026-05-06T03:00:00+00:00',
                          'failed', 3)",
            [],
        ).unwrap();
        store.conn().execute(
            "INSERT INTO cron_jobs (name, schedule, command, enabled)
                  VALUES ('retention.memory_consolidate', '0 7 * * 0', 'cortex-ops memory-consolidate --apply', 0)",
            [],
        ).unwrap();

        let dir2 = tempfile::tempdir().unwrap();
        std::env::set_var("CORTEX_ARCHIVE_ROOT", dir2.path());
        std::env::set_var(
            "CORTEX_CAS_DB",
            dir2.path().join("__nope__cas.sqlite").as_os_str(),
        );
        let state = make_state(Some(store));
        let resp = retention_state(State(state)).await;
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        std::env::remove_var("CORTEX_ARCHIVE_ROOT");
        std::env::remove_var("CORTEX_CAS_DB");

        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let next = v["next_runs"].as_array().unwrap();
        let by_slug: std::collections::HashMap<&str, &Value> = next
            .iter()
            .map(|r| (r["sweep"].as_str().unwrap(), r))
            .collect();

        let sweep = by_slug["tier_sweep"];
        assert_eq!(sweep["next_run"], "2026-05-06T03:00:00+00:00");
        assert_eq!(sweep["last_run"], "2026-05-05T03:00:11+00:00");
        assert_eq!(sweep["last_status"], "success");
        assert!(sweep.get("failure_streak").is_none());

        let prune = by_slug["consolidation_prune"];
        assert_eq!(prune["last_status"], "failed");
        assert_eq!(prune["failure_streak"], 3);

        let mem = by_slug["memory_consolidate"];
        assert_eq!(mem["next_run"], "disabled");

        let nightly = by_slug["consolidator_nightly"];
        // Phase13a §4.3: missing cron row → wire-level `null`.
        assert!(nightly.get("next_run").is_none());
        assert!(nightly.get("last_run").is_none());
    }
}
